use std::fmt;
use std::future::Future;
use std::time::Duration;

use futures::FutureExt;
use futures_concurrency::prelude::*;

use wasi::clocks::monotonic_clock;

use wasi_async_runtime::Reactor;

#[derive(Debug, PartialEq)]
pub struct Elapsed;

impl std::error::Error for Elapsed {}

impl fmt::Display for Elapsed {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(fmt, "Elapsed")
    }
}

pub async fn timeout<F: Future>(
    reactor: Reactor,
    duration: Duration,
    future: F,
) -> Result<F::Output, Elapsed> {
    let wait_for = reactor
        .wait_for(monotonic_clock::subscribe_duration(
            duration.as_nanos() as u64
        ))
        .map(|_| Err(Elapsed));

    let future = future.map(Ok);

    (wait_for, future).race().await
}
