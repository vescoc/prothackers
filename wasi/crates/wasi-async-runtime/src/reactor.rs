use std::cell::RefCell;
use std::future::{self, Future};
use std::mem;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use hashbrown::{HashMap, HashSet};

use wasi::io::poll::Pollable as WasiPollable;

use crate::noop_waker;
use crate::poller::{EventKey, Poller};

#[derive(Debug)]
pub enum Pollable {
    Wasi(WasiPollable),
}

impl Pollable {
    pub fn ready(&self) -> bool {
        match self {
            Pollable::Wasi(pollable) => pollable.ready(),
        }
    }
}

impl From<WasiPollable> for Pollable {
    fn from(pollable: WasiPollable) -> Self {
        Self::Wasi(pollable)
    }
}

#[derive(Clone)]
pub struct Reactor {
    inner: Rc<RefCell<InnerReactor>>,
}

type TaskInfo = (usize, Pin<Box<dyn Future<Output = ()> + 'static>>);

struct InnerReactor {
    next_id: usize,
    poller: Poller,
    wakers: HashMap<EventKey, Waker>,
    tasks: Vec<TaskInfo>,
    complete: HashMap<usize, Option<Waker>>,
}

impl Reactor {
    pub(crate) fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(InnerReactor {
                next_id: 0,
                poller: Poller::new(),
                wakers: HashMap::new(),
                tasks: Vec::new(),
                complete: HashMap::new(),
            })),
        }
    }

    pub async fn wait_for<P: Into<Pollable>>(&self, pollable: P) {
        let mut pollable = Some(pollable.into());
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
        let mut complete = HashSet::new();
        while let Some((task_id, mut task)) = tasks.pop() {
            if task.as_mut().poll(&mut cx).is_pending() {
                pending.push((task_id, task));
            } else {
                complete.insert(task_id);
            }
        }

        let mut reactor = self.inner.borrow_mut();
        reactor.tasks.append(&mut pending);
        for task_id in complete {
            if let Some(waker) = reactor.complete.get_mut(&task_id) {
                if let Some(waker) = waker.take() {
                    waker.wake_by_ref();
                }
            } else {
                reactor.complete.insert(task_id, None);
            }
        }

        for key in reactor.poller.block_until() {
            match reactor.wakers.get(&key) {
                Some(waker) => waker.wake_by_ref(),
                None => panic!("tried to wake the waker for non-existent `{key:?}`"),
            }
        }
    }

    pub fn spawn(&self, f: impl Future<Output = ()> + 'static) -> JoinHandle {
        let mut reactor = self.inner.borrow_mut();

        let task_id = reactor.next_id;

        reactor.next_id += 1;

        reactor.tasks.push((task_id, Box::pin(f)));

        JoinHandle {
            reactor: self.clone(),
            task_id,
        }
    }
}

pub struct JoinHandle {
    reactor: Reactor,
    task_id: usize,
}

impl Future for JoinHandle {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut reactor = this.reactor.inner.borrow_mut();
        if reactor.complete.contains_key(&this.task_id) {
            Poll::Ready(())
        } else {
            reactor
                .complete
                .entry(this.task_id)
                .or_insert_with(|| Some(cx.waker().clone()));
            Poll::Pending
        }
    }
}
