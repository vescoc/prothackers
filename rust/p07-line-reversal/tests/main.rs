use std::time::Duration;

use tokio::io::{split, AsyncReadExt, AsyncWriteExt};
use tokio::net::ToSocketAddrs;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::timeout;

use tracing::{info, warn};

use p07_line_reversal::{
    init_tracing_subscriber,
    lrcp::packets::SyncWrite,
    lrcp::protocol::{Endpoint, Packet, Socket},
    run, DefaultSocketHandler,
};

const TIMEOUT: Duration = Duration::from_millis(200);

struct UdpEndpoint<A>(UdpSocket, A);

impl<A: ToSocketAddrs + Send + Sync + 'static>
    Endpoint<Packet, mpsc::UnboundedReceiver<Packet>, mpsc::UnboundedSender<Packet>>
    for UdpEndpoint<A>
{
    fn split(
        self,
    ) -> (
        mpsc::UnboundedReceiver<Packet>,
        mpsc::UnboundedSender<Packet>,
    ) {
        let (upstream_sender, upstream_receiver) = mpsc::unbounded_channel();
        let (downstream_sender, mut downstream_receiver) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut buffer = [0; 1024];
            loop {
                tokio::select! {
                    Ok((len, _)) = self.0.recv_from(&mut buffer) => {
                        let buffer = &buffer[0..len];

                        if let Ok(packet) = Packet::try_from(buffer) {
                            info!("client --> send upstream packet {packet:?}");
                            if let Err(e) = upstream_sender.send(packet) {
                                warn!("error on send upstream: {e:?}");
                            }
                        } else {
                            warn!("invalid packet: {buffer:?}");
                        }
                    }

                    Some(packet) = downstream_receiver.recv() => {
                        info!("client <-- send downstream packet {packet:?}");

                        let mut buffer = [0_u8; 1024];
                        let mut b = buffer.as_mut_slice();

                        let len = b.write_value(&packet).unwrap();

                        if let Err(e) = self.0.send_to(&buffer[..len], &self.1).await {
                            warn!("cannot send packet: {e:?}");
                        }
                    }
                }
            }
        });

        (upstream_receiver, downstream_sender)
    }
}

#[tokio::test]
async fn test_session() {
    let (address, port) = spawn_app().await;

    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    let endpoint = UdpEndpoint(socket, format!("{address}:{port}"));

    let stream = Socket::<DefaultSocketHandler>::connect(endpoint)
        .await
        .unwrap();
    let (mut read, mut write) = split(stream);

    let mut buffer = [0; 1024];

    write.write_all(b"hello\n").await.unwrap();
    write.flush().await.unwrap();

    let len = timeout(TIMEOUT, read.read(&mut buffer))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(b"olleh\n", &buffer[..len]);

    write.write_all(b"Hello, world!\n").await.unwrap();
    write.flush().await.unwrap();

    let len = timeout(TIMEOUT, read.read(&mut buffer))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(b"!dlrow ,olleH\n", &buffer[..len]);
}

#[tokio::test]
async fn test_backslash() {
    let (address, port) = spawn_app().await;

    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    let endpoint = UdpEndpoint(socket, format!("{address}:{port}"));

    let stream = Socket::<DefaultSocketHandler>::connect(endpoint)
        .await
        .unwrap();
    let (mut read, mut write) = split(stream);

    let mut buffer = [0; 1024];

    write.write_all(b"foo/bar/baz\n").await.unwrap();
    timeout(TIMEOUT, write.flush()).await.unwrap().unwrap();

    let len = timeout(TIMEOUT, read.read(&mut buffer))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(b"zab/rab/oof\n", &buffer[..len]);

    write.write_all(b"foo\\bar\\baz\n").await.unwrap();
    timeout(TIMEOUT, write.flush()).await.unwrap().unwrap();

    let len = timeout(TIMEOUT, read.read(&mut buffer))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(b"zab\\rab\\oof\n", &buffer[..len]);
}

async fn spawn_app() -> (String, u16) {
    init_tracing_subscriber();

    let address = "127.0.0.1";

    let socket = UdpSocket::bind(&format!("{address}:0")).await.unwrap();
    let port = socket
        .local_addr()
        .expect("cannot get local address")
        .port();

    tokio::spawn(async move {
        run::<DefaultSocketHandler>(socket).await.unwrap();
    });

    info!("spawned app {address}:{port}");

    (address.to_string(), port)
}
