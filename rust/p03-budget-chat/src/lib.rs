//! Budget chat
//!
//! Modern messaging software uses too many computing resources, so
//! we're going back to basics. Budget Chat is a simple TCP-based chat
//! room protocol.
//!
//! Each message is a single line of ASCII text terminated by a
//! newline character ('\n', or ASCII 10). Clients can send multiple
//! messages per connection. Servers may optionally strip trailing
//! whitespace, such as carriage return characters ('\r', or ASCII
//! 13). All messages are raw ASCII text, not wrapped up in JSON or
//! any other format.
//!
//! # Upon connection
//!
//! ## Setting the user's name
//!
//! When a client connects to the server, it does not yet have a name
//! and is not considered to have joined. The server must prompt the
//! user by sending a single message asking for a name. The exact text
//! of this message is implementation-defined.
//!
//! Example:
//!
//! ```raw
//! Welcome to budgetchat! What shall I call you?
//! ```
//!
//! The first message from a client sets the user's name, which must
//! contain at least 1 character, and must consist entirely of
//! alphanumeric characters (uppercase, lowercase, and digits).
//!
//! Implementations may limit the maximum length of a name, but must
//! allow at least 16 characters. Implementations may choose to either
//! allow or reject duplicate names.
//!
//! If the user requests an illegal name, the server may send an
//! informative error message to the client, and the server must
//! disconnect the client, without sending anything about the illegal
//! user to any other clients.
//!
//! ## Presence notification
//!
//! Once the user has a name, they have joined the chat room and the
//! server must announce their presence to other users (see "A user
//! joins" below).
//!
//! In addition, the server must send the new user a message that
//! lists all present users' names, not including the new user, and
//! not including any users who have already left. The exact text of
//! this message is implementation-defined, but must start with an
//! asterisk ('*'), and must contain the users' names. The server must
//! send this message even if the room was empty.
//!
//! Example:
//!
//! ```raw
//! * The room contains: bob, charlie, dave
//! ```
//!
//! All subsequent messages from the client are chat messages.
//!
//! ## Chat messages
//!
//! When a client sends a chat message to the server, the server must
//! relay it to all other clients as the concatenation of:
//!
//! ```raw
//!     open square bracket character
//!     the sender's name
//!     close square bracket character
//!     space character
//!     the sender's message
//! ```
//!
//! If "bob" sends "hello", other users would receive "\[bob\] hello".
//!
//! Implementations may limit the maximum length of a chat message,
//! but must allow at least 1000 characters.
//!
//! The server must not send the chat message back to the originating
//! client, or to connected clients that have not yet joined.
//!
//! For example, if a user called "alice" sends a message saying
//! "Hello, world!", all users except alice would receive:
//!
//! ```raw
//! [alice] Hello, world!
//! ```
//!
//! ## A user joins
//!
//! When a user joins the chat room by setting an acceptable name, the
//! server must send all other users a message to inform them that the
//! user has joined. The exact text of this message is
//! implementation-defined, but must start with an asterisk ('*'), and
//! must contain the user's name.
//!
//! Example:
//!
//! ```raw
//! * bob has entered the room
//! ```
//!
//! The server must send this message to other users that have already
//! joined, but not to connected clients that have not yet joined.
//!
//! ## A user leaves
//!
//! When a joined user is disconnected from the server for any reason,
//! the server must send all other users a message to inform them that
//! the user has left. The exact text of this message is
//! implementation-defined, but must start with an asterisk ('*'), and
//! must contain the user's name.
//!
//! Example:
//!
//! ```raw
//! * bob has left the room
//! ```
//!
//! The server must send this message to other users that have already
//! joined, but not to connected clients that have not yet joined.
//!
//! If a client that has not yet joined disconnects from the server,
//! the server must not send any messages about that client to other
//! clients.
//!
//! # Example session
//!
//! In this example, "-->" denotes messages from the server to Alice's
//! client, and "<--" denotes messages from Alice's client to the
//! server.
//!
//! ```raw
//! --> Welcome to budgetchat! What shall I call you?
//! <-- alice
//! --> * The room contains: bob, charlie, dave
//! <-- Hello everyone
//! --> [bob] hi alice
//! --> [charlie] hello alice
//! --> * dave has left the room
//! ```
//!
//! Alice connects and sets her name. She is given a list of users
//! already in the room. She sends a message saying "Hello
//! everyone". Bob and Charlie reply. Dave disconnects.
//!
//! # Other requirements
//!
//! - Accept TCP connections.
//!
//! - Make sure you support at least 10 simultaneous clients.
use std::collections::HashMap;
use std::fmt::Write;
use std::marker::PhantomData;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{
    tcp::{ReadHalf, WriteHalf},
    TcpListener, TcpStream,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use tracing::{debug, error, info, warn};

use thiserror::Error;

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

#[derive(Error, Debug)]
enum ClientError {
    #[error("internal error")]
    InternalError(#[from] anyhow::Error),

    #[error("invalid username")]
    InvalidUsername,
}

struct WaitingWelcome;
struct Joining;
struct Joined;
struct Chatting;

struct Client<'a, T> {
    id: ID,
    read: BufReader<ReadHalf<'a>>,
    write: WriteHalf<'a>,
    server: &'a mut UnboundedSender<ClientMessage>,
    #[allow(clippy::struct_field_names)]
    client: UnboundedReceiver<ServerMessage>,
    _state: PhantomData<T>,
}

impl<'a, T> Client<'a, T> {
    fn into<D>(self) -> Client<'a, D> {
        Client {
            id: self.id,
            read: self.read,
            write: self.write,
            server: self.server,
            client: self.client,
            _state: PhantomData,
        }
    }
}

impl<'a> Client<'a, WaitingWelcome> {
    #[tracing::instrument(skip(self))]
    async fn welcome(mut self) -> Result<Option<Client<'a, Joining>>, ClientError> {
        debug!("state: waiting welcome message");
        match self.client.recv().await {
            Some(ServerMessage::Welcome) => {
                debug!("got welcome message");
                self.write
                    .write_all(b"Welcome to budgetchat! What shall I call you?\n")
                    .await
                    .map_err(|e| ClientError::InternalError(e.into()))?;
                Ok(Some(self.into()))
            }
            None => Ok(None),
            message => unreachable!("invalid server message {:?}", message),
        }
    }
}

impl<'a> Client<'a, Joining> {
    #[tracing::instrument(skip(self))]
    async fn joining(mut self) -> Result<Option<Client<'a, Joined>>, ClientError> {
        debug!("state: joining");
        let mut buffer = vec![];
        match self.read.read_until(b'\n', &mut buffer).await {
            Ok(_) if buffer.last() == Some(&b'\n') => {
                debug!("got username");
                self.server
                    .send(ClientMessage::SetUsername(
                        self.id,
                        String::from_utf8_lossy(&buffer[..buffer.len() - 1]).into_owned(),
                    ))
                    .map_err(|e| ClientError::InternalError(e.into()))?;
            }
            _ => return Ok(None),
        }

        match self.client.recv().await {
            Some(ServerMessage::UsernameAccepted) => {
                self.server
                    .send(ClientMessage::Joined(self.id))
                    .map_err(|e| ClientError::InternalError(e.into()))?;
                Ok(Some(self.into()))
            }
            Some(ServerMessage::UsernameInvalid) => {
                self.write
                    .write_all(b"Invalid username\n")
                    .await
                    .map_err(|e| ClientError::InternalError(e.into()))?;
                Err(ClientError::InvalidUsername)
            }
            None => Ok(None),
            message => unreachable!("invalid server message {:?}", message),
        }
    }
}

impl<'a> Client<'a, Joined> {
    #[tracing::instrument(skip(self))]
    async fn joined(mut self) -> Result<Option<Client<'a, Chatting>>, ClientError> {
        debug!("state: joined");
        match self.client.recv().await {
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

                self.write
                    .write_all(message.as_bytes())
                    .await
                    .map_err(|e| ClientError::InternalError(e.into()))?;
                Ok(Some(self.into()))
            }
            None => Ok(None),
            message => unreachable!("invalid server message {:?}", message),
        }
    }
}

impl<'a> Client<'a, Chatting> {
    #[tracing::instrument(skip(self))]
    async fn chatting(mut self) -> Result<(), ClientError> {
        debug!("state: main loop");
        let mut buffer = vec![];
        loop {
            tokio::select! {
                segment = self.read.read_until(b'\n', &mut buffer) => {
                    match segment {
                        Ok(_) if buffer.last() == Some(&b'\n') => {
                            self.server.send(ClientMessage::Message(self.id, Arc::new(String::from_utf8_lossy(&buffer[..buffer.len() - 1]).into_owned()))).map_err(|e| ClientError::InternalError(e.into()))?;
                            buffer.clear();
                        }
                        _ => break,
                    }
                }

                message = self.client.recv() => {
                    match message {
                        Some(ServerMessage::AnnounceUser(user)) => {
                            let mut message = String::new();
                            writeln!(&mut message, "* {} has entered the room", *user).ok();
                            self.write.write_all(message.as_bytes()).await.map_err(|e| ClientError::InternalError(e.into()))?;
                        }
                        Some(ServerMessage::Message(user, msg)) => {
                            let mut message = String::new();
                            writeln!(&mut message, "[{}] {}", *user, *msg).ok();
                            self.write.write_all(message.as_bytes()).await.map_err(|e| ClientError::InternalError(e.into()))?;
                        }
                        Some(ServerMessage::Disconnected(user)) => {
                            let mut message = String::new();
                            writeln!(&mut message, "* {} has left the room", *user).ok();
                            self.write.write_all(message.as_bytes()).await.map_err(|e| ClientError::InternalError(e.into()))?;
                        }
                        None => break,
                        message => unreachable!("invalid server message {:?}", message),
                    }
                }
            }
        }

        Ok(())
    }
}

/// Client iterations.
///
/// Listen for a client and run the chat.
///
/// # Errors:
/// * Error when socket returns an error.
#[tracing::instrument(skip(stream, server, client))]
async fn handle_client(
    id: ID,
    mut stream: TcpStream,
    mut server: UnboundedSender<ClientMessage>,
    client: UnboundedReceiver<ServerMessage>,
) {
    info!("start {id}");

    let (read, write) = stream.split();
    let client = Client {
        id,
        read: BufReader::new(read),
        write,
        server: &mut server,
        client,
        _state: PhantomData::<WaitingWelcome>,
    };

    let client = match client.welcome().await {
        Ok(Some(client)) => client,
        Ok(None) => {
            server.send(ClientMessage::Disconnect(id)).ok();
            return;
        }
        Err(err) => {
            error!("error {:?}", err);
            server.send(ClientMessage::Disconnect(id)).ok();
            return;
        }
    };

    let client = match client.joining().await {
        Ok(Some(client)) => client,
        Ok(None) => {
            server.send(ClientMessage::Disconnect(id)).ok();
            return;
        }
        Err(err) => {
            error!("error {:?}", err);
            server.send(ClientMessage::Disconnect(id)).ok();
            return;
        }
    };

    let client = match client.joined().await {
        Ok(Some(client)) => client,
        Ok(None) => {
            server.send(ClientMessage::Disconnect(id)).ok();
            return;
        }
        Err(err) => {
            error!("error {:?}", err);
            server.send(ClientMessage::Disconnect(id)).ok();
            return;
        }
    };

    if let Err(err) = client.chatting().await {
        error!("error {:?}", err);
        server.send(ClientMessage::Disconnect(id)).ok();
    } else {
        debug!("state: done");
        server.send(ClientMessage::Disconnect(id)).ok();
    }

    info!("done {id}");
}
