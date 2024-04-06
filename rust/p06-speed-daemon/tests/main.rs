use std::sync::Once;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use tracing::info;

use p06_speed_daemon::wire::{ReadFrom, WriteTo};

#[tokio::test]
async fn test_session() {
    let (address, port) = spawn_app().await;

    {
        let mut stream = TcpStream::connect(&format!("{address}:{port}"))
            .await
            .unwrap();
        let (_, mut write) = stream.split();

        p06_speed_daemon::wire::IAmCamera {
            road: 123,
            mile: 8,
            limit: 60,
        }
        .write_to(&mut write)
        .await
        .unwrap();

        p06_speed_daemon::wire::Plate {
            plate: "UN1X".to_string(),
            timestamp: 0,
        }
        .write_to(&mut write)
        .await
        .unwrap();

        write.flush().await.unwrap();
    }

    {
        let mut stream = TcpStream::connect(&format!("{address}:{port}"))
            .await
            .unwrap();
        let (_, mut write) = stream.split();

        p06_speed_daemon::wire::IAmCamera {
            road: 123,
            mile: 9,
            limit: 60,
        }
        .write_to(&mut write)
        .await
        .unwrap();

        p06_speed_daemon::wire::Plate {
            plate: "UN1X".to_string(),
            timestamp: 45,
        }
        .write_to(&mut write)
        .await
        .unwrap();

        write.flush().await.unwrap();
    }

    {
        let mut stream = TcpStream::connect(&format!("{address}:{port}"))
            .await
            .unwrap();
        let (mut read, mut write) = stream.split();
        p06_speed_daemon::wire::IAmDispatcher { roads: vec![123] }
            .write_to(&mut write)
            .await
            .unwrap();
        let ticket = timeout(
            Duration::from_millis(1000),
            p06_speed_daemon::wire::Ticket::read_from(&mut read),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(
            ticket,
            p06_speed_daemon::wire::Ticket {
                plate: "UN1X".to_string(),
                road: 123,
                mile1: 8,
                timestamp1: 0,
                mile2: 9,
                timestamp2: 45,
                speed: 8000,
            }
        );
    }
}

#[tokio::test]
async fn test_invalid_message() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .unwrap();
    let (mut read, mut write) = stream.split();
    p06_speed_daemon::wire::Plate {
        plate: "UN1X".to_string(),
        timestamp: 0,
    }
    .write_to(&mut write)
    .await
    .unwrap();

    assert_eq!(
        p06_speed_daemon::wire::Error {
            msg: "invalid message: 0x20".to_string()
        },
        p06_speed_daemon::wire::Error::read_from(&mut read)
            .await
            .unwrap()
    );

    if let Ok(r) = timeout(Duration::from_millis(100), read.read_u8()).await {
        match r {
            Ok(_) => panic!("got message"),
            Err(_) => {} // ok
        }
    } else {
        panic!("timeout");
    }
}

#[tokio::test]
async fn test_heartbeat_camera() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .unwrap();
    let (mut read, mut write) = stream.split();

    p06_speed_daemon::wire::IAmCamera {
        road: 123,
        mile: 8,
        limit: 60,
    }
    .write_to(&mut write)
    .await
    .unwrap();

    p06_speed_daemon::wire::WantHeartbeat { interval: 1 }
        .write_to(&mut write)
        .await
        .unwrap();

    for i in 0..10 {
        if let Ok(Ok(p06_speed_daemon::wire::Heartbeat)) = timeout(
            Duration::from_millis(1000),
            p06_speed_daemon::wire::Heartbeat::read_from(&mut read),
        )
        .await
        {
            // ok
        } else {
            panic!("expecting heartbeat: {i}");
        }
    }
}

#[tokio::test]
async fn test_heartbeat_dispatcher() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(&format!("{address}:{port}"))
        .await
        .unwrap();
    let (mut read, mut write) = stream.split();

    p06_speed_daemon::wire::IAmDispatcher { roads: vec![123] }
        .write_to(&mut write)
        .await
        .unwrap();

    p06_speed_daemon::wire::WantHeartbeat { interval: 1 }
        .write_to(&mut write)
        .await
        .unwrap();

    for i in 0..10 {
        if let Ok(Ok(p06_speed_daemon::wire::Heartbeat)) = timeout(
            Duration::from_millis(1000),
            p06_speed_daemon::wire::Heartbeat::read_from(&mut read),
        )
        .await
        {
            // ok
        } else {
            panic!("expecting heartbeat: {i}");
        }
    }
}

async fn spawn_app() -> (String, u16) {
    static TRACING_SUBSCRIBER_INIT: Once = Once::new();
    TRACING_SUBSCRIBER_INIT.call_once(tracing_subscriber::fmt::init);

    let address = "127.0.0.1";

    let listener = TcpListener::bind(&format!("{}:0", address))
        .await
        .expect("cannot bind app");
    let port = listener
        .local_addr()
        .expect("cannot get local address")
        .port();

    tokio::spawn(async move {
        p06_speed_daemon::run(listener).await.expect("run failed");
    });

    info!("spawned app {address}:{port}");

    (address.to_string(), port)
}
