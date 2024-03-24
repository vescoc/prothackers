use std::io;
use std::sync::Once;

use tracing::info;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};

struct Request(u8, i32, i32);

impl Request {
    async fn write<W: AsyncWriteExt + std::marker::Unpin>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(self.0).await?;
        writer.write_i32(self.1).await?;
        writer.write_i32(self.2).await?;
        writer.flush().await
    }
}

#[tokio::test]
async fn test_session() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .expect("cannot connect");
    let (read_half, write_half) = stream.split();
    let mut read_half = BufReader::new(read_half);
    let mut write_half = BufWriter::new(write_half);

    Request(b'I', 12345, 101)
        .write(&mut write_half)
        .await
        .unwrap();
    Request(b'Q', 12345, 16000)
        .write(&mut write_half)
        .await
        .unwrap();

    let result = read_half.read_i32().await.unwrap();
    assert_eq!(result, 101);
}

async fn spawn_app() -> (String, u16) {
    static TRACING_SUBSCRIBER_INIT: Once = Once::new();
    TRACING_SUBSCRIBER_INIT.call_once(tracing_subscriber::fmt::init);

    let address = "127.0.0.1";

    let listener = TcpListener::bind(&format!("{}:0", address))
        .await
        .expect("cannot bind");
    let port = listener
        .local_addr()
        .expect("cannot get local address")
        .port();

    tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.expect("cannot accept");

            means_to_an_end::handler(socket).await.unwrap();
        }
    });

    info!("spawned app {address}:{port}");

    (address.to_string(), port)
}
