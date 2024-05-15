#![doc = include_str!("../README.md")]

use std::sync::Arc;

use futures::channel::mpsc;

use wasi::io::streams::StreamError;
use wasi::sockets::network;

use thiserror::Error;

#[allow(warnings)]
mod bindings;

mod client;
mod server;

use client::handle_client;
pub use server::run;

#[derive(Error, Debug)]
pub enum Error {
    #[error("stream error {0}")]
    Stream(#[from] StreamError),

    #[error("tcp socket error {0}")]
    TcpSocket(#[from] network::ErrorCode),

    #[error("send error {0}")]
    Send(#[from] mpsc::SendError),

    #[error("invalid username")]
    InvalidUsername,
}

type Id = usize;
type Username = String;
type Message = String;

#[derive(Debug)]
enum ClientMessage {
    SetUsername(Id, Username),
    Joined(Id),
    Message(Id, Arc<Message>),
    Disconnect(Id),
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
