#![doc = include_str!("../README.md")]

pub mod codec;

#[cfg(test)]
mod tests {
    pub(crate) fn init_tracing_subscriber() {
        static INIT_TRACING_SUBSCRIBER: parking_lot::Once = parking_lot::Once::new();
        INIT_TRACING_SUBSCRIBER.call_once(tracing_subscriber::fmt::init);
    }
}
