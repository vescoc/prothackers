use std::sync::Once;

use tokio::net::UdpSocket;

use tracing::info;

#[tokio::test]
async fn test_session() {
    let (address, port) = spawn_app().await;

    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.connect(&format!("{address}:{port}")).await.unwrap();

    let mut buffer = [0; 1024];

    socket.send(b"foo=bar").await.unwrap();

    socket.send(b"foo").await.unwrap();
    let received = socket.recv(&mut buffer).await.unwrap();
    assert_eq!("bar", std::str::from_utf8(&buffer[0..received]).unwrap());

    socket.send(b"foo=bar=baz").await.unwrap();

    socket.send(b"foo").await.unwrap();
    let received = socket.recv(&mut buffer).await.unwrap();
    assert_eq!(
        "bar=baz",
        std::str::from_utf8(&buffer[0..received]).unwrap()
    );

    socket.send(b"foo=").await.unwrap();

    socket.send(b"foo").await.unwrap();
    let received = socket.recv(&mut buffer).await.unwrap();
    assert_eq!("", std::str::from_utf8(&buffer[0..received]).unwrap());

    socket.send(b"foo===").await.unwrap();

    socket.send(b"foo").await.unwrap();
    let received = socket.recv(&mut buffer).await.unwrap();
    assert_eq!("==", std::str::from_utf8(&buffer[0..received]).unwrap());

    socket.send(b"=foo").await.unwrap();

    socket.send(b"").await.unwrap();
    let received = socket.recv(&mut buffer).await.unwrap();
    assert_eq!("foo", std::str::from_utf8(&buffer[0..received]).unwrap());

    socket.send(b"version=ignored").await.unwrap();

    socket.send(b"version").await.unwrap();
    let received = socket.recv(&mut buffer).await.unwrap();
    assert_eq!(
        "unusual-database-program 1.0.0",
        std::str::from_utf8(&buffer[0..received]).unwrap()
    );
}

async fn spawn_app() -> (String, u16) {
    static TRACING_SUBSCRIBER_INIT: Once = Once::new();
    TRACING_SUBSCRIBER_INIT.call_once(tracing_subscriber::fmt::init);

    let address = "127.0.0.1";

    let socket = UdpSocket::bind(&format!("{address}:0")).await.unwrap();
    let port = socket
        .local_addr()
        .expect("cannot get local address")
        .port();

    tokio::spawn(async move {
        p04_unusual_database_program::run(socket).await.unwrap();
    });

    info!("spawned app {address}:{port}");

    (address.to_string(), port)
}
