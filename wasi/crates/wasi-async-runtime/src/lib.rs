use std::cell::RefCell;
use std::future::{self, Future};
use std::mem;
use std::pin::{pin, Pin};
use std::ptr;
use std::rc::Rc;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use futures::FutureExt;

use hashbrown::HashMap;

use slab::Slab;

use wasi::io::poll::{poll, Pollable};

#[allow(warnings)]
mod bindings;

#[derive(Debug)]
pub(crate) struct Poller {
    pub(crate) targets: Slab<Pollable>,
}

#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub(crate) struct EventKey(pub(crate) u32);

impl Poller {
    pub(crate) fn new() -> Self {
        Self {
            targets: Slab::new(),
        }
    }

    pub(crate) fn insert(&mut self, target: Pollable) -> EventKey {
        let key = self.targets.insert(target);
        EventKey(key as u32)
    }

    pub(crate) fn get(&self, key: &EventKey) -> Option<&Pollable> {
        self.targets.get(key.0 as usize)
    }

    pub(crate) fn remove(&mut self, key: EventKey) -> Option<Pollable> {
        self.targets.try_remove(key.0 as usize)
    }

    pub(crate) fn block_until(&mut self) -> Vec<EventKey> {
        let mut indexes = Vec::with_capacity(self.targets.len());
        let mut targets = Vec::with_capacity(self.targets.len());

        for (index, target) in self.targets.iter() {
            indexes.push(index);
            targets.push(target);
        }

        let ready_indexes = poll(&targets);

        ready_indexes
            .into_iter()
            .map(|index| EventKey(indexes[index as usize] as u32))
            .collect()
    }
}

#[derive(Clone)]
pub struct Reactor {
    inner: Rc<RefCell<InnerReactor>>,
}

struct InnerReactor {
    poller: Poller,
    wakers: HashMap<EventKey, Waker>,
    tasks: Vec<Pin<Box<dyn Future<Output = ()> + 'static>>>,
}

impl Reactor {
    pub(crate) fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(InnerReactor {
                poller: Poller::new(),
                wakers: HashMap::new(),
                tasks: Vec::new(),
            })),
        }
    }

    pub async fn wait_for(&self, pollable: Pollable) {
        let mut pollable = Some(pollable);
        let mut key = None;

        future::poll_fn(|cx| {
            let mut reactor = self.inner.borrow_mut();

            let key = key.get_or_insert_with(|| reactor.poller.insert(pollable.take().unwrap()));
            reactor.wakers.insert(*key, cx.waker().clone());

            if reactor.poller.get(key).unwrap().ready() {
                reactor.poller.remove(*key);
                reactor.wakers.remove(key);
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
    }

    pub(crate) fn block_until(&self) {
        let mut tasks = {
            let mut reactor = self.inner.borrow_mut();

            mem::take(&mut reactor.tasks)
        };

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        let mut pending = vec![];
        while let Some(mut task) = tasks.pop() {
            if task.as_mut().poll(&mut cx).is_pending() {
                pending.push(task);
            }
        }

        let mut reactor = self.inner.borrow_mut();
        reactor.tasks.append(&mut pending);

        for key in reactor.poller.block_until() {
            match reactor.wakers.get(&key) {
                Some(waker) => waker.wake_by_ref(),
                None => panic!("tried to wake the waker for non-existent `{key:?}`"),
            }
        }
    }

    pub fn spawn(&self, f: impl Future<Output = ()> + 'static) {
        let mut reactor = self.inner.borrow_mut();
        reactor.tasks.push(f.boxed_local());
    }
}

fn noop_waker() -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(|_| RAW, |_| {}, |_| {}, |_| {});
    const RAW: RawWaker = RawWaker::new(ptr::null(), &VTABLE);

    // Safety: all fields are no-ops, so this is safe
    unsafe { Waker::from_raw(RAW) }
}

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
