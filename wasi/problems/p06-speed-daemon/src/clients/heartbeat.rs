use std::future;
use std::time::Duration;

use wasi_async::time::{self, Instant};
use wasi_async_runtime::Reactor;

pub(crate) struct Heartbeat {
    reactor: Reactor,
    interval: Option<time::Interval>,
    period: Option<Duration>,
}

impl Heartbeat {
    pub(crate) fn new(reactor: Reactor) -> Self {
        Self {
            reactor,
            period: None,
            interval: None,
        }
    }

    pub(crate) fn set_period(&mut self, period: Duration) {
        self.period = Some(period);
        if period == Duration::from_millis(0) {
            self.interval = None;
        } else {
            self.interval = Some(time::interval_at(
                self.reactor.clone(),
                Instant::now() + period,
                period,
            ));
        }
    }

    pub(crate) async fn tick(&mut self) {
        if let Some(interval) = self.interval.as_mut() {
            interval.tick().await;
        } else {
            future::pending::<()>().await;
        }
    }

    pub(crate) fn is_setted(&self) -> bool {
        self.period.is_some()
    }
}
