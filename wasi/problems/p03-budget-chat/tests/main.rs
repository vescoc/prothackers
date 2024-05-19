use std::sync::Once;
use std::time::Duration;

use futures::{sink, SinkExt, StreamExt};

use wasi_async::codec::{FramedRead, LinesDecoder};
use wasi_async::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use wasi_async::net::{TcpListener, TcpStream};
use wasi_async::time;

use tracing::{debug, info, trace};

#[test]
fn test_session() {
    wasi_async_runtime::block_on(|reactor| async move {
        let (address, port) = spawn_app(reactor.clone()).await;

        let stream_bob = TcpStream::connect(reactor.clone(), format!("{address}:{port}"))
            .await
            .unwrap();
        let (mut read_bob, mut write_bob) = stream_bob.into_split();
        write_bob.write_all(b"bob\n").await.unwrap();
        reactor.clone().spawn(async move {
            let mut dev_null = sink::drain();
            loop {
                let Ok(line) = read_bob.read(1024).await else {
                    break;
                };
                trace!("bob: {:?}", String::from_utf8_lossy(&line));
                dev_null.send(line).await.unwrap();
            }
        });

        let stream_charlie = TcpStream::connect(reactor.clone(), format!("{address}:{port}"))
            .await
            .unwrap();
        let (mut read_charlie, mut write_charlie) = stream_charlie.into_split();
        write_charlie.write_all(b"charlie\n").await.unwrap();
        reactor.clone().spawn(async move {
            let mut dev_null = sink::drain();
            loop {
                let Ok(line) = read_charlie.read(1024).await else {
                    break;
                };
                trace!("charlie: {:?}", String::from_utf8_lossy(&line));
                dev_null.send(line).await.unwrap();
            }
        });

        let stream_dave = TcpStream::connect(reactor.clone(), format!("{address}:{port}"))
            .await
            .unwrap();
        let (mut read_dave, mut write_dave) = stream_dave.into_split();
        write_dave.write_all(b"dave\n").await.unwrap();
        reactor.clone().spawn(async move {
            let mut dev_null = sink::drain();
            loop {
                let Ok(line) = read_dave.read(1024).await else {
                    break;
                };
                trace!("dave: {:?}", String::from_utf8_lossy(&line));
                dev_null.send(line).await.unwrap();
            }
        });

        let mut stream_alice = TcpStream::connect(reactor.clone(), format!("{address}:{port}"))
            .await
            .expect("cannot connect");
        let (read_alice, mut write_alice) = stream_alice.split();
        let mut read_alice = FramedRead::new(read_alice, LinesDecoder::new());

        debug!("waiting welcome message");
        let result = read_alice.next().await.unwrap().unwrap();
        assert_eq!(result, b"Welcome to budgetchat! What shall I call you?");

        write_alice.write_all(b"alice\n").await.unwrap();

        let result = read_alice.next().await.unwrap().unwrap();
        assert!(
            result.starts_with(b"*")
                && result.windows("alice".len()).all(|w| w != b"alice")
                && result.windows("bob".len()).any(|w| w == b"bob")
                && result.windows("charlie".len()).any(|w| w == b"charlie")
                && result.windows("dave".len()).any(|w| w == b"dave")
        );

        write_dave.close().await.unwrap();

        let result = read_alice.next().await.unwrap().unwrap();
        assert_eq!(result, b"* dave has left the room");

        write_bob.write_all(b"hi alice\n").await.unwrap();
        let result = read_alice.next().await.unwrap().unwrap();
        assert_eq!(result, b"[bob] hi alice");

        write_charlie.write_all(b"hello alice\n").await.unwrap();
        let result = read_alice.next().await.unwrap().unwrap();
        assert_eq!(result, b"[charlie] hello alice");
    });
}

#[test]
fn test_not_joining() {
    wasi_async_runtime::block_on(|reactor| async move {
        let (address, port) = spawn_app(reactor.clone()).await;

        let mut stream_alice = TcpStream::connect(reactor.clone(), format!("{address}:{port}"))
            .await
            .expect("cannot connect");
        let (read_alice, mut write_alice) = stream_alice.split();
        let mut read_alice = FramedRead::new(read_alice, LinesDecoder::new());

        tracing::debug!("waiting welcome message");
        let result = read_alice.next().await.unwrap().unwrap();
        assert_eq!(result, b"Welcome to budgetchat! What shall I call you?");

        write_alice.write_all(b"alice\n").await.unwrap();

        let result = read_alice.next().await.unwrap().unwrap();
        assert_eq!(result, b"* The room contains:");

        let mut stream_bob = TcpStream::connect(reactor.clone(), format!("{address}:{port}"))
            .await
            .unwrap();
        let (_, mut write_bob) = stream_bob.split();
        write_bob.write_all(b"bob").await.unwrap(); // no newline
        stream_bob.close().await.unwrap();

        match time::timeout(
            reactor.clone(),
            Duration::from_millis(100),
            read_alice.next(),
        )
        .await
        {
            Err(time::Elapsed) => info!("elapsed"),
            Ok(Some(Ok(message))) => panic!("invalid: {:?}", std::str::from_utf8(&message)),
            Ok(payload) => panic!("invalid: {payload:?}"),
        }
    });
}

async fn spawn_app(reactor: wasi_async_runtime::Reactor) -> (String, u16) {
    static INIT_TRACING_SUBSCRIBER: Once = Once::new();
    INIT_TRACING_SUBSCRIBER.call_once(tracing_subscriber::fmt::init);

    let address = "127.0.0.1";

    let listener = TcpListener::bind(reactor.clone(), format!("{address}:0"))
        .await
        .expect("cannot bind");
    let port = listener
        .local_addr()
        .expect("cannot get local address")
        .port();

    reactor.clone().spawn(async move {
        p03_budget_chat::run(reactor.clone(), listener)
            .await
            .expect("run failed");
    });

    info!("spawned app {address}:{port}");

    (address.to_string(), port)
}
