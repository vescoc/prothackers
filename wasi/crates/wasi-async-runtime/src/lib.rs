use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll};

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
    let (reactor, waker) = Reactor::new();

    let fut = (f)(reactor.clone());
    let mut fut = pin!(fut);

    let mut cx = Context::from_waker(&waker);

    loop {
        reactor.reset_main_task_state();
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(res) => return res,
            Poll::Pending => reactor.block_until(),
        }
    }
}
