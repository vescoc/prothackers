use slab::Slab;

use wasi::io::poll::poll;

use crate::reactor::Pollable;

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
            match target {
                Pollable::Wasi(pollable) => {
                    indexes.push(index);
                    targets.push(pollable);
                }
            }
        }

        let ready_indexes = if targets.is_empty() {
            vec![]
        } else {
            poll(&targets)
        };

        ready_indexes
            .into_iter()
            .map(|index| EventKey(indexes[index as usize] as u32))
            .collect()
    }
}
