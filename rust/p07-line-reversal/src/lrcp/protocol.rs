use std::cmp;
use std::collections::HashMap;
use std::future::{self, Future};
use std::hash::Hash;
use std::io;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{atomic, Arc};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{mpsc, Notify};
use tokio::time::{timeout, Instant};

use parking_lot::Mutex;

use tracing::{debug, warn};

pub use crate::lrcp::packets::{Numeric, Packet, Payload, Session};

pub trait Receiver<P> {
    fn recv(&mut self) -> impl Future<Output = Result<Option<P>, io::Error>> + Send;
}

pub trait Sender<P> {
    fn send(&mut self, _: P) -> impl Future<Output = Result<(), io::Error>> + Send;
}

impl<P: Send> Receiver<P> for mpsc::UnboundedReceiver<P> {
    async fn recv(&mut self) -> Result<Option<P>, io::Error> {
        Ok(mpsc::UnboundedReceiver::recv(self).await)
    }
}

impl<P: Send> Sender<P> for mpsc::UnboundedSender<P> {
    async fn send(&mut self, packet: P) -> Result<(), io::Error> {
        mpsc::UnboundedSender::send(self, packet)
            .map_err(|e| io::Error::new(io::ErrorKind::WriteZero, e.to_string()))
    }
}

pub trait Endpoint<P, R: Receiver<P>, W: Sender<P>> {
    fn split(self) -> (R, W);
}

struct StreamUpstreamPart {
    closed: bool,
    buffer: Vec<u8>,
    waker: Option<Waker>,
    shutdown_waker: Option<Waker>,
}

struct StreamDownstreamPart {
    closed: bool,
    buffer: Vec<u8>,
    waker: Option<Waker>,
}

pub struct Stream<R, W> {
    exit_notify: Arc<Notify>,
    upstream_notify: Arc<Notify>,
    upstream_shutdown_notify: Arc<Notify>,
    upstream: Arc<Mutex<StreamUpstreamPart>>,
    downstream: Arc<Mutex<StreamDownstreamPart>>,
    _r: PhantomData<R>,
    _w: PhantomData<W>,
}

impl<R: Unpin, W: Unpin> AsyncRead for Stream<R, W> {
    #[tracing::instrument(skip(self, ctx, buffer))]
    fn poll_read(
        self: Pin<&mut Self>,
        ctx: &mut Context,
        buffer: &mut ReadBuf,
    ) -> Poll<Result<(), io::Error>> {
        debug!("poll_read");

        let downstream = &mut self.get_mut().downstream.lock();

        let downstream_len = downstream.buffer.len();
        if downstream_len > 0 {
            let buffer_size = buffer.remaining();
            let len = buffer_size.min(downstream_len);
            buffer.put_slice(downstream.buffer.drain(0..len).as_slice());
            Poll::Ready(Ok(()))
        } else if downstream.closed {
            warn!("poll_read closed");
            Poll::Ready(Ok(()))
        } else {
            debug!("poll_read waiting");
            downstream.waker = Some(ctx.waker().clone());
            Poll::Pending
        }
    }
}

impl<R: Unpin, W: Unpin> AsyncWrite for Stream<R, W> {
    #[tracing::instrument(skip(self, buffer))]
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context,
        buffer: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let len = buffer.len();

        debug!("poll_write {len}");

        let upstream = &mut self.get_mut().upstream.lock();

        if upstream.closed {
            debug!("poll_write closed");
            Poll::Ready(Ok(0))
        } else {
            upstream.buffer.reserve(len);
            for b in buffer {
                upstream.buffer.push(*b);
            }

            Poll::Ready(Ok(len))
        }
    }

    #[tracing::instrument(skip(self, ctx))]
    fn poll_flush(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Result<(), io::Error>> {
        debug!("poll_flush");

        if self.as_ref().upstream.lock().buffer.is_empty() {
            Poll::Ready(Ok(()))
        } else {
            let this = self.get_mut();

            let upstream = &mut this.upstream.lock();
            if upstream.closed {
                warn!("poll_flush: buffer not empty but closed");
                return Poll::Ready(Ok(()));
            }

            upstream.waker = Some(ctx.waker().clone());

            this.upstream_notify.notify_one();

            Poll::Pending
        }
    }

    #[tracing::instrument(skip(self, ctx))]
    fn poll_shutdown(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Result<(), io::Error>> {
        debug!("poll_shutdown");

        if self.as_ref().upstream.lock().closed {
            Poll::Ready(Ok(()))
        } else {
            let this = self.get_mut();

            let upstream = &mut this.upstream.lock();
            if upstream.closed {
                return Poll::Ready(Ok(()));
            }

            upstream.shutdown_waker = Some(ctx.waker().clone());

            this.upstream_shutdown_notify.notify_one();

            Poll::Pending
        }
    }
}

impl<R, W> Drop for Stream<R, W> {
    fn drop(&mut self) {
        self.exit_notify.notify_one();
    }
}

pub trait SocketHandler {
    const RETRASMISSION_TIMEOUT: Duration;
    const SESSION_EXPIRE_TIMEOUT: Duration;

    #[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
    #[tracing::instrument(skip(
        downstream,
        exit_notify,
        upstream_notify,
        upstream_shutdown_notify,
        upstream,
        receiver,
        sender
    ))]
    fn lrcp_handler<R, W>(
        start_connection: bool,
        handler_session: Numeric,
        exit_notify: Arc<Notify>,
        upstream_notify: Arc<Notify>,
        upstream_shutdown_notify: Arc<Notify>,
        upstream: Arc<Mutex<StreamUpstreamPart>>,
        downstream: Arc<Mutex<StreamDownstreamPart>>,
        mut receiver: R,
        mut sender: W,
    ) -> impl Future<Output = ()> + Send
    where
        R: Receiver<Packet> + Send + 'static,
        W: Sender<Packet> + Send + 'static,
    {
        async {
            let mut closing = false;
            let mut closed = false;

            let mut receiver_position = 0;

            let mut last_recv_timestamp = Instant::now();

            let (
                mut send_packet,
                mut last_packet_sent,
                mut sender_offset,
                mut sender_length,
                mut need_ack,
                mut current_timeout,
            ) = if start_connection {
                (
                    Some(Packet::Connect {
                        session: handler_session,
                    }),
                    Some(Packet::Connect {
                        session: handler_session,
                    }),
                    0,
                    0,
                    true,
                    Self::RETRASMISSION_TIMEOUT,
                )
            } else {
                (None, None, 0, 0, false, Self::SESSION_EXPIRE_TIMEOUT)
            };

            loop {
                debug!("lrcp_handler session: {handler_session:?}");

                let send = async {
                    if let Some(packet) = send_packet.as_ref() {
                        sender.send(packet.clone()).await
                    } else {
                        future::pending().await
                    }
                };

                tokio::select! {
                    _r = exit_notify.notified() => {
                        debug!("that's all folks");
                        break;
                    }

                    _r = upstream_notify.notified(), if !closed => {
                        debug!("wakeup!");
                    }

                    _r = upstream_shutdown_notify.notified(), if !closed => {
                        debug!("shutdown wakeup!");
                        closing = true;
                    }

                    Ok(()) = send, if send_packet.is_some() && !closed => {
                        debug!("sent packet");
                        last_packet_sent = send_packet.take();
                        if closing {
                            closed = true;
                        }
                    }

                    packet = timeout(current_timeout, receiver.recv()) => {
                        match packet {
                            Ok(Ok(Some(Packet::Data { session, pos, data }))) => {
                                debug!("received data packet, pos: {pos:?} len: {}", data.0.len());

                                assert_eq!(handler_session, session);

                                last_recv_timestamp = Instant::now();

                                if closed || closing {
                                    sender.send(Packet::Close { session }).await.ok();
                                } else {
                                    match pos.0.cmp(&receiver_position) {
                                        cmp::Ordering::Equal => {
                                            debug!("appending all data");
                                            {
                                                let mut downstream = downstream.lock();
                                                let len = data.write(&mut downstream.buffer, 0);
                                                receiver_position += len;
                                                if len > 0 {
                                                    if let Some(waker) = downstream.waker.take() {
                                                        waker.wake();
                                                    }
                                                }
                                            }

                                            sender.send(Packet::Ack { session, length: Numeric(receiver_position) }).await.ok();
                                        }

                                        cmp::Ordering::Less => {
                                            if pos.0 + data.0.len() as u32 > receiver_position {
                                                debug!("appending partial data");

                                                let mut downstream = downstream.lock();
                                                let len = data.write(&mut downstream.buffer, receiver_position - pos.0);
                                                receiver_position += len;
                                                if len > 0 {
                                                    if let Some(waker) = downstream.waker.take() {
                                                        waker.wake();
                                                    }
                                                }
                                            } else {
                                                debug!("ignore old data");
                                            }

                                            sender.send(Packet::Ack { session, length: Numeric(receiver_position) }).await.ok();
                                        }

                                        cmp::Ordering::Greater => {
                                            warn!("ignored data packet with position: {pos:?} > {receiver_position}");
                                        }
                                    }
                                }
                            }

                            Ok(Ok(Some(Packet::Ack { session, length }))) => {
                                debug!("received ack packet, length: {length:?}");

                                assert_eq!(handler_session, session);

                                last_recv_timestamp = Instant::now();

                                if closed || closing {
                                    sender.send(Packet::Close { session }).await.ok();
                                } else {
                                    match length.0.cmp(&sender_length) {
                                        cmp::Ordering::Equal => {
                                            sender_length = length.0;
                                            if sender_length > sender_offset {
                                                let mut upstream = upstream.lock();
                                                if (sender_length - sender_offset) as usize > upstream.buffer.len() {
                                                    warn!("invalid sender_length {sender_length} - {sender_offset} > {}", upstream.buffer.len());
                                                } else {
                                                    upstream.buffer.drain(..(sender_length - sender_offset) as usize);
                                                }
                                            }
                                            sender_offset = sender_length;

                                            need_ack = false;
                                            current_timeout = Self::SESSION_EXPIRE_TIMEOUT;
                                        }

                                        cmp::Ordering::Less => {
                                            if length.0 > sender_offset {
                                                let mut upstream = upstream.lock();
                                                if (length.0 - sender_offset) as usize > upstream.buffer.len() {
                                                    warn!("invalid length {length:?} - {sender_offset} > {}", upstream.buffer.len());
                                                } else {
                                                    upstream.buffer.drain(..(length.0 - sender_offset) as usize);
                                                }
                                                sender_length = length.0;
                                                sender_offset = sender_length;

                                                need_ack = false;
                                                current_timeout = Self::SESSION_EXPIRE_TIMEOUT;
                                            } else {
                                                warn!("invalid ack {length:?} <= {sender_offset}, too old");
                                            }
                                        }

                                        cmp::Ordering::Greater => {
                                            warn!("invalid ack {length:?} > {sender_length}, closing");

                                            closing = true;
                                        }
                                    }
                                }
                            }

                            Ok(Ok(Some(Packet::Close { session }))) => {
                                debug!("received close packet");

                                assert_eq!(handler_session, session);

                                last_recv_timestamp = Instant::now();

                                if closing {
                                    if closed {
                                        debug!("already closed, ignoring close packet");
                                    } else {
                                        sender.send(Packet::Close { session }).await.ok();
                                        closed = true;
                                    }
                                } else {
                                    sender.send(Packet::Close { session }).await.ok();

                                    let upstream = &mut upstream.lock();

                                    upstream.closed = true;

                                    if let Some(waker) = upstream.waker.take() {
                                        debug!("shutdown: wake upstream");
                                        waker.wake();
                                    }

                                    if let Some(waker) = upstream.shutdown_waker.take() {
                                        debug!("shutdown: wake shutdown upstream");
                                        waker.wake();
                                    }

                                    break;
                                }
                            }

                            Ok(Ok(Some(Packet::Connect { session }))) => {
                                debug!("received connect packet");

                                assert_eq!(handler_session, session);

                                last_recv_timestamp = Instant::now();

                                if start_connection {
                                    warn!("ignoring connection packet");
                                } else {
                                    sender.send(Packet::Ack { session, length: Numeric(0) }).await.ok();
                                }
                            }

                            Ok(Ok(None)) => {
                                warn!("TODO: recv Ok(Ok(None))");
                            }

                            Ok(Err(_)) => {
                                warn!("TODO: recv Ok(Err(_))");
                            }

                            Err(_) => {
                                if last_recv_timestamp.elapsed() > Self::SESSION_EXPIRE_TIMEOUT {
                                    warn!("session expire");

                                    sender.send(Packet::Close { session: handler_session }).await.ok();

                                    let upstream = &mut upstream.lock();

                                    upstream.closed = true;

                                    if let Some(waker) = upstream.waker.take() {
                                        debug!("shutdown: wake upstream");
                                        waker.wake();
                                    }

                                    if let Some(waker) = upstream.shutdown_waker.take() {
                                        debug!("shutdown: wake shutdown upstream");
                                        waker.wake();
                                    }

                                    break;
                                } else if need_ack {
                                    if let Some(packet) = last_packet_sent.take() {
                                        debug!("resend packet: {packet:?}");
                                        send_packet = Some(packet);
                                    } else {
                                        warn!("need ack true but no last packet sent");
                                    }
                                }
                            }
                        }
                    }
                }

                if need_ack {
                    debug!("need ack");
                } else if send_packet.is_none() {
                    if closing {
                        if closed {
                            debug!("closed");

                            let upstream = &mut upstream.lock();

                            upstream.closed = true;

                            if let Some(waker) = upstream.waker.take() {
                                debug!("shutdown: wake upstream");
                                waker.wake();
                            }

                            if let Some(waker) = upstream.shutdown_waker.take() {
                                debug!("shutdown: wake shutdown upstream");
                                waker.wake();
                            }
                        } else {
                            warn!("shutdown requested");
                            send_packet = Some(Packet::Close {
                                session: handler_session,
                            });
                        }
                    } else {
                        let upstream = &mut upstream.lock();
                        if let Some(data) = Payload::new(&upstream.buffer) {
                            sender_length += data.0.len() as u32;
                            send_packet = Some(Packet::Data {
                                session: handler_session,
                                pos: Numeric(sender_offset),
                                data,
                            });

                            need_ack = true;
                            current_timeout = Self::RETRASMISSION_TIMEOUT;
                        } else {
                            debug!("no payload");
                            if let Some(waker) = upstream.waker.take() {
                                debug!("wake upstream");
                                waker.wake();
                            }
                        }
                    }
                }
            }
        }
    }
}

struct Connection {
    upstream_sender: mpsc::UnboundedSender<Packet>,
}

impl Connection {
    fn new<H, ADDR, W>(
        addr: ADDR,
        session: Numeric,
        mut downstream_sender: W,
        listener_sender: &mpsc::UnboundedSender<
            Stream<mpsc::UnboundedSender<Packet>, mpsc::UnboundedReceiver<Packet>>,
        >,
    ) -> Self
    where
        W: Sender<(ADDR, Packet)> + Send + 'static,
        H: SocketHandler,
        ADDR: Send + Copy + 'static,
    {
        let (upstream_sender, upstream_receiver) = mpsc::unbounded_channel();
        let (anon_downstream_sender, mut downstream_receiver) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            loop {
                if let Some(packet) = downstream_receiver.recv().await {
                    if let Err(err) = downstream_sender.send((addr, packet)).await {
                        warn!("sending downstream failed: {err}");
                    }
                } else {
                    warn!("connection closed? exit");
                    break;
                }
            }
        });

        let upstream = Arc::new(Mutex::new(StreamUpstreamPart {
            closed: false,
            buffer: vec![],
            waker: None,
            shutdown_waker: None,
        }));
        let downstream = Arc::new(Mutex::new(StreamDownstreamPart {
            closed: false,
            buffer: vec![],
            waker: None,
        }));

        let exit_notify = Arc::new(Notify::new());
        let upstream_notify = Arc::new(Notify::new());
        let upstream_shutdown_notify = Arc::new(Notify::new());

        let _handle = {
            let exit_notify = Arc::clone(&exit_notify);
            let upstream_notify = Arc::clone(&upstream_notify);
            let upstream_shutdown_notify = Arc::clone(&upstream_shutdown_notify);
            let upstream = Arc::clone(&upstream);
            let downstream = Arc::clone(&downstream);
            tokio::spawn(async move {
                H::lrcp_handler(
                    false,
                    session,
                    exit_notify,
                    upstream_notify,
                    upstream_shutdown_notify,
                    upstream,
                    downstream,
                    upstream_receiver,
                    anon_downstream_sender,
                )
                .await;
            })
        };

        listener_sender
            .send(Stream {
                exit_notify,
                upstream_notify,
                upstream_shutdown_notify,
                upstream,
                downstream,
                _r: PhantomData,
                _w: PhantomData,
            })
            .expect("cannot send stream");

        Self { upstream_sender }
    }
}

#[derive(Debug)]
pub struct Listener {
    listener_receiver: mpsc::UnboundedReceiver<
        Stream<mpsc::UnboundedSender<Packet>, mpsc::UnboundedReceiver<Packet>>,
    >,
}

impl Listener {
    #[tracing::instrument]
    pub async fn accept(
        &mut self,
    ) -> Result<Stream<mpsc::UnboundedSender<Packet>, mpsc::UnboundedReceiver<Packet>>, io::Error>
    {
        self.listener_receiver
            .recv()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "not connected"))
    }
}

pub struct Socket<H>(PhantomData<H>);

impl<H> Socket<H> {
    fn next_session() -> Session {
        static SESSION: atomic::AtomicU32 = atomic::AtomicU32::new(0);

        Numeric(SESSION.fetch_add(1, atomic::Ordering::Relaxed))
    }
}

impl<H: SocketHandler + Send> Socket<H> {
    #[tracing::instrument(skip(endpoint))]
    pub fn listener<ADDR, R, W, E>(endpoint: E) -> Result<Listener, io::Error>
    where
        R: Receiver<(ADDR, Packet)> + Send + 'static,
        W: Sender<(ADDR, Packet)> + Send + Clone + 'static,
        E: Endpoint<(ADDR, Packet), R, W>,
        ADDR: std::fmt::Debug + Eq + Hash + Copy + Send + 'static,
    {
        debug!("listener");

        let (mut receiver, downstream_sender) = endpoint.split();

        let (listener_sender, listener_receiver) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut connections = HashMap::new();
            loop {
                match receiver.recv().await {
                    Ok(Some((addr, packet))) => {
                        let session = packet.session();

                        let result = connections
                            .entry((addr, session))
                            .or_insert_with(|| {
                                debug!("added connection ({addr:?}, {session:?})");
                                Connection::new::<H, ADDR, W>(
                                    addr,
                                    session,
                                    downstream_sender.clone(),
                                    &listener_sender,
                                )
                            })
                            .upstream_sender
                            .send(packet);
                        if let Err(e) = result {
                            warn!("sending upstream failed: {e}");
                        }
                    }
                    Ok(None) => {
                        warn!("sending upstream, got None packet");
                    }
                    Err(e) => {
                        warn!("sending upstream, got error {e}");
                    }
                }
            }
        });

        Ok(Listener { listener_receiver })
    }

    #[tracing::instrument(skip(endpoint))]
    pub async fn connect<R, W, E>(endpoint: E) -> Result<Stream<R, W>, io::Error>
    where
        R: Receiver<Packet> + Send + 'static,
        W: Sender<Packet> + Send + 'static,
        E: Endpoint<Packet, R, W>,
    {
        let session = Self::next_session();

        let (receiver, sender) = endpoint.split();

        let upstream = Arc::new(Mutex::new(StreamUpstreamPart {
            closed: false,
            buffer: vec![],
            waker: None,
            shutdown_waker: None,
        }));
        let downstream = Arc::new(Mutex::new(StreamDownstreamPart {
            closed: false,
            buffer: vec![],
            waker: None,
        }));

        let exit_notify = Arc::new(Notify::new());
        let upstream_notify = Arc::new(Notify::new());
        let upstream_shutdown_notify = Arc::new(Notify::new());

        let _handle = {
            let exit_notify = Arc::clone(&exit_notify);
            let upstream_notify = Arc::clone(&upstream_notify);
            let upstream_shutdown_notify = Arc::clone(&upstream_shutdown_notify);
            let upstream = Arc::clone(&upstream);
            let downstream = Arc::clone(&downstream);
            tokio::spawn(async move {
                H::lrcp_handler(
                    true,
                    session,
                    exit_notify,
                    upstream_notify,
                    upstream_shutdown_notify,
                    upstream,
                    downstream,
                    receiver,
                    sender,
                )
                .await;
            })
        };

        Ok(Stream {
            exit_notify,
            upstream_notify,
            upstream_shutdown_notify,
            upstream,
            downstream,
            _r: PhantomData,
            _w: PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{split, AsyncReadExt, AsyncWriteExt};
    use tokio::sync::mpsc;
    use tokio::task::yield_now;
    use tokio::time::{sleep, timeout};

    use tracing::Instrument;

    use super::*;

    const RETRASMISSION_TIMEOUT: Duration = Duration::from_millis(100);
    const SESSION_EXPIRE_TIMEOUT: Duration = Duration::from_millis(600);
    const DELAY: Duration = Duration::from_millis(50);

    const _: () =
        assert!(SESSION_EXPIRE_TIMEOUT.as_millis() > RETRASMISSION_TIMEOUT.as_millis() * 2);
    const _: () = assert!(RETRASMISSION_TIMEOUT.as_millis() >= DELAY.as_millis() * 2);
    const _: () = assert!(DELAY.as_millis() >= 5);

    fn init_tracing_subscriber() {
        static TRACING_SUBSCRIBER_INIT: parking_lot::Once = parking_lot::Once::new();
        TRACING_SUBSCRIBER_INIT.call_once(tracing_subscriber::fmt::init);
    }

    struct TestSocketHandler;

    impl SocketHandler for TestSocketHandler {
        const RETRASMISSION_TIMEOUT: Duration = RETRASMISSION_TIMEOUT;
        const SESSION_EXPIRE_TIMEOUT: Duration = SESSION_EXPIRE_TIMEOUT;
    }

    struct EchoEndpoint;

    impl Endpoint<Packet, EchoEndpointRead, EchoEndpointWrite> for EchoEndpoint {
        fn split(self) -> (EchoEndpointRead, EchoEndpointWrite) {
            let (sender, receiver) = mpsc::unbounded_channel();

            (EchoEndpointRead(receiver), EchoEndpointWrite(sender))
        }
    }

    struct EchoEndpointRead(mpsc::UnboundedReceiver<Packet>);

    impl Receiver<Packet> for EchoEndpointRead {
        async fn recv(&mut self) -> Result<Option<Packet>, io::Error> {
            Ok(self.0.recv().await)
        }
    }

    struct EchoEndpointWrite(mpsc::UnboundedSender<Packet>);

    impl Sender<Packet> for EchoEndpointWrite {
        async fn send(&mut self, packet: Packet) -> Result<(), io::Error> {
            let r = match packet {
                Packet::Connect { session } => self.0.send(Packet::Ack {
                    session,
                    length: Numeric(0),
                }),
                Packet::Ack { session, length } => self.0.send(Packet::Ack { session, length }),
                Packet::Close { session } => self.0.send(Packet::Close { session }),
                Packet::Data { session, pos, data } => {
                    self.0.send(Packet::Data { session, pos, data })
                }
            };

            r.map_err(|e| io::Error::new(io::ErrorKind::WriteZero, e))
        }
    }

    struct TestEndpoint<P> {
        sender: mpsc::UnboundedSender<P>,
        receiver: mpsc::UnboundedReceiver<P>,
    }

    impl<P: Send> Endpoint<P, mpsc::UnboundedReceiver<P>, mpsc::UnboundedSender<P>>
        for TestEndpoint<P>
    {
        fn split(self) -> (mpsc::UnboundedReceiver<P>, mpsc::UnboundedSender<P>) {
            (self.receiver, self.sender)
        }
    }

    #[tokio::test]
    async fn test_echo_connect() {
        init_tracing_subscriber();

        let span = tracing::info_span!("test_connect");

        async {
            let echo_endpoint = EchoEndpoint;

            let stream = Socket::<TestSocketHandler>::connect(echo_endpoint)
                .await
                .unwrap();
            let (mut read, mut write) = split(stream);

            write.write_all(b"Hello World!\n").await.unwrap();
            write.flush().await.unwrap();

            let mut buffer = [0; 1024];
            let len = read.read(&mut buffer).await.unwrap();

            assert_eq!(&buffer[0..len], b"Hello World!\n");
        }
        .instrument(span)
        .await;
    }

    #[tokio::test]
    async fn test_echo_connect_big() {
        init_tracing_subscriber();

        let span = tracing::info_span!("test_connect_big");

        async {
            let echo_endpoint = EchoEndpoint;

            let stream = Socket::<TestSocketHandler>::connect(echo_endpoint)
                .await
                .unwrap();
            let (mut read, mut write) = split(stream);

            let buffer = [b'A'; 10_000];

            write.write_all(&buffer).await.unwrap();
            write.flush().await.unwrap();

            let mut buffer = [0; 1024];
            let len = read.read(&mut buffer).await.unwrap();

            assert_eq!(&buffer[0..len], &[b'A'; 1024]);
        }
        .instrument(span)
        .await;
    }

    #[tokio::test]
    async fn test_echo_shutdown() {
        init_tracing_subscriber();

        let echo_endpoint = EchoEndpoint;

        let stream = Socket::<TestSocketHandler>::connect(echo_endpoint)
            .await
            .unwrap();
        let (mut read, mut write) = split(stream);

        write.write_all(b"Hello World!\n").await.unwrap();
        write.flush().await.unwrap();

        let mut buffer = [0; 1024];
        let _ = read.read(&mut buffer).await.unwrap();

        timeout(Duration::from_millis(100), write.shutdown())
            .await
            .unwrap()
            .unwrap();

        let stream = read.unsplit(write);

        drop(stream);

        yield_now().await;
    }

    #[tokio::test]
    async fn test_connect() {
        init_tracing_subscriber();

        let (upstream_sender, mut upstream_receiver) = mpsc::unbounded_channel();
        let (downstream_sender, downstream_receiver) = mpsc::unbounded_channel();

        let test_endpoint = TestEndpoint {
            sender: upstream_sender,
            receiver: downstream_receiver,
        };

        let stream = Socket::<TestSocketHandler>::connect(test_endpoint)
            .await
            .unwrap();
        let (_read, mut write) = split(stream);

        let session = match timeout(DELAY, upstream_receiver.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Packet::Connect { session } => session,
            packet => panic!("invalid packet: {packet:?}"),
        };

        sleep(RETRASMISSION_TIMEOUT + DELAY).await;
        match timeout(Duration::from_millis(100), upstream_receiver.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Packet::Connect { .. } => {}
            packet => panic!("invalid packet: {packet:?}"),
        }

        sleep(RETRASMISSION_TIMEOUT + DELAY).await;
        match timeout(Duration::from_millis(100), upstream_receiver.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Packet::Connect { .. } => {}
            packet => panic!("invalid packet: {packet:?}"),
        }

        downstream_sender
            .send(Packet::Ack {
                session,
                length: Numeric(0),
            })
            .unwrap();

        write.write_all(b"Hello World!").await.unwrap();
        timeout(Duration::from_millis(100), write.flush())
            .await
            .ok();

        match timeout(DELAY, upstream_receiver.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Packet::Data { pos, data, .. } => {
                assert_eq!(Numeric(0), pos);
                assert_eq!(Payload("Hello World!".to_string()), data);
            }
            packet => panic!("invalid packet: {packet:?}"),
        }

        sleep(RETRASMISSION_TIMEOUT + DELAY).await;
        match timeout(Duration::from_millis(100), upstream_receiver.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Packet::Data { .. } => {}
            packet => panic!("invalid packet: {packet:?}"),
        }
    }

    #[tokio::test]
    async fn test_accept() {
        init_tracing_subscriber();

        let (upstream_sender, mut upstream_receiver) = mpsc::unbounded_channel();
        let (downstream_sender, downstream_receiver) = mpsc::unbounded_channel();

        let endpoint = TestEndpoint::<((), Packet)> {
            sender: upstream_sender,
            receiver: downstream_receiver,
        };

        let mut listener = Socket::<TestSocketHandler>::listener(endpoint).unwrap();

        let session = Numeric(666);

        downstream_sender
            .send(((), Packet::Connect { session }))
            .unwrap();

        let _stream = timeout(Duration::from_millis(100), listener.accept())
            .await
            .unwrap()
            .unwrap();

        let packet = timeout(Duration::from_millis(100), upstream_receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                (),
                Packet::Ack {
                    session,
                    length: Numeric(0)
                }
            ),
            packet
        );
    }
}
