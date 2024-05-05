use std::collections::HashMap;
use std::fmt;

use tokio::sync::mpsc;

use tracing::{debug, info, instrument, warn};

use crate::actors::authority_server::AuthorityServer;
use crate::actors::Provider;
use crate::codec::packets;

pub struct Controller<P> {
    authority_server_provider: P,
    site_visits: mpsc::Receiver<packets::site_visit::Packet>,
    authority_servers: HashMap<u32, mpsc::UnboundedSender<Vec<packets::site_visit::Population>>>,
}

impl<P> fmt::Debug for Controller<P> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "Controller")
    }
}

impl<P> Controller<P> {
    pub fn new(
        authority_server_provider: P,
        site_visits: mpsc::Receiver<packets::site_visit::Packet>,
    ) -> Self {
        Self {
            authority_server_provider,
            site_visits,
            authority_servers: HashMap::new(),
        }
    }
}

impl<P: Provider + Clone + Send + 'static> Controller<P> {
    #[instrument]
    pub async fn run(mut self) {
        info!("run start");

        while let Some(packets::site_visit::Packet { site, populations }) =
            self.site_visits.recv().await
        {
            debug!("got populations: {populations:?}");

            let authority_server = self
                .authority_servers
                .entry(site)
                .or_insert_with_key(|site| {
                    let authority_server_provider = self.authority_server_provider.clone();
                    let (authority_server_tx, authority_server_rx) = mpsc::unbounded_channel();

                    let authority_server =
                        AuthorityServer::new(*site, authority_server_provider, authority_server_rx);
                    tokio::spawn(async move { authority_server.run().await });

                    authority_server_tx
                });

            if let Err(err) = authority_server.send(populations) {
                warn!("cannot send to site {site}: {err}");
                self.authority_servers.remove(&site);
            }
        }

        info!("run done");
    }
}

#[cfg(test)]
mod tests {
    use futures::{SinkExt, StreamExt};

    use tokio::time::timeout;

    use crate::actors::tests::TestProvider;
    use crate::tests::{init_tracing_subscriber, TIMEOUT};

    use super::*;

    #[tokio::test]
    #[instrument]
    async fn test_session() {
        init_tracing_subscriber();

        let (mut endpoints, provider) = TestProvider::new();

        let (site_visits, site_visits_rx) = mpsc::channel(1);

        let controller = Controller::new(provider, site_visits_rx);

        tokio::spawn(controller.run());

        site_visits
            .send(packets::site_visit::Packet::new(
                12345,
                vec![packets::site_visit::Population::new("dog", 10)],
            ))
            .await
            .unwrap();

        let Some((12345, mut upstream, mut downstream)) = endpoints.recv().await else {
            panic!("invalid endpoints")
        };

        assert_eq!(
            timeout(TIMEOUT, upstream.next()).await.unwrap().unwrap(),
            packets::hello::Packet::new().into(),
        );

        downstream
            .send(Ok(packets::hello::Packet::new().into()))
            .await
            .unwrap();

        assert_eq!(
            packets::Packet::DialAuthority(packets::dial_authority::Packet::new(12345)),
            timeout(TIMEOUT, upstream.next()).await.unwrap().unwrap()
        );
    }
}
