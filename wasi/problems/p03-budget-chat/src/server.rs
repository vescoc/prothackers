use std::collections::HashMap;
use std::sync::Arc;

use futures::{channel::mpsc, StreamExt};

use tracing::{debug, instrument, trace, warn};

use wasi_async::net::TcpListener;

use crate::{handle_client, ClientMessage, Error, Id, ServerMessage};

struct ClientInfo {
    id: Id,
    sender: mpsc::UnboundedSender<ServerMessage>,
    username: Option<Arc<String>>,
}

impl ClientInfo {
    fn is_joined(&self) -> bool {
        self.username.is_some()
    }
}

fn is_username_valid(clients: &HashMap<Id, ClientInfo>, username: &str) -> bool {
    !username.is_empty()
        && username.chars().all(char::is_alphanumeric)
        && clients.values().all(
            |ClientInfo {
                 username: other_username,
                 ..
             }| {
                other_username
                    .as_ref()
                    .map_or(true, |other_username| **other_username != username)
            },
        )
}

/// Run the main loop.
///
/// Listen for clients and run the chat.
///
/// # Errors
/// * Error when socket returns an error.
///
/// # Panics
/// * None
#[instrument(skip_all)]
pub async fn run(reactor: wasi_async_runtime::Reactor, listener: TcpListener) -> Result<(), Error> {
    let mut id = 0;
    let mut clients = HashMap::new();
    let (server_sender, mut receiver) = mpsc::unbounded();

    let mut incoming_clients = listener.into_stream().fuse();
    loop {
        trace!("main loop");
        futures::select_biased! {
            client_message = receiver.next() => {
                trace!("client message: {client_message:?}");

                // Panic: nobody closes this channel, unwrap ok
                match client_message.expect("invalid state") {
                    ClientMessage::SetUsername(id, username) => {
                        if is_username_valid(&clients, &username) {
                            let client = clients.get_mut(&id).unwrap();

                            assert!(client.username.is_none());

                            client.username = Some(Arc::new(username));
                            client.sender.unbounded_send(ServerMessage::UsernameAccepted).ok();

                            let mut users = vec![];
                            for ClientInfo { username, .. } in clients.values().filter(|c| c.is_joined() && c.id != id) {
                                // username must be set
                                users.push(username.as_ref().unwrap().clone());
                            }
                            clients.get(&id).unwrap().sender.unbounded_send(ServerMessage::Users(users)).ok();
                        } else {
                            clients.get(&id).unwrap().sender.unbounded_send(ServerMessage::UsernameInvalid).ok();
                        }
                    }

                    ClientMessage::Joined(id) => {
                        if let Some(user) = clients[&id].username.clone() {
                            for ClientInfo { sender, .. } in clients.values().filter(|c| c.is_joined() && c.id != id) {
                                sender.unbounded_send(ServerMessage::AnnounceUser(user.clone())).ok();
                            }
                        } else {
                            warn!("joined from invalid id {id}");
                        }
                    }

                    ClientMessage::Message(id, message) => {
                        if let Some(user) = clients[&id].username.clone() {
                            for ClientInfo { sender, .. } in clients.values().filter(|c| c.is_joined() && c.id != id) {
                                sender.unbounded_send(ServerMessage::Message(user.clone(), message.clone())).ok();
                            }
                        } else {
                            warn!("message from invalid id {id}");
                        }
                    }

                    ClientMessage::Disconnect(id) => {
                        if let Some(client_info) = clients.get(&id) {
                            if let Some(user) = client_info.username.clone() {
                                for ClientInfo { sender, .. } in clients.values().filter(|c| c.is_joined() && c.id != id) {
                                    sender.unbounded_send(ServerMessage::Disconnected(user.clone())).ok();
                                }
                            }
                            clients.remove(&id);
                        }
                    }
                }
            }

            client = incoming_clients.next() => {
                let Some(Ok((socket, remote_address))) = client else { break Ok(()) };

                debug!("new client: {remote_address:?}");

                id += 1;

                let (sender, receiver) = mpsc::unbounded();

                reactor.clone().spawn(handle_client(id, socket, server_sender.clone(), receiver));

                sender.unbounded_send(ServerMessage::Welcome).ok();

                clients.insert(id,
                               ClientInfo {
                                   id,
                                   sender,
                                   username: None,
                               });
            }
        }
    }
}
