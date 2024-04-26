use std::time::Duration;

use futures::{SinkExt, StreamExt};

use tokio::io::{BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use tokio_util::codec::{FramedRead, FramedWrite};

use tracing::info;

use p09_job_centre::{protocol, run};

const TIMEOUT: Duration = Duration::from_millis(100);

#[tokio::test]
async fn test_session() {
    let (address, port) = spawn_app().await;

    let mut stream = TcpStream::connect(format!("{address}:{port}"))
        .await
        .unwrap();
    let (read, write) = stream.split();
    let mut read = FramedRead::new(BufReader::new(read), protocol::ResponseCodec::new());
    let mut write = FramedWrite::new(BufWriter::new(write), protocol::RequestCodec::new());

    let job = serde_json::json!({"title": "example-job"});

    write
        .send(protocol::Request::Put {
            queue: "queue1".to_string(),
            job: job.clone(),
            priority: 123,
        })
        .await
        .unwrap();
    write.flush().await.unwrap();

    let Ok(protocol::Response::Id(id)) = timeout(TIMEOUT, read.next()).await.unwrap().unwrap()
    else {
        panic!("invalid response");
    };

    write
        .send(protocol::Request::Get {
            queues: vec!["queue1".to_string()],
            wait: false,
        })
        .await
        .unwrap();
    write.flush().await.unwrap();

    let Ok(protocol::Response::JobRef(job_ref_id, job_ref_job, priority, queue)) =
        timeout(TIMEOUT, read.next()).await.unwrap().unwrap()
    else {
        panic!("invalid response")
    };

    assert_eq!(id, job_ref_id);
    assert_eq!(job, job_ref_job);
    assert_eq!(123, priority);
    assert_eq!("queue1", queue);

    write.send(protocol::Request::Abort { id }).await.unwrap();
    write.flush().await.unwrap();

    let Ok(protocol::Response::Ok) = timeout(TIMEOUT, read.next()).await.unwrap().unwrap() else {
        panic!("invalid response")
    };

    write
        .send(protocol::Request::Get {
            queues: vec!["queue1".to_string()],
            wait: false,
        })
        .await
        .unwrap();
    write.flush().await.unwrap();

    let Ok(protocol::Response::JobRef(job_ref_id, job_ref_job, priority, queue)) =
        timeout(TIMEOUT, read.next()).await.unwrap().unwrap()
    else {
        panic!("invalid response")
    };

    assert_eq!(id, job_ref_id);
    assert_eq!(job, job_ref_job);
    assert_eq!(123, priority);
    assert_eq!("queue1", queue);

    write.send(protocol::Request::Delete { id }).await.unwrap();
    write.flush().await.unwrap();

    let Ok(protocol::Response::Ok) = timeout(TIMEOUT, read.next()).await.unwrap().unwrap() else {
        panic!("invalid response")
    };

    write
        .send(protocol::Request::Get {
            queues: vec!["queue1".to_string()],
            wait: false,
        })
        .await
        .unwrap();
    write.flush().await.unwrap();

    let Ok(protocol::Response::NoJob) = timeout(TIMEOUT, read.next()).await.unwrap().unwrap()
    else {
        panic!("invalid response")
    };
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
