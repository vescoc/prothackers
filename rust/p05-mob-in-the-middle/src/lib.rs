//! Mob in the middle
//!
//! You're escorted to a dark, smoky, basement office. Big Tony sits
//! the other side of a large desk, leaning back in his chair, puffing
//! on a cigar that you can only describe as
//! comedically-oversized. Two of his goons loiter in the
//! doorway. They are tall and wide but not obviously very bright,
//! which only makes them all the more intimidating. Tony flashes a
//! menacing grin, revealing an unusual number of gold-plated teeth,
//! and makes you an offer you can't refuse: he wants you to write a
//! malicious proxy server for Budget Chat.
//!
//! For each client that connects to your proxy server, you'll make a
//! corresponding outward connection to the upstream server. When the
//! client sends a message to your proxy, you'll pass it on
//! upstream. When the upstream server sends a message to your proxy,
//! you'll pass it on downstream. Remember that messages in Budget
//! Chat are delimited by newline characters ('\n', or ASCII 10).
//!
//! Most messages are passed back and forth without modification, so
//! that the client believes it is talking directly to the upstream
//! server, except that you will be rewriting Boguscoin addresses, in
//! both directions, so that all payments go to Tony.
//!
//! # Connecting to the upstream server
//!
//! The upstream Budget Chat server is at chat.protohackers.com on
//! port 16963. You can connect using either IPv4 or IPv6.
//!
//! # Rewriting Boguscoin addresses
//!
//! Tony is trying to steal people's cryptocurrency. He has already
//! arranged to have his victim's internet connections compromised,
//! and to have their Budget Chat sessions re-routed to your proxy
//! server.
//!
//! Your server will rewrite Boguscoin addresses, in both directions,
//! so that they are always changed to Tony's address instead.
//!
//! A substring is considered to be a Boguscoin address if it
//! satisfies all of:
//!
//! - it starts with a "7"
//! - it consists of at least 26, and at most 35, alphanumeric characters
//! - it starts at the start of a chat message, or is preceded by a space
//! - it ends at the end of a chat message, or is followed by a space
//!
//! You should rewrite all Boguscoin addresses to Tony's address,
//! which is
//! ```raw
//! 7YWHMfk9JZe0LM0g1ZauHuiSxhI
//! ```
//!
//! Some more example Boguscoin addresses:
//!
//! ```raw
//!     7F1u3wSD5RbOHQmupo9nx4TnhQ
//!     7iKDZEwPZSqIvDnHvVN2r0hUWXD5rHX
//!     7LOrwbDlS8NujgjddyogWgIM93MV5N2VR
//!     7adNeSwJkMakpEcln9HEtthSRtxdmEHOT8T
//! ```
//!
//! # Example session
//!
//! In this first example, "-->" denotes messages from the proxy
//! server to Bob's client, and "<--" denotes messages from Bob's
//! client to the proxy server.
//!
//! ```raw
//! --> Welcome to budgetchat! What shall I call you?
//! <-- bob
//! --> * The room contains: alice
//! <-- Hi alice, please send payment to 7iKDZEwPZSqIvDnHvVN2r0hUWXD5rHX
//! ```
//!
//! Bob connects to the server and asks Alice to send payment.
//!
//! In this next example, "-->" denotes messages from the upstream
//! server to the proxy server, and "<--" denotes messages from the
//! proxy server to the upstream server.
//!
//! ```raw
//! --> Welcome to budgetchat! What shall I call you?
//! <-- bob
//! --> * The room contains: alice
//! <-- Hi alice, please send payment to 7YWHMfk9JZe0LM0g1ZauHuiSxhI
//! ```
//!
//! Bob's Boguscoin address has been replaced with Tony's, but
//! everything else is unchanged. If Alice sends payment to this
//! address, it will go to Tony instead of Bob. Tony will be pleased,
//! and will elect not to have his goons break your kneecaps.
//!
//! # Other requirements
//!
//! Make sure your proxy server supports at least 10 simultaneous
//! clients.
//!
//! When either a client or an upstream connection disconnects from
//! your proxy server, disconnect the other side of the same
//! session. (But you don't have to worry about half-duplex
//! shutdowns.)
//!
//! As a reminder, Tony's Boguscoin address is:
//!
//! ```raw
//! 7YWHMfk9JZe0LM0g1ZauHuiSxhI
//! ```
use std::borrow::Cow;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};

use tracing::debug;

pub const BOGUSCOIN: &str = "7YWHMfk9JZe0LM0g1ZauHuiSxhI";

#[tracing::instrument(skip(listener, chat_address, chat_port, boguscoin))]
pub async fn run(
    listener: TcpListener,
    chat_address: String,
    chat_port: u16,
    boguscoin: String,
) -> Result<(), anyhow::Error> {
    loop {
        let (stream, _) = listener.accept().await?;
        let chat_address = chat_address.clone();
        let boguscoin = boguscoin.clone();
        tokio::spawn(async move {
            handle(stream, chat_address, chat_port, boguscoin)
                .await
                .ok();
        });
    }
}

#[tracing::instrument(skip(stream, chat_address, chat_port, boguscoin))]
async fn handle(
    mut stream: TcpStream,
    chat_address: String,
    chat_port: u16,
    boguscoin: String,
) -> Result<(), anyhow::Error> {
    let trasform = |message: &[u8]| -> String {
        String::from_utf8_lossy(message)
            .split_ascii_whitespace()
            .map(|word| {
                if word.starts_with('7')
                    && (26..=35).contains(&word.len())
                    && word.chars().all(char::is_alphanumeric)
                {
                    Cow::Borrowed(boguscoin.as_str())
                } else {
                    Cow::Borrowed(word)
                }
            })
            .fold(String::new(), |mut s, word| {
                if s.is_empty() {
                    word.into_owned()
                } else {
                    s.push(' ');
                    s.push_str(&word);
                    s
                }
            })
    };

    let (client_read, client_write) = stream.split();
    let mut client_read = BufReader::new(client_read);
    let mut client_write = BufWriter::new(client_write);

    let mut chat_stream = TcpStream::connect(&format!("{chat_address}:{chat_port}")).await?;
    let (chat_read, chat_write) = chat_stream.split();
    let mut chat_read = BufReader::new(chat_read);
    let mut chat_write = BufWriter::new(chat_write);

    let mut done_upstream = false;
    let mut done_downstream = false;
    let (mut client_buffer, mut chat_buffer) = (vec![], vec![]);
    loop {
        tokio::select! {
            biased;

            client_result = client_read.read_until(b'\n', &mut client_buffer), if !done_upstream => {
                match client_result {
                    Ok(_) if client_buffer.last() == Some(&b'\n') => {
                        let message = trasform(&client_buffer);
                        client_buffer.clear();

                        debug!("--> {message}");

                        chat_write.write_all(message.as_bytes()).await?;
                        chat_write.write_u8(b'\n').await?;
                        chat_write.flush().await?;
                    }
                    _ => {
                        debug!("--> <CLOSING>");
                        done_upstream = true;
                        chat_write.shutdown().await?;
                    }
                }
            }

            chat_result = chat_read.read_until(b'\n', &mut chat_buffer), if !done_downstream => {
                match chat_result {
                    Ok(_) if chat_buffer.last() == Some(&b'\n') => {
                        let message = trasform(&chat_buffer);
                        chat_buffer.clear();

                        debug!("--> {message}");

                        client_write.write_all(message.as_bytes()).await?;
                        client_write.write_u8(b'\n').await?;
                        client_write.flush().await?;
                    }
                    _ => {
                        debug!("<-- <CLOSING>");
                        done_downstream = true;
                        client_write.shutdown().await?;
                    }
                }
            }

            else => break,
        }
    }

    Ok(())
}
