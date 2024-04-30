use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use tracing::info;

use p10_voracious_code_storage::{run, WriteLine};

const TIMEOUT: Duration = Duration::from_millis(100);

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn test_session() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(format!("{address}:{port}"))
        .await
        .unwrap();
    let (read, write) = stream.split();
    let mut read = BufReader::new(read);
    let mut write = BufWriter::new(write);

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");

    timeout(TIMEOUT, write.write_line(b"HELP"))
        .await
        .unwrap()
        .unwrap();

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "OK usage: HELP|GET|PUT|LIST\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");

    write.write_line(b"LIST /").await.unwrap();

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "OK 0\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");

    write.write_line(b"PUT /test").await.unwrap();

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "ERR usage: PUT file length newline data\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");

    write.write_line(b"GET").await.unwrap();

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "ERR usage: GET file [revision]\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");

    write.write_line(b"GET /test").await.unwrap();

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "ERR no such file\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");

    write.write_line(b"PUT test").await.unwrap();

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "ERR illegal file name\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");

    write.write_line(b"PUT /dir/test 2").await.unwrap();
    write.write_line(b"A").await.unwrap();

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "OK r1\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");

    write.write_line(b"GET /dir").await.unwrap();

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "ERR no such file\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");

    write.write_line(b"PUT /.. 2").await.unwrap();
    write.write_line(b"A").await.unwrap();

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "OK r1\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");

    write.write_line(b"/LIST").await.unwrap();

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "ERR illegal method: /LIST\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");

    write.write_line(b"LIST").await.unwrap();

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "ERR usage: LIST dir\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");

    write.write_line(b"LIST /").await.unwrap();

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "OK 2\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, ".. r1\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "dir/ DIR\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");

    write.write_line(b"PUT //test").await.unwrap();

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "ERR illegal file name\n");

    let mut buffer = String::new();
    assert!(
        timeout(TIMEOUT, read.read_line(&mut buffer))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    assert_eq!(buffer, "READY\n");
}

async fn spawn_app() -> (String, u16) {
    static TRACING_SUBSCRIBER_INIT: parking_lot::Once = parking_lot::Once::new();
    TRACING_SUBSCRIBER_INIT.call_once(tracing_subscriber::fmt::init);

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
