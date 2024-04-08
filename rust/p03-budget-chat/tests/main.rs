use std::sync::Once;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use tracing::info;

#[tokio::test]
async fn test_session() {
    let (address, port) = spawn_app().await;

    let mut stream_bob = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .unwrap();
    let (_, mut write_bob) = stream_bob.split();
    write_bob.write_all(b"bob\n").await.unwrap();

    let mut stream_charlie = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .unwrap();
    let (_, mut write_charlie) = stream_charlie.split();
    write_charlie.write_all(b"charlie\n").await.unwrap();

    let mut stream_dave = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .unwrap();
    let (_, mut write_dave) = stream_dave.split();
    write_dave.write_all(b"dave\n").await.unwrap();

    let mut stream_alice = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .expect("cannot connect");
    let (read_alice, mut write_alice) = stream_alice.split();
    let mut read_alice = BufReader::new(read_alice).split(b'\n');

    tracing::debug!("waiting welcome message");
    let result = read_alice.next_segment().await.unwrap().unwrap();
    assert_eq!(result, b"Welcome to budgetchat! What shall I call you?");

    write_alice.write_all(b"alice\n").await.unwrap();

    let result = read_alice.next_segment().await.unwrap().unwrap();
    assert!(
        result.starts_with(b"*")
            && result.windows("alice".len()).all(|w| w != b"alice")
            && result.windows("bob".len()).any(|w| w == b"bob")
            && result.windows("charlie".len()).any(|w| w == b"charlie")
            && result.windows("dave".len()).any(|w| w == b"dave")
    );

    write_bob.write_all(b"hi alice\n").await.unwrap();
    let result = read_alice.next_segment().await.unwrap().unwrap();
    assert_eq!(result, b"[bob] hi alice");

    write_charlie.write_all(b"hello alice\n").await.unwrap();
    let result = read_alice.next_segment().await.unwrap().unwrap();
    assert_eq!(result, b"[charlie] hello alice");

    stream_dave.shutdown().await.unwrap();

    let result = read_alice.next_segment().await.unwrap().unwrap();
    assert_eq!(result, b"* dave has left the room");
}

#[tokio::test]
async fn test_not_joining() {
    let (address, port) = spawn_app().await;

    let mut stream_alice = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .expect("cannot connect");
    let (read_alice, mut write_alice) = stream_alice.split();
    let mut read_alice = BufReader::new(read_alice).split(b'\n');

    tracing::debug!("waiting welcome message");
    let result = read_alice.next_segment().await.unwrap().unwrap();
    assert_eq!(result, b"Welcome to budgetchat! What shall I call you?");

    write_alice.write_all(b"alice\n").await.unwrap();

    let result = read_alice.next_segment().await.unwrap().unwrap();
    assert_eq!(result, b"* The room contains:");

    let mut stream_bob = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .unwrap();
    let (_, mut write_bob) = stream_bob.split();
    write_bob.write_all(b"bob").await.unwrap(); // no newline
    write_bob.shutdown().await.unwrap();

    match timeout(Duration::from_millis(100), read_alice.next_segment()).await {
        Err(_) => { /* ok */ }
        Ok(Ok(Some(message))) => panic!("invalid: {:?}", std::str::from_utf8(&message)),
        Ok(payload) => panic!("invalid: {payload:?}"),
    }
}

async fn spawn_app() -> (String, u16) {
    static TRACING_SUBSCRIBER_INIT: Once = Once::new();
    TRACING_SUBSCRIBER_INIT.call_once(tracing_subscriber::fmt::init);

    let address = "127.0.0.1";

    let listener = TcpListener::bind(&format!("{address}:0"))
        .await
        .expect("cannot bind");
    let port = listener
        .local_addr()
        .expect("cannot get local address")
        .port();

    tokio::spawn(async move {
        p03_budget_chat::run(listener).await.expect("run failed");
    });

    info!("spawned app {address}:{port}");

    (address.to_string(), port)
}
