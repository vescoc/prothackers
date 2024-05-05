use std::time::Duration;

use futures::{SinkExt, StreamExt};

use tokio::io::{BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;

use tokio_util::codec::{FramedRead, FramedWrite};

use parking_lot::Once;

use tracing::{debug, info, instrument};

use p11_pest_control::{codec::packets, run, DefaultProvider};

const TIMEOUT: Duration = Duration::from_millis(100);

fn init_tracing_subscriber() {
    static INIT_TRACING_SUBSCRIBER: Once = Once::new();
    INIT_TRACING_SUBSCRIBER.call_once(tracing_subscriber::fmt::init);
}

#[tokio::test]
async fn test_session() {
    init_tracing_subscriber();

    let (authority_server_address, authority_server_port, mut endpoints) =
        spawn_authority_server_app().await;

    let (address, port) = spawn_app(authority_server_address, authority_server_port).await;

    let stream = TcpStream::connect(format!("{address}:{port}"))
        .await
        .unwrap();
    let (read, write) = stream.into_split();
    let mut reader = FramedRead::new(BufReader::new(read), packets::PacketCodec::new());
    let mut writer = FramedWrite::new(BufWriter::new(write), packets::PacketCodec::new());

    writer
        .send(packets::hello::Packet::new().into())
        .await
        .unwrap();
    assert_eq!(
        timeout(TIMEOUT, reader.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        packets::hello::Packet::new().into()
    );

    writer
        .send(
            packets::site_visit::Packet::new(
                12345,
                vec![packets::site_visit::Population::new("long-tailed rat", 20)],
            )
            .into(),
        )
        .await
        .unwrap();

    let (upstream, mut downstream) = timeout(TIMEOUT, endpoints.recv()).await.unwrap().unwrap();
    assert_eq!(
        timeout(TIMEOUT, downstream.recv()).await.unwrap().unwrap(),
        packets::hello::Packet::new().into()
    );

    upstream.send(packets::hello::Packet::new().into()).unwrap();

    assert_eq!(
        timeout(TIMEOUT, downstream.recv()).await.unwrap().unwrap(),
        packets::dial_authority::Packet::new(12345).into()
    );
    upstream
        .send(
            packets::target_populations::Packet::new(
                12345,
                vec![packets::target_populations::Population::new(
                    "long-tailed rat",
                    0,
                    10,
                )],
            )
            .into(),
        )
        .unwrap();

    assert_eq!(
        timeout(TIMEOUT, downstream.recv()).await.unwrap().unwrap(),
        packets::create_policy::Packet::new(
            "long-tailed rat",
            packets::create_policy::PolicyAction::Cull
        )
        .into()
    );

    upstream
        .send(packets::policy_result::Packet::new(123).into())
        .unwrap();

    writer
        .send(
            packets::site_visit::Packet::new(
                12345,
                vec![packets::site_visit::Population::new("long-tailed rat", 10)],
            )
            .into(),
        )
        .await
        .unwrap();

    assert_eq!(
        timeout(TIMEOUT, downstream.recv()).await.unwrap().unwrap(),
        packets::delete_policy::Packet::new(123).into()
    );

    timeout(TIMEOUT, downstream.recv()).await.unwrap_err();
}

async fn spawn_app(authority_server_address: String, authority_server_port: u16) -> (String, u16) {
    let address = "127.0.0.1";

    let authority_server_provider =
        DefaultProvider::new(authority_server_address, authority_server_port);

    let listener = TcpListener::bind(&format!("{address}:0"))
        .await
        .expect("cannot bind app");
    let port = listener
        .local_addr()
        .expect("cannot get local address")
        .port();

    tokio::spawn(async move {
        run(listener, authority_server_provider)
            .await
            .expect("run failed");
    });

    info!("spawned app {address}:{port}");

    (address.to_string(), port)
}

#[instrument]
#[allow(clippy::type_complexity)]
async fn spawn_authority_server_app() -> (
    String,
    u16,
    mpsc::UnboundedReceiver<(
        mpsc::UnboundedSender<packets::Packet>,
        mpsc::UnboundedReceiver<packets::Packet>,
    )>,
) {
    let (endpoints_tx, endpoints) = mpsc::unbounded_channel();

    let address = "127.0.0.1";

    let listener = TcpListener::bind(&format!("{address}:0"))
        .await
        .expect("cannot bind authority server");
    let port = listener
        .local_addr()
        .expect("cannot get local address for authority server")
        .port();

    tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.expect("cannot accept");

            let (read, write) = socket.into_split();
            let mut reader = FramedRead::new(BufReader::new(read), packets::PacketCodec::new());
            let mut writer = FramedWrite::new(BufWriter::new(write), packets::PacketCodec::new());

            let (upstream, mut upstream_rx) = mpsc::unbounded_channel();
            let (downstream_tx, downstream) = mpsc::unbounded_channel();

            endpoints_tx
                .send((upstream, downstream))
                .expect("cannot send endpoinst");

            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        Some(Ok(packet)) = reader.next() => {
                            debug!("reader packet: {packet:?}");
                            downstream_tx.send(packet).expect("cannot send packet downstream");
                            debug!("reader packet, sent");
                        }

                        Some(packet) = upstream_rx.recv() => {
                            debug!("writer packet: {packet:?}");
                            writer.send(packet).await.expect("cannot send packet upstream");
                            debug!("writer packet, sent");
                        }
                    }
                }
            });
        }
    });

    info!("spawned authority server app {address}:{port}");

    (address.to_string(), port, endpoints)
}
