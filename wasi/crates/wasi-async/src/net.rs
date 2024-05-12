use std::mem::ManuallyDrop;

use wasi::sockets::instance_network::instance_network;
use wasi::sockets::ip_name_lookup::resolve_addresses;
use wasi::sockets::network::{
    ErrorCode, IpAddress, IpAddressFamily, IpSocketAddress, Ipv4SocketAddress, Ipv6SocketAddress,
};
use wasi::sockets::tcp::{ShutdownType, TcpSocket};
use wasi::sockets::tcp_create_socket::create_tcp_socket;

use wasi::io::streams::{InputStream, OutputStream, StreamError};

use wasi_async_runtime::Reactor;

pub struct SocketAddress<'a>(&'a str, u16);

pub trait ToSocketAddress {
    fn to_socket_address(&self) -> Result<SocketAddress, ErrorCode>;
}

impl ToSocketAddress for String {
    fn to_socket_address(&self) -> Result<SocketAddress, ErrorCode> {
        let (address, port) = self.split_once(':').ok_or(ErrorCode::InvalidArgument)?;

        let port = port.parse().map_err(|_| ErrorCode::InvalidArgument)?;

        Ok(SocketAddress(address, port))
    }
}

pub struct TcpListener {
    reactor: Reactor,
    socket: TcpSocket,
}

impl TcpListener {
    pub async fn bind(reactor: Reactor, address: impl ToSocketAddress) -> Result<Self, ErrorCode> {
        let SocketAddress(address, port) = address.to_socket_address()?;

        let network = instance_network();

        let addresses = resolve_addresses(&network, address)?;

        reactor.wait_for(addresses.subscribe()).await;

        let address = addresses
            .resolve_next_address()?
            .ok_or(ErrorCode::InvalidArgument)?;

        let (family, socket_address) = match address {
            IpAddress::Ipv4(address) => (
                IpAddressFamily::Ipv4,
                IpSocketAddress::Ipv4(Ipv4SocketAddress { address, port }),
            ),

            IpAddress::Ipv6(address) => panic!("unsupported ipv6 address: {address:?}"),
        };

        let socket = create_tcp_socket(family)?;

        socket.start_bind(&network, socket_address)?;

        reactor.wait_for(socket.subscribe()).await;

        socket.finish_bind()?;

        socket.start_listen()?;

        reactor.wait_for(socket.subscribe()).await;

        socket.finish_listen()?;

        Ok(Self { reactor, socket })
    }

    pub async fn accept(&self) -> Result<(TcpStream, IpSocketAddress), ErrorCode> {
        self.reactor.wait_for(self.socket.subscribe()).await;

        let (socket, input_stream, output_stream) = self.socket.accept()?;

        let address = socket.remote_address()?;

        Ok((
            TcpStream {
                reactor: self.reactor.clone(),
                socket: ManuallyDrop::new(socket),
                input_stream,
                output_stream,
            },
            address,
        ))
    }

    pub fn local_addr(&self) -> Result<LocalSocketAddress, ErrorCode> {
        match self.socket.local_address()? {
            IpSocketAddress::Ipv4(Ipv4SocketAddress { port, .. })
            | IpSocketAddress::Ipv6(Ipv6SocketAddress { port, .. }) => Ok(LocalSocketAddress(port)),
        }
    }
}

pub struct LocalSocketAddress(u16);

impl LocalSocketAddress {
    pub fn port(&self) -> u16 {
        self.0
    }
}

pub struct TcpStream {
    reactor: Reactor,
    // There is a panic in drop, I must manually drop the socket
    socket: ManuallyDrop<TcpSocket>,
    input_stream: InputStream,
    output_stream: OutputStream,
}

impl TcpStream {
    pub async fn connect(
        reactor: Reactor,
        remote_address: impl ToSocketAddress,
    ) -> Result<Self, ErrorCode> {
        let SocketAddress(address, port) = remote_address.to_socket_address()?;

        let network = instance_network();

        let addresses = resolve_addresses(&network, address)?;

        reactor.wait_for(addresses.subscribe()).await;

        let address = addresses
            .resolve_next_address()?
            .ok_or(ErrorCode::InvalidArgument)?;

        let (family, socket_address) = match address {
            IpAddress::Ipv4(address) => (
                IpAddressFamily::Ipv4,
                IpSocketAddress::Ipv4(Ipv4SocketAddress { address, port }),
            ),

            IpAddress::Ipv6(address) => panic!("unsupported ipv6 address: {address:?}"),
        };

        let socket = create_tcp_socket(family)?;

        socket.start_connect(&network, socket_address)?;

        reactor.wait_for(socket.subscribe()).await;

        let (input_stream, output_stream) = socket.finish_connect()?;

        Ok(Self {
            reactor,
            socket: ManuallyDrop::new(socket),
            input_stream,
            output_stream,
        })
    }

    pub fn split(&mut self) -> (ReadHalf, WriteHalf) {
        let read = ReadHalf {
            reactor: self.reactor.clone(),
            input_stream: &mut self.input_stream,
        };
        let write = WriteHalf {
            reactor: self.reactor.clone(),
            output_stream: &mut self.output_stream,
        };
        (read, write)
    }

    pub async fn close(&mut self) -> Result<(), ErrorCode> {
        self.socket.shutdown(ShutdownType::Both)
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        self.socket.shutdown(ShutdownType::Both).ok();
    }
}

pub struct ReadHalf<'a> {
    reactor: Reactor,
    input_stream: &'a mut InputStream,
}

impl<'a> ReadHalf<'a> {
    pub async fn read(&mut self, len: u64) -> Result<Vec<u8>, StreamError> {
        self.reactor.wait_for(self.input_stream.subscribe()).await;

        let data = self.input_stream.read(len)?;
        if data.is_empty() {
            self.reactor.wait_for(self.input_stream.subscribe()).await;
            self.input_stream.read(len)
        } else {
            Ok(data)
        }
    }
}

pub struct WriteHalf<'a> {
    reactor: Reactor,
    output_stream: &'a mut OutputStream,
}

impl<'a> WriteHalf<'a> {
    pub async fn write(&mut self, data: &[u8]) -> Result<u64, StreamError> {
        self.reactor.wait_for(self.output_stream.subscribe()).await;

        let len = self.output_stream.check_write()?;
        assert!(len > 0, "write len zero");

        let len = data.len().min(len as usize);

        self.output_stream.write(&data[0..len])?;

        Ok(len as u64)
    }

    pub async fn write_all(&mut self, mut data: &[u8]) -> Result<(), StreamError> {
        while !data.is_empty() {
            let len = self.write(data).await?;
            assert!(len > 0, "write_all len zero");

            if len as usize == data.len() {
                break;
            }

            data = &data[len as usize..];
        }

        Ok(())
    }

    pub async fn flush(&mut self) -> Result<(), StreamError> {
        self.output_stream.flush()
    }
}
