use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use futures::{SinkExt, StreamExt};

use tokio::sync::mpsc;
use tokio::time::sleep;

use thiserror::Error;

use tracing::{debug, error, info, instrument, warn};

use crate::actors::Provider;
use crate::codec::{self, packets};

const INITIAL_BACKOFF: Duration = Duration::from_millis(100);
const MAX_RETRY: usize = 5;

#[derive(Error, Debug)]
pub enum Error {
    #[error("invalid packet: {0}")]
    InvalidPacket(&'static str),

    #[error("packet error: {0}")]
    Packet(#[from] codec::Error),

    #[error("cannot connect to authority server")]
    CannotConnect,
}

pub struct AuthorityServer<P> {
    site: u32,
    authority_server_provider: P,
    controller: mpsc::UnboundedReceiver<Vec<packets::site_visit::Population>>,
    policies: HashMap<String, (u32, packets::create_policy::PolicyAction)>,
}

impl<P> AuthorityServer<P> {
    pub fn new(
        site: u32,
        authority_server_provider: P,
        controller: mpsc::UnboundedReceiver<Vec<packets::site_visit::Population>>,
    ) -> Self {
        Self {
            site,
            authority_server_provider,
            controller,
            policies: HashMap::new(),
        }
    }
}

impl<P: Provider + Send + 'static> AuthorityServer<P> {
    #[instrument]
    pub async fn run(mut self) {
        debug!("run");

        let Some(mut populations) = self.controller.recv().await else {
            info!("controller disconnected");
            return;
        };

        debug!("got first populations: {populations:?}");

        while let Ok((mut upstream, mut downstream, target_populations)) = self.connect().await {
            loop {
                if let Err(err) = self
                    .adjust_policies(
                        &mut upstream,
                        &mut downstream,
                        &target_populations,
                        &populations,
                    )
                    .await
                {
                    warn!("error: {err}");
                    break;
                }

                populations = if let Some(populations) = self.controller.recv().await {
                    populations
                } else {
                    info!("controller closed");
                    return;
                };
                debug!("got next populations: {populations:?}");
            }
        }

        warn!("cannot connect to authority server");
    }

    #[instrument]
    #[allow(clippy::type_complexity)]
    async fn connect(
        &mut self,
    ) -> Result<
        (
            P::Sink,
            P::Stream,
            Vec<packets::target_populations::Population>,
        ),
        Error,
    > {
        let mut delay = INITIAL_BACKOFF;
        let mut retry = 0;
        loop {
            debug!("connection {retry} try");

            if let Ok(r) = self.try_connect().await {
                return Ok(r);
            }

            retry += 1;
            if retry > MAX_RETRY {
                error!("max retry");
                return Err(Error::CannotConnect);
            }

            warn!("backoff {delay:?}");

            sleep(delay).await;

            delay *= 2;
        }
    }

    #[instrument]
    #[allow(clippy::type_complexity)]
    async fn try_connect(
        &mut self,
    ) -> Result<
        (
            P::Sink,
            P::Stream,
            Vec<packets::target_populations::Population>,
        ),
        Error,
    > {
        let (mut upstream, mut downstream) = self
            .authority_server_provider
            .connect(self.site)
            .await
            .map_err(|_| Error::CannotConnect)?;

        debug!("got connection");

        upstream
            .send(packets::hello::Packet::new().into())
            .await
            .map_err(|_| Error::CannotConnect)?;

        debug!("sent hello packet");

        match downstream.next().await {
            Some(Ok(packets::Packet::Hello(packets::hello::Packet { protocol, version })))
                if protocol == packets::hello::PESTCONTROL_PROTOCOL
                    && version == packets::hello::PESTCONTROL_VERSION =>
            {
                debug!("got hello packet");

                upstream
                    .send(packets::dial_authority::Packet::new(self.site).into())
                    .await
                    .map_err(|_| Error::CannotConnect)?;

                match downstream.next().await {
                    Some(Ok(packets::Packet::TargetPopulations(
                        packets::target_populations::Packet { site, populations },
                    ))) if site == self.site => {
                        debug!("got target populations: {populations:?}");
                        return Ok((upstream, downstream, populations));
                    }

                    r => {
                        warn!("invalid packet waiting target populations: {r:?}");
                    }
                }
            }

            r => {
                warn!("invalid packet waiting hello: {r:?}");
            }
        }

        Err(Error::CannotConnect)
    }

    #[instrument(skip(upstream, downstream, target_populations, populations))]
    async fn adjust_policies(
        &mut self,
        upstream: &mut P::Sink,
        downstream: &mut P::Stream,
        target_populations: &[packets::target_populations::Population],
        populations: &[packets::site_visit::Population],
    ) -> Result<(), codec::Error> {
        for packets::site_visit::Population { species, count, .. } in populations {
            if let Some(packets::target_populations::Population { min, max, .. }) =
                target_populations.iter().find(
                    |packets::target_populations::Population {
                         species: target_species,
                         ..
                     }| target_species == species,
                )
            {
                debug!("found species: {species} {min} [ {count} ] {max}");
                if (*min..=*max).contains(count) {
                    self.delete_policy(upstream, downstream, species).await?;
                } else if count < min {
                    self.add_policy(
                        upstream,
                        downstream,
                        species,
                        packets::create_policy::PolicyAction::Conserve,
                    )
                    .await?;
                } else if count > max {
                    self.add_policy(
                        upstream,
                        downstream,
                        species,
                        packets::create_policy::PolicyAction::Cull,
                    )
                    .await?;
                }
            }
        }

        for packets::target_populations::Population {
            species, min, max, ..
        } in target_populations
        {
            if !populations.iter().any(
                |packets::site_visit::Population {
                     species: target_species,
                     ..
                 }| target_species == species,
            ) {
                debug!("NOT found species: {species} {min} [ 0 ] {max}");
                if (*min..=*max).contains(&0) {
                    self.delete_policy(upstream, downstream, species).await?;
                } else if *min > 0 {
                    self.add_policy(
                        upstream,
                        downstream,
                        species,
                        packets::create_policy::PolicyAction::Conserve,
                    )
                    .await?;
                }
            }
        }

        Ok(())
    }

    #[instrument(skip(upstream, downstream))]
    async fn delete_policy(
        &mut self,
        upstream: &mut P::Sink,
        downstream: &mut P::Stream,
        species: &str,
    ) -> Result<(), codec::Error> {
        debug!("delete policy");
        if let Some((id, _)) = self.policies.remove(species) {
            upstream
                .send(packets::delete_policy::Packet::new(id).into())
                .await
                .map_err(|_| codec::Error::InvalidPacket)?;
            if let Some(Ok(packets::Packet::Ok(packets::ok::Packet))) = downstream.next().await {
                debug!("deleted policy {id}");
            } else {
                return Err(codec::Error::InvalidPacket);
            }
        } else {
            debug!("nothing to do");
        }

        Ok(())
    }

    #[instrument(skip(upstream, downstream))]
    async fn add_policy(
        &mut self,
        upstream: &mut P::Sink,
        downstream: &mut P::Stream,
        species: &str,
        action: packets::create_policy::PolicyAction,
    ) -> Result<(), codec::Error> {
        debug!("add policy");
        match self.policies.get(species) {
            Some((id, current_action)) if action == *current_action => {
                debug!("policy already defined: ({id}, {action:?})");
            }
            _ => {
                self.delete_policy(upstream, downstream, species).await?;

                upstream
                    .send(packets::create_policy::Packet::new(species, action).into())
                    .await
                    .map_err(|_| codec::Error::InvalidPacket)?;

                if let Some(Ok(packets::Packet::PolicyResult(packets::policy_result::Packet {
                    policy,
                }))) = downstream.next().await
                {
                    self.policies.insert(species.to_string(), (policy, action));
                    debug!("added policy ({policy}, {action:?})");
                } else {
                    return Err(codec::Error::InvalidPacket);
                }
            }
        }

        Ok(())
    }
}

impl<P> fmt::Debug for AuthorityServer<P> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(fmt, "AuthorityServer[{}]", self.site)
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::timeout;

    use crate::actors::tests::TestProvider;
    use crate::tests::{init_tracing_subscriber, TIMEOUT};

    use super::*;

    #[tokio::test]
    async fn test_session() {
        init_tracing_subscriber();

        let (mut endpoints, provider) = TestProvider::new();

        let (controller_tx, controller) = tokio::sync::mpsc::unbounded_channel();

        let authority_server = AuthorityServer::new(12345, provider, controller);

        let handler = tokio::spawn(authority_server.run());

        controller_tx
            .send(vec![packets::site_visit::Population::new(
                "long-tailed rat",
                20,
            )])
            .unwrap();

        let Some((12345, mut upstream, mut downstream)) = endpoints.recv().await else {
            panic!("cannot get endpoints");
        };

        assert_eq!(
            timeout(TIMEOUT, upstream.next()).await.unwrap().unwrap(),
            packets::hello::Packet::new().into(),
        );

        downstream
            .send(Ok(packets::Packet::Hello(packets::hello::Packet::new())))
            .await
            .unwrap();

        assert_eq!(
            timeout(TIMEOUT, upstream.next()).await.unwrap().unwrap(),
            packets::dial_authority::Packet::new(12345).into(),
        );

        downstream
            .send(Ok(packets::target_populations::Packet::new(
                12345,
                vec![packets::target_populations::Population {
                    species: "long-tailed rat".to_string(),
                    min: 0,
                    max: 10,
                }],
            )
            .into()))
            .await
            .unwrap();

        assert_eq!(
            timeout(TIMEOUT, upstream.next()).await.unwrap().unwrap(),
            packets::create_policy::Packet::new(
                "long-tailed rat".to_string(),
                packets::create_policy::PolicyAction::Cull
            )
            .into(),
        );

        downstream
            .send(Ok(packets::policy_result::Packet::new(123).into()))
            .await
            .unwrap();

        controller_tx
            .send(vec![packets::site_visit::Population {
                species: "long-tailed rat".to_string(),
                count: 20,
            }])
            .unwrap();

        drop(controller_tx);

        timeout(TIMEOUT, handler).await.unwrap().unwrap();
    }
}
