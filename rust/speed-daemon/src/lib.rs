use std::collections::{HashMap, HashSet};
use std::future;
use std::sync::{atomic, Arc, Mutex};
use tokio::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{
    tcp::{ReadHalf, WriteHalf},
    TcpListener, TcpStream,
};
use tokio::sync::mpsc;
use tokio::time;

use tracing::{debug, info, warn};

pub mod controller;
pub mod wire;

use controller::Controller;
use wire::{ReadFrom, TaggedMessage, WriteTo};

enum ControllerMessage {
    AddDispatcher(
        usize,
        HashSet<u16>,
        mpsc::UnboundedSender<controller::Ticket>,
    ),
    RemoveDispatcher(usize),
    Plate(controller::Plate),
}

type Cameras = Arc<Mutex<HashMap<u16, (u16, usize)>>>;

#[derive(Default)]
struct Dispatchers {
    dispatchers: Vec<(
        usize,
        HashSet<u16>,
        mpsc::UnboundedSender<controller::Ticket>,
    )>,
    pending_tickets: Vec<controller::Ticket>,
}

impl Dispatchers {
    fn add_dispatcher(
        &mut self,
        id: usize,
        roads: HashSet<u16>,
        ticket_sender: mpsc::UnboundedSender<controller::Ticket>,
    ) -> Result<(), anyhow::Error> {
        self.dispatchers.push((id, roads, ticket_sender));

        self.send_pending_tickets()
    }

    fn remove_dispatcher(&mut self, removed_id: usize) {
        self.dispatchers.retain(|(id, _, _)| *id != removed_id);
    }

    fn send_tickets(&mut self, mut tickets: Vec<controller::Ticket>) -> Result<(), anyhow::Error> {
        self.pending_tickets.append(&mut tickets);

        self.send_pending_tickets()
    }

    fn send_pending_tickets(&mut self) -> Result<(), anyhow::Error> {
        let mut pending_tickets = vec![];

        for ticket in self.pending_tickets.drain(..) {
            if let Some(sender) = self.dispatchers.iter().find_map(|(_, roads, sender)| {
                if roads.contains(&ticket.road) {
                    Some(sender)
                } else {
                    None
                }
            }) {
                sender.send(ticket)?;
            } else {
                pending_tickets.push(ticket);
            }
        }

        self.pending_tickets.append(&mut pending_tickets);

        Ok(())
    }
}

/// Run the main loop.
///
/// Listen for clients.
///
/// # Errors
/// * Error when socket returns an error.
#[tracing::instrument(skip(listener))]
pub async fn run(listener: TcpListener) -> Result<(), anyhow::Error> {
    let mut controller = Controller::default();
    let mut dispatchers = Dispatchers::default();

    let cameras = Arc::new(Mutex::new(HashMap::new()));

    let (controller_sender, mut controller_receiver) = mpsc::unbounded_channel();

    loop {
        tokio::select! {
            handler = listener.accept() => {
                let (socket, _) = handler?;

                tokio::spawn(handle_client(
                    socket,
                    controller_sender.clone(),
                    cameras.clone(),
                ));
            }

            message = controller_receiver.recv() => {
                match message {
                    Some(ControllerMessage::AddDispatcher(id, roads, ticket_sender)) => {
                        debug!("adding dispatcher {id} roads {roads:?}");
                        dispatchers.add_dispatcher(id, roads, ticket_sender)?;
                    }
                    Some(ControllerMessage::RemoveDispatcher(id)) => {
                        debug!("removing dispatcher {id}");
                        dispatchers.remove_dispatcher(id);
                    }
                    Some(ControllerMessage::Plate(plate)) => {
                        info!("handling plate: {plate:?}");
                        let tickets = controller.signal(plate);
                        debug!("tickets: {tickets:?}");
                        dispatchers.send_tickets(tickets)?;
                    }
                    None => {
                        warn!("empty controller message");
                    }
                }
            }
        }
    }
}

#[tracing::instrument(skip(socket, controller_sender, cameras))]
async fn handle_client(
    mut socket: TcpStream,
    controller_sender: mpsc::UnboundedSender<ControllerMessage>,
    cameras: Cameras,
) {
    let (read, write) = socket.split();
    let mut read = BufReader::new(read);
    let mut write = BufWriter::new(write);

    let handler = async {
        let mut heartbeat = Heartbeat::new(None);
        loop {
            tokio::select! {
                msg = read.read_u8() => {
                    match msg? {
                        wire::IAmCamera::TAG => {
                            return handle_camera(
                                cameras,
                                controller_sender,
                                wire::IAmCamera::read_payload_from(&mut read).await?,
                                heartbeat,
                                &mut read,
                                &mut write,
                            )
                                .await;
                        }
                        wire::IAmDispatcher::TAG => {
                            return handle_dispatcher(
                                controller_sender,
                                wire::IAmDispatcher::read_payload_from(&mut read).await?,
                                heartbeat,
                                &mut read,
                                &mut write,
                            )
                                .await;
                        }
                        wire::WantHeartbeat::TAG if !heartbeat.is_setted() => {
                            let wire::WantHeartbeat { interval: i } =
                                wire::WantHeartbeat::read_payload_from(&mut read).await?;
                            heartbeat.set_period(Duration::from_millis(u64::from(i * 100)));
                        }
                        msg => {
                            warn!("got invalid message: 0x{msg:02x}");
                            return Err(anyhow::anyhow!("invalid message: 0x{msg:02x}"));
                        }
                    }
                }

                _r = heartbeat.tick(), if heartbeat.is_valid() => {
                    info!("sending heartbeat");
                    wire::Heartbeat.write_to(&mut write).await?;
                    write.flush().await?;
                }
            }
        }
    };

    if let Err(err) = handler.await {
        wire::Error {
            msg: err.to_string(),
        }
        .write_to(&mut write)
        .await
        .ok();
        write.flush().await.ok();
        write.shutdown().await.ok();
    }
}

#[derive(Debug)]
struct CameraGuard(Cameras, u16);

impl CameraGuard {
    fn new(cameras: Cameras, road: u16, limit: u16) -> Result<Self, &'static str> {
        {
            let mut cameras = cameras.lock().unwrap();
            let (l, c) = cameras.entry(road).or_insert((limit, 0));
            if *l != limit {
                return Err("limit conflict");
            }
            *c += 1;
        }

        debug!("added camera road {road}");

        Ok(Self(cameras, road))
    }
}

impl Drop for CameraGuard {
    fn drop(&mut self) {
        if let Ok(mut cameras) = self.0.lock() {
            if let Some((_, c)) = cameras.get_mut(&self.1) {
                *c -= 1;
                if *c == 0 {
                    cameras.remove(&self.1);
                    debug!("removed camera road {}", self.1);
                }
            }
        }
    }
}

#[derive(Debug)]
struct DispatcherGuard(mpsc::UnboundedSender<ControllerMessage>, usize);

impl DispatcherGuard {
    fn new(
        controller_sender: mpsc::UnboundedSender<ControllerMessage>,
        roads: Vec<u16>,
        ticket_sender: mpsc::UnboundedSender<controller::Ticket>,
    ) -> Result<Self, anyhow::Error> {
        static IDS: atomic::AtomicUsize = atomic::AtomicUsize::new(0);

        let id = IDS.fetch_add(1, atomic::Ordering::Relaxed);

        controller_sender.send(ControllerMessage::AddDispatcher(
            id,
            roads.into_iter().collect(),
            ticket_sender,
        ))?;

        debug!("added dispatcher {id}");

        Ok(Self(controller_sender, id))
    }
}

impl Drop for DispatcherGuard {
    fn drop(&mut self) {
        if let Err(err) = self.0.send(ControllerMessage::RemoveDispatcher(self.1)) {
            warn!("cannot remove dispatcher: {err:?}");
        }
    }
}

#[derive(Default)]
struct Heartbeat {
    interval: Option<time::Interval>,
    period: Option<Duration>,
}

impl Heartbeat {
    fn new(period: Option<Duration>) -> Self {
        let mut result = Self::default();
        if let Some(period) = period {
            result.set_period(period);
        }
        result
    }

    fn set_period(&mut self, period: Duration) {
        self.period = Some(period);
        if period == Duration::from_millis(0) {
            self.interval = None;
        } else {
            self.interval = Some(time::interval_at(Instant::now() + period, period));
        }
    }

    async fn tick(&mut self) {
        if let Some(interval) = self.interval.as_mut() {
            interval.tick().await;
        } else {
            future::pending::<()>().await;
        }
    }

    fn is_valid(&self) -> bool {
        if let Some(interval) = self.interval.as_ref() {
            interval.period() != Duration::from_millis(0)
        } else {
            false
        }
    }

    fn is_setted(&self) -> bool {
        self.period.is_some()
    }
}

#[tracing::instrument(skip(cameras, controller_sender, heartbeat, read, write))]
async fn handle_camera<'a>(
    cameras: Cameras,
    controller_sender: mpsc::UnboundedSender<ControllerMessage>,
    i_am_camera: wire::IAmCamera,
    mut heartbeat: Heartbeat,
    read: &mut BufReader<ReadHalf<'a>>,
    write: &mut BufWriter<WriteHalf<'a>>,
) -> Result<(), anyhow::Error> {
    debug!("start {i_am_camera:?}");

    let _guard = CameraGuard::new(cameras, i_am_camera.road, i_am_camera.limit)
        .map_err(|e| anyhow::anyhow!("invalid camera: {e}"))?;

    loop {
        tokio::select! {
            msg = read.read_u8() => {
                match msg? {
                    wire::Plate::TAG => {
                        let wire::Plate { plate, timestamp } = wire::Plate::read_payload_from(read).await?;
                        info!("got plate {plate:?}");

                        controller_sender.send(ControllerMessage::Plate(controller::Plate {
                            plate,
                            road: i_am_camera.road,
                            limit: i_am_camera.limit,
                            mile: i_am_camera.mile,
                            timestamp,
                        }))?;
                    }
                    wire::WantHeartbeat::TAG if !heartbeat.is_setted() => {
                        let wire::WantHeartbeat { interval: i } = wire::WantHeartbeat::read_payload_from(read).await?;

                        info!("got want heartbeat {i}");

                        heartbeat.set_period(Duration::from_millis(u64::from(i) * 100));
                    }
                    msg => {
                        return Err(anyhow::anyhow!("invalid msg: 0x{msg:02x}"));
                    }
                }
            }

            _r = heartbeat.tick(), if heartbeat.is_valid() => {
                info!("sending heartbeat");
                wire::Heartbeat.write_to(write).await?;
                write.flush().await?;
            }
        }
    }
}

#[tracing::instrument(skip(controller_sender, heartbeat, read, write))]
async fn handle_dispatcher<'a>(
    controller_sender: mpsc::UnboundedSender<ControllerMessage>,
    i_am_dispatcher: wire::IAmDispatcher,
    mut heartbeat: Heartbeat,
    read: &mut BufReader<ReadHalf<'a>>,
    write: &mut BufWriter<WriteHalf<'a>>,
) -> Result<(), anyhow::Error> {
    debug!("start {i_am_dispatcher:?}");

    let (ticket_sender, mut ticket_receiver) = mpsc::unbounded_channel();

    let _guard = DispatcherGuard::new(controller_sender, i_am_dispatcher.roads, ticket_sender);

    loop {
        tokio::select! {
            msg = read.read_u8() => {
                match msg? {
                    wire::WantHeartbeat::TAG if !heartbeat.is_setted() => {
                        let wire::WantHeartbeat { interval: i } = wire::WantHeartbeat::read_payload_from(read).await?;

                        info!("got want heartbeat {i}");

                        heartbeat.set_period(Duration::from_millis(u64::from(i) * 100));
                    }
                    msg => {
                        return Err(anyhow::anyhow!("invalid msg: 0x{msg:02x}"));
                    }
                }
            }

            _r = heartbeat.tick(), if heartbeat.is_valid() => {
                info!("sending heartbeat");
                wire::Heartbeat.write_to(write).await?;
                write.flush().await?;
            }

            ticket = ticket_receiver.recv() => {
                if let Some(ticket) = ticket {
                    info!("got {ticket:?}");
                    ticket.write_to(write).await?;
                    write.flush().await?;
                } else {
                    warn!("got null ticket");
                    break Ok(());
                }
            }
        }
    }
}
