use std::sync::Once;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use tracing::info;

use p05_mob_in_the_middle::{run, BOGUSCOIN};

#[tokio::test]
async fn test_session() {
    init_logging();

    let (chat_address, chat_port) = spawn_budget_chat_app().await;
    let (address, port) = spawn_app(chat_address.clone(), chat_port).await;

    let mut stream_alice = TcpStream::connect(&format!("{chat_address}:{port}"))
        .await
        .unwrap();
    let (read_alice, mut write_alice) = stream_alice.split();
    let mut read_alice = BufReader::new(read_alice).split(b'\n');
    write_alice.write_all(b"alice\n").await.unwrap();

    let mut stream_bob = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .unwrap();
    let (read_bob, mut write_bob) = stream_bob.split();
    let mut read_bob = BufReader::new(read_bob).split(b'\n');

    let result = read_bob.next_segment().await.unwrap().unwrap();
    assert_eq!(
        std::str::from_utf8(&result).unwrap(),
        "Welcome to budgetchat! What shall I call you?"
    );

    write_bob.write_all(b"bob\n").await.unwrap();

    let result = read_bob.next_segment().await.unwrap().unwrap();
    assert_eq!(result, b"* The room contains: alice");

    write_bob
        .write_all(b"Hi alice, please send payment to 7iKDZEwPZSqIvDnHvVN2r0hUWXD5rHX\n")
        .await
        .unwrap();

    let _step_0 = read_alice.next_segment().await.unwrap(); // Welcome...
    let _step_1 = read_alice.next_segment().await.unwrap(); // * The room contains...
    let _step_2 = read_alice.next_segment().await.unwrap(); // * join

    let result = read_alice.next_segment().await.unwrap().unwrap();
    assert_eq!(
        std::str::from_utf8(&result).unwrap(),
        "[bob] Hi alice, please send payment to 7YWHMfk9JZe0LM0g1ZauHuiSxhI"
    );
}

#[tokio::test]
async fn test_not_joining() {
    init_logging();

    let (chat_address, chat_port) = spawn_budget_chat_app().await;
    let (address, port) = spawn_app(chat_address.clone(), chat_port).await;

    let mut stream_alice = TcpStream::connect(&format!("{chat_address}:{port}"))
        .await
        .unwrap();
    let (read_alice, mut write_alice) = stream_alice.split();
    let mut read_alice = BufReader::new(read_alice).split(b'\n');

    let result = read_alice.next_segment().await.unwrap().unwrap(); // Welcome...
    assert_eq!(
        std::str::from_utf8(&result).unwrap(),
        "Welcome to budgetchat! What shall I call you?"
    );

    write_alice.write_all(b"alice\n").await.unwrap();

    let result = read_alice.next_segment().await.unwrap().unwrap(); // * The room contains...
    assert_eq!(
        std::str::from_utf8(&result).unwrap(),
        "* The room contains:"
    );

    let mut stream_bob = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .unwrap();
    let (read_bob, mut write_bob) = stream_bob.split();
    let mut read_bob = BufReader::new(read_bob).split(b'\n');

    let result = read_bob.next_segment().await.unwrap().unwrap();
    assert_eq!(
        std::str::from_utf8(&result).unwrap(),
        "Welcome to budgetchat! What shall I call you?"
    );

    write_bob.write_all(b"bob").await.unwrap();
    write_bob.shutdown().await.unwrap();

    match timeout(Duration::from_millis(100), read_alice.next_segment()).await {
        Err(_) => {}
        Ok(Ok(Some(message))) => panic!("invalid: {:?}", std::str::from_utf8(&message)),
        Ok(payload) => panic!("invalid: {payload:?}"),
    }
}

fn init_logging() {
    static TRACING_SUBSCRIBER_INIT: Once = Once::new();
    TRACING_SUBSCRIBER_INIT.call_once(tracing_subscriber::fmt::init);
}

async fn spawn_budget_chat_app() -> (String, u16) {
    let address = "127.0.0.1";

    let listener = TcpListener::bind(&format!("{address}:0"))
        .await
        .expect("cannot bind budget_chat app");
    let port = listener
        .local_addr()
        .expect("cannot get local address")
        .port();

    tokio::spawn(async move {
        p03_budget_chat::run(listener).await.expect("run failed");
    });

    info!("spawned budget chat app {address}:{port}");

    (address.to_string(), port)
}

async fn spawn_app(chat_address: String, chat_port: u16) -> (String, u16) {
    let address = "127.0.0.1";

    let listener = TcpListener::bind(&format!("{address}:0"))
        .await
        .expect("cannot bind app");
    let port = listener
        .local_addr()
        .expect("cannot get local address")
        .port();

    tokio::spawn(async move {
        run(listener, chat_address, chat_port, BOGUSCOIN.to_string())
            .await
            .expect("run failed");
    });

    info!("spawned app {address}:{port}");

    (address.to_string(), port)
}
