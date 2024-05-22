// use std::sync::Once;
// use std::time::Duration;

// use tracing::info;

// use p06_speed_daemon::{run, wire::{self, ReadFrom, WriteTo}};

// #[test]
// fn test_session() {
//     block_on(|reactor| async move {
//         let (address, port) = spawn_app().await;

//         {
//             let mut stream = TcpStream::connect(reactor, format!("{address}:{port}"))
//                 .await
//                 .unwrap();
//             let (_, mut write) = stream.split();

//             wire::IAmCamera {
//                 road: 123,
//                 mile: 8,
//                 limit: 60,
//             }
//             .write_to(&mut write)
//                 .await
//                 .unwrap();

//             wire::Plate {
//                 plate: "UN1X".to_string(),
//                 timestamp: 0,
//             }
//             .write_to(&mut write)
//                 .await
//                 .unwrap();

//             write.flush().await.unwrap();
//         }

//         {
//             let mut stream = TcpStream::connect(reactor, format!("{address}:{port}"))
//                 .await
//                 .unwrap();
//             let (_, mut write) = stream.split();

//             wire::IAmCamera {
//                 road: 123,
//                 mile: 9,
//                 limit: 60,
//             }
//             .write_to(&mut write)
//                 .await
//                 .unwrap();

//             wire::Plate {
//                 plate: "UN1X".to_string(),
//                 timestamp: 45,
//             }
//             .write_to(&mut write)
//                 .await
//                 .unwrap();

//             write.flush().await.unwrap();
//         }

//         {
//             let mut stream = TcpStream::connect(reactor, format!("{address}:{port}"))
//                 .await
//                 .unwrap();
//             let (mut read, mut write) = stream.split();
//             wire::IAmDispatcher { roads: vec![123] }
//             .write_to(&mut write)
//                 .await
//                 .unwrap();
//             let ticket = timeout(
//                 Duration::from_millis(1000),
//                 wire::Ticket::read_from(&mut read),
//             )
//                 .await
//                 .unwrap()
//                 .unwrap();

//             assert_eq!(
//                 ticket,
//                 wire::Ticket {
//                     plate: "UN1X".to_string(),
//                     road: 123,
//                     mile1: 8,
//                     timestamp1: 0,
//                     mile2: 9,
//                     timestamp2: 45,
//                     speed: 8000,
//                 }
//             );
//         }
//     });
// }

// #[test]
// fn test_invalid_message() {
//     block_on(|reactor| async move {
//         let (address, port) = spawn_app().await;

//         let mut stream = TcpStream::connect(&format!("{address}:{port}"))
//             .await
//             .unwrap();
//         let (mut read, mut write) = stream.split();
//         wire::Plate {
//             plate: "UN1X".to_string(),
//             timestamp: 0,
//         }
//         .write_to(&mut write)
//             .await
//             .unwrap();

//         assert_eq!(
//             wire::Error {
//                 msg: "invalid message: 0x20".to_string()
//             },
//             wire::Error::read_from(&mut read)
//                 .await
//                 .unwrap()
//         );

//         if let Ok(r) = timeout(Duration::from_millis(100), read.read_u8()).await {
//             assert!(r.is_err(), "got message");
//         } else {
//             panic!("timeout");
//         }
//     });
// }

// #[test]
// fn test_heartbeat_camera() {
//     block_on(|reactor| async move {
//         let (address, port) = spawn_app().await;

//         let mut stream = TcpStream::connect(&format!("{address}:{port}"))
//             .await
//             .unwrap();
//         let (mut read, mut write) = stream.split();

//         wire::IAmCamera {
//             road: 123,
//             mile: 8,
//             limit: 60,
//         }
//         .write_to(&mut write)
//             .await
//             .unwrap();

//         wire::WantHeartbeat { interval: 1 }
//         .write_to(&mut write)
//             .await
//             .unwrap();

//         for i in 0..10 {
//             if let Ok(Ok(wire::Heartbeat)) = timeout(
//                 Duration::from_millis(1000),
//                 wire::Heartbeat::read_from(&mut read),
//             )
//                 .await
//             {
//                 // ok
//             } else {
//                 panic!("expecting heartbeat: {i}");
//             }
//         }
//     });
// }

// #[test]
// fn test_heartbeat_dispatcher() {
//     block_on(|reactor| async move {
//         let (address, port) = spawn_app().await;

//         let mut stream = TcpStream::connect(&format!("{address}:{port}"))
//             .await
//             .unwrap();
//         let (mut read, mut write) = stream.split();

//         wire::IAmDispatcher { roads: vec![123] }
//         .write_to(&mut write)
//             .await
//             .unwrap();

//         wire::WantHeartbeat { interval: 1 }
//         .write_to(&mut write)
//             .await
//             .unwrap();

//         for i in 0..10 {
//             if let Ok(Ok(wire::Heartbeat)) = timeout(
//                 Duration::from_millis(1000),
//                 wire::Heartbeat::read_from(&mut read),
//             )
//                 .await
//             {
//                 // ok
//             } else {
//                 panic!("expecting heartbeat: {i}");
//             }
//         }
//     });
// }

// async fn spawn_app(reactor: Reactor) -> (String, u16) {
//     static INIT_TRACING_SUBSCRIBER: Once = Once::new();
//     INIT_TRACING_SUBSCRIBER.call_once(tracing_subscriber::fmt::init);

//     let address = "127.0.0.1";

//     let listener = TcpListener::bind(reactor.clone(), format!("{address}:0"))
//         .await
//         .expect("cannot bind app");
//     let port = listener
//         .local_addr()
//         .expect("cannot get local address")
//         .port();

//     reactor.clone().spawn(async move {
//         run(reactor, listener).await.expect("run failed");
//     });

//     info!("spawned app {address}:{port}");

//     (address.to_string(), port)
// }
