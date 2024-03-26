use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use tracing::{debug, error, info, warn};

type ID = usize;
type Username = String;
type Message = String;

enum ClientMessage {
    SetUsername(ID, Username),
    Joined(ID),
    Message(ID, Arc<Message>),
    Disconnect(ID),
}

#[derive(Debug)]
enum ServerMessage {
    Welcome,
    UsernameAccepted,
    UsernameInvalid,
    AnnounceUser(Arc<Username>),
    Users(Vec<Arc<Username>>),
    Message(Arc<Username>, Arc<Message>),
    Disconnected(Arc<Username>),
}

struct ClientInfo {
    id: ID,
    sender: UnboundedSender<ServerMessage>,
    username: Option<Arc<String>>,
}

impl ClientInfo {
    fn is_joined(&self) -> bool {
        self.username.is_some()
    }
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
#[tracing::instrument(skip(listener))]
pub async fn run(listener: TcpListener) -> Result<(), anyhow::Error> {
    let mut id = 0;
    let mut clients = HashMap::new();
    let (server_sender, mut receiver) = unbounded_channel();
    loop {
        debug!("main loop");
        tokio::select! {
            biased;

            socket_addr = listener.accept() => {
                debug!("new client");

                let (socket, _) = socket_addr?;

                id += 1;

                let (sender, receiver) = unbounded_channel();

                tokio::spawn(handle_client(id, socket, server_sender.clone(), receiver));

                sender.send(ServerMessage::Welcome)?;

                clients.insert(id,
                               ClientInfo {
                                   id,
                                   sender,
                                   username: None,
                               });
            }

            client_message = receiver.recv() => {
                debug!("client message");

                // nobody closes this channel
                match client_message.expect("invalid state") {
                    ClientMessage::SetUsername(id, username) => {
                        if is_username_valid(&clients, &username) {
                            let client = clients.get_mut(&id).unwrap();

                            assert!(client.username.is_none());

                            client.username = Some(Arc::new(username));
                            client.sender.send(ServerMessage::UsernameAccepted)?;
                        } else {
                            clients[&id].sender.send(ServerMessage::UsernameInvalid)?;
                        }
                    }
                    ClientMessage::Joined(id) => {
                        if let Some(user) = &clients[&id].username {
                            let mut users = vec![];
                            for ClientInfo { username, sender, .. } in clients.values().filter(|c| c.is_joined() && c.id != id) {
                                // username must be set
                                users.push(username.as_ref().unwrap().clone());
                                sender.send(ServerMessage::AnnounceUser(user.clone()))?;
                            }
                            clients[&id].sender.send(ServerMessage::Users(users))?;
                        } else {
                            warn!("joined from invalid id {id}");
                        }
                    }
                    ClientMessage::Message(id, message) => {
                        if let Some(user) = &clients[&id].username {
                            for ClientInfo { sender, .. } in clients.values().filter(|c| c.is_joined() && c.id != id) {
                                sender.send(ServerMessage::Message(user.clone(), message.clone()))?;
                            }
                        } else {
                            warn!("message from invalid id {id}");
                        }
                    }
                    ClientMessage::Disconnect(id) => {
                        if let Some(client_info) = clients.get(&id) {
                            if let Some(user) = &client_info.username {
                                for ClientInfo { sender, .. } in clients.values().filter(|c| c.is_joined() && c.id != id) {
                                    sender.send(ServerMessage::Disconnected(user.clone()))?;
                                }
                            }
                            clients.remove(&id);
                        }
                    }
                }
            }
        }
    }
}

fn is_username_valid(clients: &HashMap<ID, ClientInfo>, username: &str) -> bool {
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

/// Client iterations.
///
/// Listen for a client and run the chat.
///
/// # Errors:
/// * Error when socket returns an error.
#[allow(clippy::never_loop)]
#[tracing::instrument(skip(stream, server, client))]
async fn handle_client(
    id: ID,
    mut stream: TcpStream,
    server: UnboundedSender<ClientMessage>,
    mut client: UnboundedReceiver<ServerMessage>,
) {
    info!("start {id}");

    let (read, mut write) = stream.split();
    let mut segments = BufReader::new(read).split(b'\n');

    loop {
        debug!("state: waiting welcome message");
        match client.recv().await {
            Some(ServerMessage::Welcome) => {
                debug!("got welcome message");
                write
                    .write_all(b"Welcome to budgetchat! What shall I call you?\n")
                    .await
                    .expect("cannot write to client");
                write.flush().await.expect("cannot flush");
            }
            None => {
                info!("got empty message");
                server.send(ClientMessage::Disconnect(id)).ok();
                break;
            }
            message => {
                error!("got invalid message from server, close connection: {message:?}");
                server.send(ClientMessage::Disconnect(id)).ok();
                break;
            }
        }

        debug!("state: joining");
        if let Ok(Some(username)) = segments.next_segment().await {
            debug!("got username");
            server
                .send(ClientMessage::SetUsername(
                    id,
                    String::from_utf8_lossy(&username).into_owned(),
                ))
                .ok();
        } else {
            warn!("got invalid message from client, close connection");
            server.send(ClientMessage::Disconnect(id)).ok();
            break;
        }

        debug!("state: got username");
        match client.recv().await {
            Some(ServerMessage::UsernameAccepted) => {
                server.send(ClientMessage::Joined(id)).ok();
            }
            Some(ServerMessage::UsernameInvalid) => {
                warn!("invalid username for id {id}");
                write.write_all(b"Invalid username\n").await.ok();
                server.send(ClientMessage::Disconnect(id)).ok();
                break;
            }
            None => {
                info!("got empty message");
                server.send(ClientMessage::Disconnect(id)).ok();
                break;
            }
            message => {
                error!("got invalid message from server, close connection: {message:?}");
                server.send(ClientMessage::Disconnect(id)).ok();
                break;
            }
        }

        debug!("state: joined");
        match client.recv().await {
            Some(ServerMessage::Users(users)) => {
                let mut message = String::new();
                message.push_str("* The room contains:");
                if !users.is_empty() {
                    message.push(' ');
                    let mut iter = users.iter();
                    message.push_str(iter.next().unwrap());
                    for user in iter {
                        message.push_str(", ");
                        message.push_str(user);
                    }
                }
                message.push('\n');

                write.write_all(message.as_bytes()).await.ok();
            }
            None => {
                info!("got empty message");
                server.send(ClientMessage::Disconnect(id)).ok();
                break;
            }
            message => {
                error!("got invalid message from server, close connection: {message:?}");
                server.send(ClientMessage::Disconnect(id)).ok();
                break;
            }
        }

        debug!("state: main loop");
        loop {
            tokio::select! {
                segment = segments.next_segment() => {
                    match segment {
                        Ok(Some(message)) => {
                            server.send(ClientMessage::Message(id, Arc::new(String::from_utf8_lossy(&message).into_owned()))).ok();
                        }
                        Ok(None) => {
                            info!("got empty message {id}");
                            server.send(ClientMessage::Disconnect(id)).ok();
                            break;
                        }
                        Err(err) => {
                            warn!("got {err:?}, disconnect {id}");
                            server.send(ClientMessage::Disconnect(id)).ok();
                            break;
                        }
                    }
                }
                message = client.recv() => {
                    match message {
                        Some(ServerMessage::AnnounceUser(user)) => {
                            let mut message = String::new();
                            writeln!(&mut message, "* {} has entered the room", *user).ok();
                            write.write_all(message.as_bytes()).await.ok();
                        }
                        Some(ServerMessage::Message(user, msg)) => {
                            let mut message = String::new();
                            writeln!(&mut message, "[{}] {}", *user, *msg).ok();
                            write.write_all(message.as_bytes()).await.ok();
                        }
                        Some(ServerMessage::Disconnected(user)) => {
                            let mut message = String::new();
                            writeln!(&mut message, "* {} has left the room", *user).ok();
                            write.write_all(message.as_bytes()).await.ok();
                        }
                        None => {
                            info!("got empty message");
                            server.send(ClientMessage::Disconnect(id)).ok();
                            break;
                        }
                        message => {
                            error!("got invalid message from server, close connection: {message:?}");
                            server.send(ClientMessage::Disconnect(id)).ok();
                            break;
                        }
                    }
                }
            }
        }

        debug!("state: done");
        server.send(ClientMessage::Disconnect(id)).ok();
        break;
    }

    info!("done {id}");
}
