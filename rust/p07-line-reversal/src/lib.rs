//! Line Reversal
//!
//! We're going to be writing a simple network server to reverse the
//! characters within lines of ASCII text. For example, we'll turn
//! "hello" into "olleh".
//!
//! There's just one snag: we've never heard of TCP! Instead, we've
//! designed our own connection-oriented byte stream protocol that
//! runs on top of UDP, called "Line Reversal Control Protocol", or
//! LRCP for short.
//!
//! The goal of LRCP is to turn unreliable and out-of-order UDP
//! packets into a pair of reliable and in-order byte streams. To
//! achieve this, it maintains a per-session payload length counter on
//! each side, labels all payload transmissions with their position in
//! the overall stream, and retransmits any data that has been
//! dropped. A sender detects that a packet has been dropped either by
//! not receiving an acknowledgment within an expected time window, or
//! by receiving a duplicate of a prior acknowledgement.
//!
//! Client sessions are identified by a numeric session token which is
//! supplied by the client. You can assume that session tokens
//! uniquely identify clients, and that the peer for any given session
//! is at a fixed IP address and port number.
//!
//! # Messages
//!
//! Messages are sent in UDP packets. Each UDP packet contains a
//! single LRCP message. Each message consists of a series of values
//! separated by forward slash characters ("/"), and starts and ends
//! with a forward slash character, like so:
//!
//! ```raw
//! /data/1234567/0/hello/
//! ```
//!
//! The first field is a string specifying the message type (here,
//! "data"). The remaining fields depend on the message type. Numeric
//! fields are represented as ASCII text.
//!
//! ### Validation
//!
//! When the server receives an illegal packet it must silently ignore
//! the packet instead of interpreting it as LRCP.
//!
//! 1. Packet contents must begin with a forward slash, end with a
//! forward slash, have a valid message type, and have the correct
//! number of fields for the message type.
//!
//! 2. Numeric field values must be smaller than 2147483648. This
//! means sessions are limited to 2 billion bytes of data transferred
//! in each direction.
//!
//! 3. LRCP messages must be smaller than 1000 bytes. You might have
//! to break up data into multiple data messages in order to fit it
//! below this limit.
//!
//! ### Parameters
//!
//! - retransmission timeout: the time to wait before retransmitting a
//! message. Suggested default value: **3 seconds**.
//!
//! - session expiry timeout: the time to wait before accepting that a
//! peer has disappeared, in the event that no responses are being
//! received. Suggested default value: **60 seconds**.
//!
//! ## 1. `/connect/SESSION/`
//!
//! This message is sent by a client, to a server, to request that a
//! session is opened. The SESSION field must be a non-negative
//! integer.
//!
//! If a client does not receive a response to a connect message
//! within the retransmission timeout (e.g. the request or response
//! may have been dropped), it will re-send the connect message,
//! multiple times if necessary.
//!
//! For the purposes of the Line Reversal application, your server
//! will never need to initiate the opening of any sessions.
//!
//! When you receive a connect message
//!
//! 1. If no session with this token is open: open one, and associate
//! it with the IP address and port number that the UDP packet
//! originated from.
//!
//! 2. Send /ack/SESSION/0/ to let the client know that the session is
//! open (do this even if it is a duplicate connect, because the first
//! ack may have been dropped).
//!
//! ### Example: open session number 1234567:
//!
//! ```raw
//! <-- /connect/1234567/
//! --> /ack/1234567/0/
//! ```
//!
//! ## 2. `/data/SESSION/POS/DATA/`
//!
//! This message transmits payload data. The POS field must be a
//! non-negative integer representing the position in the stream that
//! the DATA belongs.
//!
//! Where the DATA contains forward slash ("/") or backslash ("\")
//! characters, the sender must escape the slashes by prepending them
//! each with a single backslash character ("foo/bar\baz" becomes
//! "foo\/bar\\baz"). This escaping must be reversed by the recipient
//! before passing it to the application layer. All unescaped
//! characters are interpreted as literal characters, including
//! control characters such as newline characters.
//!
//! The POS field refers to the position in the stream of unescaped
//! application-layer bytes, not the escaped data passed in LRCP.
//!
//! Behaviour is undefined if a peer sends payload data that overlaps
//! with payload data you've already received, but differs from it.
//!
//! When you want to send payload data, send it as a data packet. If
//! the payload you sent hasn't been acknowledged within the
//! retransmission timeout, send it again. Do this multiple times if
//! necessary. If the data hasn't been acknowledged within the session
//! expiry timeout, consider the session closed.
//!
//! When you receive a data message
//!
//! - If the session is not open: `send /close/SESSION/` and stop.
//!
//! - If you've already received everything up to POS: unescape "\\"
//! and "\/", find the total LENGTH of unescaped data that you've
//! already received (including the data in this message, if any),
//! send `/ack/SESSION/LENGTH/`, and pass on the new data (if any) to
//! the application layer.
//!
//! - If you have not received everything up to POS: send a duplicate
//! of your previous ack (or `/ack/SESSION/0/` if none), saying how much
//! you have received, to provoke the other side to retransmit
//! whatever you're missing.
//!
//! ### Example: transmit "hello", starting at the very start of session 1234567:
//!
//! ```raw
//! <-- /data/1234567/0/hello/
//! --> /ack/1234567/5/
//! ```
//!
//! ### Example: transmit a single forward slash, starting at the very start of session 1234568:
//!
//! ```raw
//! <-- /data/1234568/0/\//
//! --> /ack/1234568/1/ # note: 1, not 2, because the sequence "\/" only represents 1 byte of data
//! ```
//!
//! ## 3. `/ack/SESSION/LENGTH/`
//!
//! This message acknowledges receipt of payload data. The LENGTH
//! field must be a non-negative integer telling the other side how
//! many bytes of payload have been successfully received so far.
//!
//! When you receive an ack message
//!
//! - If the SESSION is not open: `send /close/SESSION/` and stop.
//!
//! - If the LENGTH value is not larger than the largest LENGTH value
//! in any ack message you've received on this session so far: do
//! nothing and stop (assume it's a duplicate ack that got delayed).
//!
//! - If the LENGTH value is larger than the total amount of payload
//! you've sent: the peer is misbehaving, close the session.
//!
//! - If the LENGTH value is smaller than the total amount of payload
//! you've sent: retransmit all payload data after the first LENGTH
//! bytes.
//!
//! - If the LENGTH value is equal to the total amount of payload
//! you've sent: don't send any reply.
//!
//! ### Example: acknowledge reading the first 1024 bytes of content, on session 1234567:
//!
//! ```raw
//! /ack/1234567/1024/
//! ```
//!
//! ## 4. `/close/SESSION/`
//!
//! This message requests that the session is closed. This can be
//! initiated by either the server or the client.
//!
//! For the purposes of the Line Reversal application, your server
//! will never need to initiate the closing of any sessions.
//!
//! When you receive a `/close/SESSION/` message, send a matching close
//! message back.
//!
//! ### Example: close session 1234567:
//!
//! ```raw
//! <-- /close/1234567/
//! --> /close/1234567/
//! ```
//!
//! ## Example session
//!
//! The client connects with session token 12345, sends "Hello,
//! world!" and then closes the session.
//!
//! ```raw
//! <-- /connect/12345/
//! --> /ack/12345/0/
//! <-- /data/12345/0/Hello, world!/
//! --> /ack/12345/13/
//! <-- /close/12345/
//! --> /close/12345/
//! ```
//!
//! # Application layer: Line Reversal
//!
//! Accept LRCP connections. Make sure you support at least 20
//! simultaneous sessions.
//!
//! Reverse each line of input. Each line will be no longer than
//! 10,000 characters. Lines contain ASCII text and are delimited by
//! ASCII newline characters ("\n").
//!
//! From the LRCP perspective, a given data message can contain bytes
//! for one or more lines in a single packet, it doesn't matter how
//! they're chunked, and a line isn't complete until the newline
//! character. The abstraction presented to the application layer
//! should be that of a pair of byte streams (one for sending and one
//! for receiving).
//!
//! ### Example session at application layer ("-->" denotes lines from the server to the client, and "<--" denotes lines from the client to the server):
//!
//! ```raw
//! <-- hello
//! --> olleh
//! <-- Hello, world!
//! --> !dlrow ,olleH
//! ```
//!
//! The same session at the LRCP layer might look like this ("\n"
//! denotes an ASCII newline character, "-->" denotes UDP packets from
//! the server to the client, and "<--" denotes UDP packets from the
//! client to the server):
//!
//! ```raw
//! <-- /connect/12345/
//! --> /ack/12345/0/
//! <-- /data/12345/0/hello\n/
//! --> /ack/12345/6/
//! --> /data/12345/0/olleh\n/
//! <-- /ack/12345/6/
//! <-- /data/12345/6/Hello, world!\n/
//! --> /ack/12345/20/
//! --> /data/12345/6/!dlrow ,olleH\n/
//! <-- /ack/12345/20/
//! <-- /close/12345/
//! --> /close/12345/
//! ```
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use tracing::{debug, info, warn};

use tokio::io::{split, AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

pub mod lrcp;

use lrcp::packets::SyncWrite;
use lrcp::protocol::{Endpoint, Packet, Socket, SocketHandler, Stream};

const RETRASMISSION_TIMEOUT: Duration = Duration::from_secs(3);
const SESSION_EXPIRE_TIMEOUT: Duration = Duration::from_secs(60);

pub struct DefaultSocketHandler;

impl SocketHandler for DefaultSocketHandler {
    const RETRASMISSION_TIMEOUT: Duration = RETRASMISSION_TIMEOUT;
    const SESSION_EXPIRE_TIMEOUT: Duration = SESSION_EXPIRE_TIMEOUT;
}

pub struct UdpEndpoint(UdpSocket);

impl
    Endpoint<
        (SocketAddr, Packet),
        mpsc::UnboundedReceiver<(SocketAddr, Packet)>,
        mpsc::UnboundedSender<(SocketAddr, Packet)>,
    > for UdpEndpoint
{
    fn split(
        self,
    ) -> (
        mpsc::UnboundedReceiver<(SocketAddr, Packet)>,
        mpsc::UnboundedSender<(SocketAddr, Packet)>,
    ) {
        let (upstream_sender, upstream_receiver) = mpsc::unbounded_channel();
        let (downstream_sender, mut downstream_receiver) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut buffer = [0; 1024];
            loop {
                tokio::select! {
                    Ok((len, addr)) = self.0.recv_from(&mut buffer) => {
                        let buffer = &buffer[0..len];

                        if let Ok(packet) = Packet::try_from(buffer) {
                            info!("server --> send upstream packet {packet:?} from: {addr}");
                            if let Err(e) = upstream_sender.send((addr, packet)) {
                                warn!("error on send: {e:?}");
                            }
                        } else {
                            warn!("invalid packet: {buffer:?} [{:?}]", String::from_utf8_lossy(buffer));
                        }
                    }

                    Some((addr, packet)) = downstream_receiver.recv() => {
                        info!("server <-- send downstream packet {packet:?} to: {addr}");

                        let mut buffer = [0_u8; 1024];
                        let mut b = buffer.as_mut_slice();

                        let len = b.write_value(&packet).unwrap();

                        self.0.send_to(&buffer[..len], addr).await.unwrap();
                    }
                }
            }
        });

        (upstream_receiver, downstream_sender)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum LineReversalError {
    #[error("io internal error")]
    IoError(#[from] io::Error),
}

#[tracing::instrument(skip(socket))]
pub async fn run<H: SocketHandler + Send>(socket: UdpSocket) -> Result<(), LineReversalError> {
    debug!(
        "socket addr: {:?} ttl: {:?}",
        socket.local_addr(),
        socket.ttl(),
    );

    let endpoint = UdpEndpoint(socket);

    let mut listener = Socket::<H>::listener(endpoint)?;
    loop {
        let stream = listener.accept().await?;
        tokio::spawn(async move {
            handle(stream).await.ok();
        });
    }
}

#[tracing::instrument(skip(stream))]
async fn handle<R: Unpin, W: Unpin>(stream: Stream<R, W>) -> Result<(), LineReversalError> {
    let (read, write) = split(stream);
    let mut read = BufReader::new(read);
    let mut write = BufWriter::new(write);

    let mut line = String::new();
    loop {
        debug!("handle");

        line.clear();
        let len = read.read_line(&mut line).await?;
        info!("working on {line}");
        if len > 0 {
            for b in line.bytes().take(len - 1).rev() {
                write.write_u8(b).await?;
            }
            write.write_u8(b'\n').await?;
            write.flush().await?;
        } else {
            warn!("line is empty");
        }
    }
}
