use std::sync::Once;

use tracing::info;

use wasi_async::net::UdpSocket;

#[test]
fn test_session() {
    wasi_async_runtime::block_on(|reactor| async move {
        let (address, port) = spawn_app(reactor.clone()).await;

        let socket = UdpSocket::bind(reactor.clone(), "127.0.0.1:0".to_string())
            .await
            .unwrap();

        socket.connect(format!("{address}:{port}")).await.unwrap();

        socket.send(b"foo=bar".to_vec()).await.unwrap();

        socket.send(b"foo=bar".to_vec()).await.unwrap();

        socket.send(b"foo".to_vec()).await.unwrap();
        let data = socket.recv().await.unwrap();
        assert_eq!("foo=bar", std::str::from_utf8(&data).unwrap());

        socket.send(b"foo=bar=baz".to_vec()).await.unwrap();

        socket.send(b"foo".to_vec()).await.unwrap();
        let data = socket.recv().await.unwrap();
        assert_eq!("foo=bar=baz", std::str::from_utf8(&data).unwrap());

        socket.send(b"foo=".to_vec()).await.unwrap();

        socket.send(b"foo".to_vec()).await.unwrap();
        let data = socket.recv().await.unwrap();
        assert_eq!("foo=", std::str::from_utf8(&data).unwrap());

        socket.send(b"foo===".to_vec()).await.unwrap();

        socket.send(b"foo".to_vec()).await.unwrap();
        let data = socket.recv().await.unwrap();
        assert_eq!("foo===", std::str::from_utf8(&data).unwrap());

        socket.send(b"=foo".to_vec()).await.unwrap();

        socket.send(b"".to_vec()).await.unwrap();
        let data = socket.recv().await.unwrap();
        assert_eq!("=foo", std::str::from_utf8(&data).unwrap());

        socket.send(b"version=ignored".to_vec()).await.unwrap();

        socket.send(b"version".to_vec()).await.unwrap();
        let data = socket.recv().await.unwrap();
        assert_eq!(
            "version=unusual-database-program 1.0.0",
            std::str::from_utf8(&data).unwrap()
        );
    });
}

async fn spawn_app(reactor: wasi_async_runtime::Reactor) -> (String, u16) {
    static INIT_TRACING_SUBSCRIBER: Once = Once::new();
    INIT_TRACING_SUBSCRIBER.call_once(tracing_subscriber::fmt::init);

    let address = "127.0.0.1";

    let socket = UdpSocket::bind(reactor.clone(), format!("{address}:0"))
        .await
        .unwrap();
    let port = socket
        .local_addr()
        .expect("cannot get local address")
        .port();

    reactor.spawn(async move {
        p04_unusual_database_program::run(socket).await.unwrap();
    });

    info!("spawned app {address}:{port}");

    (address.to_string(), port)
}
