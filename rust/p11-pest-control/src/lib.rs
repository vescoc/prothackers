#![doc = include_str!("../README.md")]

use std::io;

use tokio::io::{BufReader, BufWriter};
use tokio::net::{
    tcp::{OwnedReadHalf, OwnedWriteHalf},
    TcpListener, TcpStream,
};
use tokio::sync::mpsc;

use tokio_util::codec::{FramedRead, FramedWrite};

use tracing::{info, instrument};

pub mod actors;
pub mod codec;

use actors::controller::Controller;
use actors::site_visitor::SiteVisitor;
use actors::Provider;
use codec::packets::PacketCodec;

#[derive(Clone, Debug)]
pub struct DefaultProvider {
    address: String,
    port: u16,
}

impl DefaultProvider {
    #[must_use]
    pub fn new(address: String, port: u16) -> Self {
        Self { address, port }
    }
}

impl Provider for DefaultProvider {
    type Sink = FramedWrite<BufWriter<OwnedWriteHalf>, PacketCodec>;
    type Stream = FramedRead<BufReader<OwnedReadHalf>, PacketCodec>;

    type Error = io::Error;

    #[instrument]
    async fn connect(&mut self, _: u32) -> Result<(Self::Sink, Self::Stream), Self::Error> {
        let socket = TcpStream::connect(&format!("{}:{}", self.address, self.port)).await?;

        info!("new connection to authority server");

        let (read, write) = socket.into_split();
        let reader = FramedRead::new(BufReader::new(read), PacketCodec::new());
        let writer = FramedWrite::new(BufWriter::new(write), PacketCodec::new());

        Ok((writer, reader))
    }
}

#[instrument(skip_all)]
pub async fn run<P: Provider + Clone + Send + 'static>(
    listener: TcpListener,
    authority_server_provider: P,
) -> Result<(), io::Error> {
    let (site_visits, site_visits_rx) = mpsc::channel(1000);

    let controller = Controller::new(authority_server_provider, site_visits_rx);
    tokio::spawn(controller.run());

    loop {
        let (socket, remote_addr) = listener.accept().await?;

        info!("remote: {remote_addr:?}");

        let (read, write) = socket.into_split();
        let reader = FramedRead::new(BufReader::new(read), PacketCodec::new());
        let writer = FramedWrite::new(BufWriter::new(write), PacketCodec::new());

        let site_visitor = SiteVisitor::new(reader, writer, site_visits.clone());
        tokio::spawn(site_visitor.run());
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    pub(crate) const TIMEOUT: Duration = Duration::from_millis(100);

    pub(crate) fn init_tracing_subscriber() {
        static INIT_TRACING_SUBSCRIBER: parking_lot::Once = parking_lot::Once::new();
        INIT_TRACING_SUBSCRIBER.call_once(tracing_subscriber::fmt::init);
    }
}
