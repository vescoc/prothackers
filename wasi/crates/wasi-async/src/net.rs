use std::future::Future;
use std::mem::ManuallyDrop;
use std::pin::pin;
use std::rc::Rc;
use std::task::Poll;

use futures::{stream, Stream};

use wasi::sockets::instance_network::instance_network;
use wasi::sockets::ip_name_lookup::resolve_addresses;
use wasi::sockets::network::{
    ErrorCode, IpAddress, IpAddressFamily, IpSocketAddress, Ipv4SocketAddress, Ipv6SocketAddress,
};
use wasi::sockets::tcp::{ShutdownType, TcpSocket};
use wasi::sockets::tcp_create_socket::create_tcp_socket;

use wasi::io::streams::{InputStream, OutputStream, StreamError};

use wasi_async_runtime::Reactor;

use crate::io::{AsyncRead, AsyncWrite};

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

    pub fn into_stream(
        self,
    ) -> impl Stream<Item = Result<(TcpStream, IpSocketAddress), ErrorCode>> {
        stream::poll_fn(move |cx| {
            let reactor = self.reactor.clone();
            let subscription = self.socket.subscribe();
            let wait_for = pin!(reactor.wait_for(subscription));
            if wait_for.poll(cx).is_pending() {
                return Poll::Pending;
            }

            match self.socket.accept() {
                Ok((socket, input_stream, output_stream)) => {
                    let address = match socket.remote_address() {
                        Ok(address) => address,
                        Err(err) => return Poll::Ready(Some(Err(err))),
                    };

                    Poll::Ready(Some(Ok((
                        TcpStream(Some(TcpStreamInner {
                            reactor: self.reactor.clone(),
                            socket: ManuallyDrop::new(socket),
                            input_stream: ManuallyDrop::new(input_stream),
                            output_stream: ManuallyDrop::new(output_stream),
                        })),
                        address,
                    ))))
                }
                Err(err) => Poll::Ready(Some(Err(err))),
            }
        })
    }

    pub async fn accept(&self) -> Result<(TcpStream, IpSocketAddress), ErrorCode> {
        self.reactor.wait_for(self.socket.subscribe()).await;

        let (socket, input_stream, output_stream) = self.socket.accept()?;

        let address = socket.remote_address()?;

        Ok((
            TcpStream(Some(TcpStreamInner {
                reactor: self.reactor.clone(),
                socket: ManuallyDrop::new(socket),
                input_stream: ManuallyDrop::new(input_stream),
                output_stream: ManuallyDrop::new(output_stream),
            })),
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

struct TcpStreamInner {
    reactor: Reactor,
    // There is a panic in drop, I must manually drop the socket
    socket: ManuallyDrop<TcpSocket>,
    input_stream: ManuallyDrop<InputStream>,
    output_stream: ManuallyDrop<OutputStream>,
}

pub struct TcpStream(Option<TcpStreamInner>);

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

        Ok(Self(Some(TcpStreamInner {
            reactor,
            socket: ManuallyDrop::new(socket),
            input_stream: ManuallyDrop::new(input_stream),
            output_stream: ManuallyDrop::new(output_stream),
        })))
    }

    pub fn into_split(mut self) -> (OwnedReadHalf, OwnedWriteHalf) {
        let TcpStreamInner {
            socket,
            reactor,
            input_stream,
            output_stream,
        } = self.0.take().unwrap();

        let socket = Rc::new(socket);
        let read = OwnedReadHalf {
            socket: socket.clone(),
            reactor: reactor.clone(),
            input_stream,
        };
        let write = OwnedWriteHalf {
            socket,
            reactor,
            output_stream,
        };
        (read, write)
    }

    pub fn split(&mut self) -> (ReadHalf, WriteHalf) {
        let this = self.0.as_mut().unwrap();

        let read = ReadHalf {
            reactor: this.reactor.clone(),
            input_stream: &mut this.input_stream,
        };
        let write = WriteHalf {
            reactor: this.reactor.clone(),
            output_stream: &mut this.output_stream,
        };
        (read, write)
    }

    pub async fn close(self) -> Result<(), ErrorCode> {
        if let Some(TcpStreamInner { socket, .. }) = &self.0 {
            socket.shutdown(ShutdownType::Both)
        } else {
            Ok(())
        }
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        if let Some(TcpStreamInner { socket, .. }) = &self.0 {
            socket.shutdown(ShutdownType::Both).ok();
        }
    }
}

pub struct ReadHalf<'a> {
    reactor: Reactor,
    input_stream: &'a mut InputStream,
}

impl<'a> AsyncRead for ReadHalf<'a> {
    async fn read(&mut self, len: u64) -> Result<Vec<u8>, StreamError> {
        loop {
            let data = self.input_stream.read(len)?;
            if !data.is_empty() {
                return Ok(data);
            }
            self.reactor.wait_for(self.input_stream.subscribe()).await;
        }
    }
}

pub struct WriteHalf<'a> {
    reactor: Reactor,
    output_stream: &'a mut OutputStream,
}

impl<'a> AsyncWrite for WriteHalf<'a> {
    async fn write(&mut self, data: &[u8]) -> Result<u64, StreamError> {
        let len = loop {
            let len = self.output_stream.check_write()?;
            if len > 0 {
                break len;
            }
            self.reactor.wait_for(self.output_stream.subscribe()).await;
        };

        let len = data.len().min(len as usize);

        self.output_stream.write(&data[0..len])?;

        Ok(len as u64)
    }

    async fn flush(&mut self) -> Result<(), StreamError> {
        self.output_stream.flush()
    }
}

impl<'a> WriteHalf<'a> {
    pub async fn splice(&mut self, read: &mut ReadHalf<'_>, len: u64) -> Result<u64, StreamError> {
        if len == 0 {
            return Ok(0);
        }

        while self.output_stream.check_write()?.min(len) == 0 {
            self.reactor.wait_for(self.output_stream.subscribe()).await;
        }

        loop {
            let len = self.output_stream.splice(read.input_stream, len)?;
            if len > 0 {
                return Ok(len);
            }
            self.reactor.wait_for(read.input_stream.subscribe()).await;
        }
    }
}

pub struct OwnedReadHalf {
    socket: Rc<ManuallyDrop<TcpSocket>>,
    reactor: Reactor,
    input_stream: ManuallyDrop<InputStream>,
}

impl Drop for OwnedReadHalf {
    fn drop(&mut self) {
        self.socket.shutdown(ShutdownType::Receive).ok();
    }
}

impl AsyncRead for OwnedReadHalf {
    async fn read(&mut self, len: u64) -> Result<Vec<u8>, StreamError> {
        loop {
            let data = self.input_stream.read(len)?;
            if !data.is_empty() {
                return Ok(data);
            }
            self.reactor.wait_for(self.input_stream.subscribe()).await;
        }
    }
}

pub struct OwnedWriteHalf {
    socket: Rc<ManuallyDrop<TcpSocket>>,
    reactor: Reactor,
    output_stream: ManuallyDrop<OutputStream>,
}

impl OwnedWriteHalf {
    pub async fn close(self) -> Result<(), ErrorCode> {
        self.socket.shutdown(ShutdownType::Send)
    }
}

impl Drop for OwnedWriteHalf {
    fn drop(&mut self) {
        self.socket.shutdown(ShutdownType::Send).ok();
    }
}

impl AsyncWrite for OwnedWriteHalf {
    async fn write(&mut self, data: &[u8]) -> Result<u64, StreamError> {
        let len = loop {
            let len = self.output_stream.check_write()?;
            if len > 0 {
                break len;
            }
            self.reactor.wait_for(self.output_stream.subscribe()).await;
        };

        let len = data.len().min(len as usize);

        self.output_stream.write(&data[0..len])?;

        Ok(len as u64)
    }

    async fn flush(&mut self) -> Result<(), StreamError> {
        self.output_stream.flush()
    }
}

impl OwnedWriteHalf {
    pub async fn splice(&mut self, read: &mut OwnedReadHalf, len: u64) -> Result<u64, StreamError> {
        if len == 0 {
            return Ok(0);
        }

        while self.output_stream.check_write()?.min(len) == 0 {
            self.reactor.wait_for(self.output_stream.subscribe()).await;
        }

        loop {
            let len = self.output_stream.splice(&read.input_stream, len)?;
            if len > 0 {
                return Ok(len);
            }
            self.reactor.wait_for(read.input_stream.subscribe()).await;
        }
    }
}
