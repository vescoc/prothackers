use std::future::Future;
use std::pin::pin;
use std::ptr;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

#[allow(warnings)]
mod bindings;

mod poller;
mod reactor;
pub mod sync;

pub use reactor::Reactor;

pub fn block_on<F, Fut>(f: F) -> Fut::Output
where
    F: FnOnce(Reactor) -> Fut,
    Fut: Future,
{
    let reactor = Reactor::new();

    let fut = (f)(reactor.clone());
    let mut fut = pin!(fut);

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(res) => return res,
            Poll::Pending => reactor.block_until(),
        }
    }
}

fn noop_waker() -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(|_| RAW, |_| {}, |_| {}, |_| {});
    const RAW: RawWaker = RawWaker::new(ptr::null(), &VTABLE);

    // Safety: all fields are no-ops, so this is safe
    unsafe { Waker::from_raw(RAW) }
}
