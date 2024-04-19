use std::time::Duration;

use futures::{SinkExt, StreamExt, TryStreamExt};

use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use tokio_util::codec::{BytesCodec, Framed};

use bytes::Bytes;

use parking_lot::Once;

use tracing::info;

use p08_insecure_sockets_layer::run;

const TIMEOUT: Duration = Duration::from_millis(100);

#[tokio::test]
async fn test_session() {
    let (address, port) = spawn_app().await;

    let stream = TcpStream::connect(format!("{address}:{port}"))
        .await
        .unwrap();
    let stream = Framed::new(stream, BytesCodec::new());
    let (mut sink, mut stream) = stream.split();

    sink.send(Bytes::from([0x02, 0x7b, 0x05, 0x01, 0x00].as_slice()))
        .await
        .unwrap();
    sink.flush().await.unwrap();

    sink.send(Bytes::from(
        [
            0xf2, 0x20, 0xba, 0x44, 0x18, 0x84, 0xba, 0xaa, 0xd0, 0x26, 0x44, 0xa4, 0xa8, 0x7e,
        ]
        .as_slice(),
    ))
    .await
    .unwrap();
    timeout(TIMEOUT, sink.flush()).await.unwrap().unwrap();

    let value = timeout(TIMEOUT, stream.try_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        Bytes::from([0x72, 0x20, 0xba, 0xd8, 0x78, 0x70, 0xee].as_slice()),
        value
    );

    sink.send(Bytes::from(
        [
            0x6a, 0x48, 0xd6, 0x58, 0x34, 0x44, 0xd6, 0x7a, 0x98, 0x4e, 0x0c, 0xcc, 0x94, 0x31,
        ]
        .as_slice(),
    ))
    .await
    .unwrap();
    timeout(TIMEOUT, sink.flush()).await.unwrap().unwrap();

    let value = timeout(TIMEOUT, stream.try_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        Bytes::from([0xf2, 0xd0, 0x26, 0xc8, 0xa4, 0xd8, 0x7e].as_slice()),
        value
    );
}

fn init_tracing_subscriber() {
    static TRACING_SUBSCRIBER_INIT: Once = Once::new();
    TRACING_SUBSCRIBER_INIT.call_once(tracing_subscriber::fmt::init);
}

async fn spawn_app() -> (String, u16) {
    init_tracing_subscriber();

    let address = "127.0.0.1";

    let listener = TcpListener::bind(&format!("{address}:0"))
        .await
        .expect("cannot bind app");
    let port = listener
        .local_addr()
        .expect("cannot get local address")
        .port();

    tokio::spawn(async move {
        run(listener).await.expect("run failed");
    });

    info!("spawned app {address}:{port}");

    (address.to_string(), port)
}
