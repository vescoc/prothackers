use parking_lot::Once;

pub mod lrcp;

pub fn init_tracing_subscriber() {
    static TRACING_SUBSCRIBER_INIT: Once = Once::new();
    TRACING_SUBSCRIBER_INIT.call_once(tracing_subscriber::fmt::init);
}
