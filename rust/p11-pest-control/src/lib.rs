#![doc = include_str!("../README.md")]

pub mod actors;
pub mod codec;

#[cfg(test)]
mod tests {
    use std::time::Duration;

    pub(crate) const TIMEOUT: Duration = Duration::from_millis(100);

    pub(crate) fn init_tracing_subscriber() {
        static INIT_TRACING_SUBSCRIBER: parking_lot::Once = parking_lot::Once::new();
        INIT_TRACING_SUBSCRIBER.call_once(tracing_subscriber::fmt::init);
    }
}
