use std::collections::HashSet;

use futures::channel::mpsc;

use thiserror::Error;

use tracing::{debug, instrument};

use crate::controller;

#[derive(Error, Debug)]
pub enum Error {
    #[error("send error {0}")]
    Send(#[from] mpsc::TrySendError<controller::Ticket>),
}

#[derive(Default)]
pub(crate) struct Dispatchers {
    dispatchers: Vec<(
        usize,
        HashSet<u16>,
        mpsc::UnboundedSender<controller::Ticket>,
    )>,
    pending_tickets: Vec<controller::Ticket>,
}

impl Dispatchers {
    #[instrument(skip(self))]
    pub(crate) fn add_dispatcher(
        &mut self,
        id: usize,
        roads: HashSet<u16>,
        ticket_sender: mpsc::UnboundedSender<controller::Ticket>,
    ) -> Result<(), Error> {
        debug!("add dispatcher");
        self.dispatchers.push((id, roads, ticket_sender));

        self.send_pending_tickets()
    }

    #[instrument(skip(self))]
    pub(crate) fn remove_dispatcher(&mut self, removed_id: usize) {
        debug!("remove dispatcher");
        self.dispatchers.retain(|(id, _, _)| *id != removed_id);
    }

    #[instrument(skip(self))]
    pub(crate) fn send_tickets(
        &mut self,
        mut tickets: Vec<controller::Ticket>,
    ) -> Result<(), Error> {
        debug!("send tickets");
        self.pending_tickets.append(&mut tickets);

        self.send_pending_tickets()
    }

    #[instrument(skip(self))]
    pub(crate) fn send_pending_tickets(&mut self) -> Result<(), Error> {
        debug!("send pending tickets");

        let mut pending_tickets = vec![];

        for ticket in self.pending_tickets.drain(..) {
            if let Some(sender) = self.dispatchers.iter().find_map(|(_, roads, sender)| {
                if roads.contains(&ticket.road) {
                    Some(sender)
                } else {
                    None
                }
            }) {
                sender.unbounded_send(ticket)?;
            } else {
                pending_tickets.push(ticket);
            }
        }

        debug!("pending tickets: {pending_tickets:?}");
        self.pending_tickets.append(&mut pending_tickets);

        Ok(())
    }
}
