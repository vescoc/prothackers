use futures::{Sink, SinkExt, Stream, StreamExt};

use tokio::sync::mpsc;

use thiserror::Error;

use tracing::{debug, instrument, warn};

use crate::codec::{self, packets};

#[derive(Error, Debug)]
pub enum Error {
    #[error("invalid packet: {0}")]
    InvalidPacket(&'static str),

    #[error("packet receiving error: {0}")]
    Receiving(#[from] codec::Error),

    #[error("packet sending error: {0}")]
    Sending(codec::Error),

    #[error("controller error: {0}")]
    Controller(#[from] mpsc::error::SendError<packets::site_visit::Packet>),
}

pub struct SiteVisitor<U, D> {
    upstream: U,
    downstream: D,
    controller: mpsc::Sender<packets::site_visit::Packet>,
}

impl<U, D> SiteVisitor<U, D> {
    pub fn new(
        upstream: U,
        downstream: D,
        controller: mpsc::Sender<packets::site_visit::Packet>,
    ) -> Self {
        Self {
            upstream,
            downstream,
            controller,
        }
    }
}

impl<U, D, UE> SiteVisitor<U, D>
where
    U: Stream<Item = Result<packets::Packet, UE>> + Unpin,
    UE: Into<codec::Error>,
    D: Sink<packets::Packet> + Unpin,
    Error: From<D::Error>,
{
    #[instrument(skip_all)]
    pub async fn run(mut self) {
        debug!("run");

        if let Err(err) = self.handle_hello().await {
            warn!("got error on handle hello phase: {err}");
            self.downstream
                .send(packets::error::Packet::new(err.to_string()).into())
                .await
                .ok();
            return;
        }

        if let Err(err) = self.handle_visits().await {
            warn!("got error on handle visits phase: {err}");
            self.downstream
                .send(packets::error::Packet::new(err.to_string()).into())
                .await
                .ok();
        }
    }

    async fn handle_hello(&mut self) -> Result<(), Error> {
        match self.upstream.next().await {
            Some(Ok(packets::Packet::Hello(packets::hello::Packet { protocol, version })))
                if protocol == packets::hello::PESTCONTROL_PROTOCOL
                    && version == packets::hello::PESTCONTROL_VERSION =>
            {
                Ok(self
                    .downstream
                    .send(packets::hello::Packet::new().into())
                    .await?)
            }
            Some(Err(err)) => Err(Error::Receiving(err.into())),
            _ => Err(Error::InvalidPacket("waiting hello msg")),
        }
    }

    async fn handle_visits(&mut self) -> Result<(), Error> {
        loop {
            match self.upstream.next().await {
                Some(Ok(packets::Packet::SiteVisit(packet))) => {
                    self.controller.send(packet).await?;
                }
                Some(Ok(_)) => return Err(Error::InvalidPacket("got invalid packet")),
                Some(Err(err)) => return Err(Error::Receiving(err.into())),
                None => break Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::sink;
    use futures::stream;

    use tokio::time::timeout;

    use crate::tests::{init_tracing_subscriber, TIMEOUT};

    use super::*;

    #[tokio::test]
    async fn test_hello() {
        init_tracing_subscriber();

        let (client_tx, mut client_rx) = mpsc::unbounded_channel();

        let upstream = Box::pin(stream::once(async {
            Ok::<_, codec::Error>(packets::hello::Packet::new().into())
        }));

        let downstream = Box::pin(sink::unfold(client_tx, |client_tx, packet| async move {
            client_tx.send(packet).unwrap();
            Ok::<_, codec::Error>(client_tx)
        }));
        let (controller, _) = mpsc::channel(1);

        let site_visitor = SiteVisitor::new(upstream, downstream, controller);

        site_visitor.run().await;

        assert_eq!(
            timeout(TIMEOUT, client_rx.recv()).await.unwrap().unwrap(),
            packets::hello::Packet::new().into()
        );
    }

    #[tokio::test]
    async fn test_session() {
        init_tracing_subscriber();

        let (client_tx, mut client_rx) = mpsc::unbounded_channel();

        let upstream = Box::pin(stream::iter(vec![
            Ok::<_, codec::Error>(packets::hello::Packet::new().into()),
            Ok(packets::site_visit::Packet {
                site: 12354,
                populations: vec![packets::site_visit::Population {
                    species: "long-tailed-rat".to_string(),
                    count: 20,
                }],
            }
            .into()),
        ]));

        let downstream = Box::pin(sink::unfold(client_tx, |client_tx, packet| async move {
            client_tx.send(packet).unwrap();
            Ok::<_, codec::Error>(client_tx)
        }));
        let (controller, mut controller_tx) = mpsc::channel(1);

        let site_visitor = SiteVisitor::new(upstream, downstream, controller);

        site_visitor.run().await;

        assert_eq!(
            timeout(TIMEOUT, client_rx.recv()).await.unwrap().unwrap(),
            packets::hello::Packet::new().into()
        );

        assert_eq!(
            packets::site_visit::Packet {
                site: 12354,
                populations: vec![packets::site_visit::Population {
                    species: "long-tailed-rat".to_string(),
                    count: 20
                }]
            },
            timeout(TIMEOUT, controller_tx.recv())
                .await
                .unwrap()
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_invalid_packet() {
        init_tracing_subscriber();

        let (client_tx, mut client_rx) = mpsc::unbounded_channel();

        let upstream = Box::pin(stream::iter(vec![
            Ok::<_, codec::Error>(packets::hello::Packet::new().into()),
            Ok(packets::ok::Packet.into()),
        ]));

        let downstream = Box::pin(sink::unfold(client_tx, |client_tx, packet| async move {
            client_tx.send(packet).unwrap();
            Ok::<_, codec::Error>(client_tx)
        }));
        let (controller, _controller_tx) = mpsc::channel(1);

        let site_visitor = SiteVisitor::new(upstream, downstream, controller);

        site_visitor.run().await;

        assert_eq!(
            timeout(TIMEOUT, client_rx.recv()).await.unwrap().unwrap(),
            packets::hello::Packet::new().into(),
        );
        assert_eq!(
            timeout(TIMEOUT, client_rx.recv()).await.unwrap().unwrap(),
            packets::error::Packet::new("invalid packet: got invalid packet").into()
        );

        assert!(timeout(TIMEOUT, client_rx.recv()).await.unwrap().is_none());
    }
}
