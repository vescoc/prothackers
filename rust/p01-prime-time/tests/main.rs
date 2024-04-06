use std::sync::Once;

use tracing::info;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

#[tokio::test]
async fn test_invalid_number_float() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .expect("cannot connect");
    let (read_half, mut write_half) = stream.split();
    let mut read_half = BufReader::new(read_half);

    let payload = br#"{"method":"isPrime","number":3.0}"#;
    write_half.write_all(payload).await.unwrap();
    write_half.write_u8(b'\n').await.unwrap();
    write_half.shutdown().await.unwrap();

    let mut buffer = vec![];
    let n = read_half.read_until(b'\n', &mut buffer).await.unwrap();
    assert!(n > 0);

    let response: p01_prime_time::Response = serde_json::from_slice(&buffer[0..n - 1]).unwrap();
    assert!(response.prime == false);
}

#[tokio::test]
async fn test_invalid_number() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .expect("cannot connect");
    let (read_half, mut write_half) = stream.split();
    let mut read_half = BufReader::new(read_half);

    let payload = br#"{"method":"isPrime","number":"not a number"}"#;
    write_half.write_all(payload).await.unwrap();
    write_half.write_u8(b'\n').await.unwrap();
    write_half.shutdown().await.unwrap();

    let mut buffer = vec![];
    let n = read_half.read_until(b'\n', &mut buffer).await.unwrap();
    assert!(n > 0);

    assert_eq!(&buffer, b"MALFORMED");
}

#[tokio::test]
async fn test_ignore_field() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .expect("cannot connect");
    let (read_half, mut write_half) = stream.split();
    let mut read_half = BufReader::new(read_half);

    let payload = br#"{"method":"isPrime","number":3,"ignored":false}"#;
    write_half.write_all(payload).await.unwrap();
    write_half.write_u8(b'\n').await.unwrap();
    write_half.shutdown().await.unwrap();

    let mut buffer = vec![];
    let n = read_half.read_until(b'\n', &mut buffer).await.unwrap();
    assert!(n > 0);

    let response: p01_prime_time::Response = serde_json::from_slice(&buffer[0..n - 1]).unwrap();
    assert!(response.prime == true);
}

#[tokio::test]
async fn test_invalid_message() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .expect("cannot connect");
    let (read_half, mut write_half) = stream.split();
    let mut read_half = BufReader::new(read_half);

    let payload = br#"{"method":"bho","number":3}"#;
    write_half.write_all(payload).await.unwrap();
    write_half.write_u8(b'\n').await.unwrap();
    write_half.shutdown().await.unwrap();

    let mut buffer = vec![];
    let n = read_half.read_until(b'\n', &mut buffer).await.unwrap();
    assert!(n > 0);

    assert_eq!(&buffer, b"MALFORMED");
}

#[tokio::test]
async fn test_invalid_json() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .expect("cannot connect");
    let (read_half, mut write_half) = stream.split();
    let mut read_half = BufReader::new(read_half);

    let payload = br#"{"method":"bho","number":3"#;
    write_half.write_all(payload).await.unwrap();
    write_half.write_u8(b'\n').await.unwrap();
    write_half.shutdown().await.unwrap();

    let mut buffer = vec![];
    let n = read_half.read_until(b'\n', &mut buffer).await.unwrap();
    assert!(n > 0);

    assert_eq!(&buffer, b"MALFORMED");
}

#[tokio::test]
async fn test_is_prime() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .expect("cannot connect");
    let (read_half, mut write_half) = stream.split();
    let mut read_half = BufReader::new(read_half);

    let payload = br#"{"method":"isPrime","number":3}"#;
    write_half.write_all(payload).await.unwrap();
    write_half.write_u8(b'\n').await.unwrap();
    write_half.shutdown().await.unwrap();

    let mut buffer = vec![];
    let n = read_half.read_until(b'\n', &mut buffer).await.unwrap();
    assert!(n > 0);

    let response: p01_prime_time::Response = serde_json::from_slice(&buffer[0..n - 1]).unwrap();
    assert!(response.prime == true);
}

#[tokio::test]
async fn test_not_is_prime() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .expect("cannot connect");
    let (read_half, mut write_half) = stream.split();
    let mut read_half = BufReader::new(read_half);

    let payload = br#"{"method":"isPrime","number":10}"#;
    write_half.write_all(payload).await.unwrap();
    write_half.write_u8(b'\n').await.unwrap();
    write_half.shutdown().await.unwrap();

    let mut buffer = vec![];
    let n = read_half.read_until(b'\n', &mut buffer).await.unwrap();
    assert!(n > 0);

    let response: p01_prime_time::Response = serde_json::from_slice(&buffer[0..n - 1]).unwrap();
    assert!(response.prime == false);
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

            p01_prime_time::handler(socket).await.unwrap();
        }
    });

    info!("spawned app {address}:{port}");

    (address.to_string(), port)
}
