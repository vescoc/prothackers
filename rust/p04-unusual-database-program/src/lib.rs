//! # Unusual Database Program
//!
//! It's your first day on the job. Your predecessor, Ken, left in
//! mysterious circumstances, but not before coming up with a protocol
//! for the new key-value database. You have some doubts about Ken's
//! motivations, but there's no time for questions! Let's implement
//! his protocol.
//!
//! Ken's strange database is a key-value store accessed over
//! UDP. Since UDP does not provide retransmission of dropped packets,
//! and neither does Ken's protocol, clients have to be careful not to
//! send requests too fast, and have to accept that some requests or
//! responses may be dropped.
//!
//! Each request, and each response, is a single UDP packet.
//!
//! There are only two types of request: insert and retrieve. Insert
//! allows a client to insert a value for a key, and retrieve allows a
//! client to retrieve the value for a key.
//!
//! ## Insert
//!
//! A request is an insert if it contains an equals sign ("=", or
//! ASCII 61).
//!
//! The first equals sign separates the key from the value. This means
//! keys can not contain the equals sign. Other than the equals sign,
//! keys can be made up of any arbitrary characters. The empty string
//! is a valid key.
//!
//! Subsequent equals signs (if any) are included in the value. The
//! value can be any arbitrary data, including the empty string.
//!
//! For example:
//!
//! - `foo=bar` will insert a key foo with value "bar".
//!
//! - `foo=bar=baz` will insert a key foo with value "bar=baz".
//!
//! - `foo=` will insert a key foo with value "" (i.e. the empty
//! string).
//!
//! - `foo===` will insert a key foo with value "==".
//!
//! - `=foo` will insert a key of the empty string with value "foo".
//!
//! If the server receives an insert request for a key that already
//! exists, the stored value must be updated to the new value.
//!
//! An insert request does not yield a response.
//!
//! ## Retrieve
//!
//! A request that does not contain an equals sign is a retrieve
//! request.
//!
//! In response to a retrieve request, the server must send back the
//! key and its corresponding value, separated by an equals
//! sign. Responses must be sent to the IP address and port number
//! that the request originated from, and must be sent from the IP
//! address and port number that the request was sent to.
//!
//! If a requests is for a key that has been inserted multiple times,
//! the server must return the most recent value.
//!
//! If a request attempts to retrieve a key for which no value exists,
//! the server can either return a response as if the key had the
//! empty value (e.g. "key="), or return no response at all.
//!
//! Example request:
//!
//! ```raw
//! message
//! ```
//!
//! Example response:
//!
//! ```raw
//! message=Hello,world!
//! ```
//!
//! ## Version reporting
//!
//! The server must implement the special key "version". This should
//! identify the server software in some way (for example, it could
//! contain the software name and version number). It must not be the
//! empty string.
//!
//! Attempts by clients to modify the version must be ignored.
//!
//! Example request:
//!
//! ```raw
//! version
//! ```
//!
//! Example response:
//!
//! ```raw
//! version=Ken's Key-Value Store 1.0
//! ```
//!
//! ## Other requirements
//!
//! All requests and responses must be shorter than 1000 bytes.
//!
//! Issues related to UDP packets being dropped, delayed, or reordered
//! are considered to be the client's problem. The server should act
//! as if it assumes that UDP works reliably.
use std::collections::HashMap;
use std::io;

use tokio::net::UdpSocket;

use tracing::debug;

#[tracing::instrument(skip(socket))]
pub async fn run(socket: UdpSocket) -> Result<(), io::Error> {
    debug!(
        "socket addr: {:?} ttl: {:?}",
        socket.local_addr(),
        socket.ttl()
    );

    let mut data = HashMap::new();

    let mut buffer = [0; 1024];
    loop {
        let (len, addr) = socket.recv_from(&mut buffer).await?;

        let buffer = &buffer[0..len];
        let mut iter = buffer.iter();
        if let Some(position) = iter.position(|c| *c == b'=') {
            let key = &buffer[0..position];
            if key != b"version" {
                let value = iter.as_slice();
                data.insert(key.to_vec(), value.to_vec());
                debug!(
                    "[{:?}] insert key: {:?} value: {:?}",
                    addr,
                    std::str::from_utf8(key),
                    std::str::from_utf8(value)
                );
            }
        } else {
            let key = buffer;
            debug!("[{:?}] retrieve key: {:?}", addr, std::str::from_utf8(key));
            if key == b"version" {
                socket
                    .send_to(b"version=unusual-database-program 1.0.0", addr)
                    .await?;
            } else if let Some(value) = data.get(key) {
                debug!("[{:?}] value: {:?}", addr, std::str::from_utf8(value));

                socket.send_to(&[key, value].join(&b"="[..]), addr).await?;
            } else {
                debug!("[{:?}] no value", addr);
            }
        }
    }
}
