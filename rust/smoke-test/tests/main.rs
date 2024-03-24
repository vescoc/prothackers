use std::sync::Once;

use tracing::info;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[tokio::test]
async fn simple_echo() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .expect("cannot connect");
    let (mut read_half, mut write_half) = stream.split();

    let payload = b"ciccio cunicio";
    write_half.write_all(payload).await.unwrap();
    write_half.shutdown().await.unwrap();

    info!("done write");

    let mut buffer = [0; 1024];
    let mut end = 0;
    loop {
        let n = read_half
            .read(&mut buffer[end..])
            .await
            .expect("cannot read");
        if n == 0 {
            break;
        }

        info!("read {n} bytes");

        end += n;
    }

    assert_eq!(payload, &buffer[0..end]);
}

async fn spawn_app() -> (String, u16) {
    static TRACING_SUBSCRIBER_INIT: Once = Once::new();
    TRACING_SUBSCRIBER_INIT.call_once(tracing_subscriber::fmt::init);

    let address = "127.0.0.1";

    let listener = TcpListener::bind(&format!("{}:0", address))
        .await
        .expect("cannot bind");
    let port = listener.local_addr().expect("cannot get local addr").port();

    tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.expect("cannot accept");

            smoke_test::echo(socket).await.unwrap();
        }
    });

    info!("spawned app {address}:{port}");

    (address.to_string(), port)
}
