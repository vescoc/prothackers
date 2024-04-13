//! Prime Time
//!
//! To keep costs down, a hot new government department is contracting
//! out its mission-critical primality testing to the lowest
//! bidder. (That's you).
//!
//! Officials have devised a JSON-based request-response
//! protocol. Each request is a single line containing a JSON object,
//! terminated by a newline character ('\n', or ASCII 10). Each
//! request begets a response, which is also a single line containing
//! a JSON object, terminated by a newline character.
//!
//! After connecting, a client may send multiple requests in a single
//! session. Each request should be handled in order.
//!
//! A conforming request object has the required field method, which
//! must always contain the string "isPrime", and the required field
//! number, which must contain a number. Any JSON number is a valid
//! number, including floating-point values.
//!
//! # Example request:
//!
//! ```json
//! {"method":"isPrime","number":123}
//! ```
//!
//! A request is malformed if it is not a well-formed JSON object, if
//! any required field is missing, if the method name is not
//! "isPrime", or if the number value is not a number.
//!
//! Extraneous fields are to be ignored.
//!
//! A conforming response object has the required field method, which
//! must always contain the string "isPrime", and the required field
//! prime, which must contain a boolean value: true if the number in
//! the request was prime, false if it was not.
//!
//! # Example response:
//!
//! ```json
//! {"method":"isPrime","prime":false}
//! ```
//!
//! A response is malformed if it is not a well-formed JSON object, if
//! any required field is missing, if the method name is not
//! "isPrime", or if the prime value is not a boolean.
//!
//! A response object is considered incorrect if it is well-formed but
//! has an incorrect prime value. Note that non-integers can not be
//! prime.
//!
//! Accept TCP connections.
//!
//! Whenever you receive a conforming request, send back a correct
//! response, and wait for another request.
//!
//! Whenever you receive a malformed request, send back a single
//! malformed response, and disconnect the client.
//!
//! Make sure you can handle at least 5 simultaneous clients.
use tracing::{debug, warn};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

const METHOD: &str = "isPrime";

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Request<'a> {
    pub method: &'a str,
    pub number: serde_json::Number,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Response<'a> {
    pub method: &'a str,
    pub prime: bool,
}

/// Check if the messages are valid and primes.
///
/// # Errors
/// * Error when socket returns an error.
#[tracing::instrument(skip(stream))]
pub async fn handler(mut stream: TcpStream) -> Result<(), anyhow::Error> {
    debug!("start");

    let (read_half, mut write_half) = stream.split();
    let mut read_half = BufReader::new(read_half);

    let mut buffer = vec![];
    loop {
        buffer.clear();
        match read_half.read_until(b'\n', &mut buffer).await {
            Ok(0) => break,
            Ok(n) => {
                let end = if buffer[n - 1] == b'\n' { n - 1 } else { n };
                match check(&buffer[0..end]) {
                    Ok(is_prime) => {
                        write_half
                            .write_all(
                                &serde_json::to_vec(&Response {
                                    method: METHOD,
                                    prime: is_prime,
                                })
                                .unwrap(),
                            )
                            .await?;
                        write_half.write_u8(b'\n').await?;
                    }
                    Err(err) => {
                        warn!("invalid request");
                        write_half.write_all(err.as_bytes()).await?;
                        break;
                    }
                }
            }
            Err(err) => return Err(err.into()),
        }
    }

    write_half.flush().await?;
    write_half.shutdown().await?;

    debug!("end");

    Ok(())
}

fn check(buffer: &[u8]) -> Result<bool, &'static str> {
    let request = serde_json::from_slice::<Request>(buffer).map_err(|_| "MALFORMED")?;
    match (request.method, request.number.as_u64()) {
        ("isPrime", Some(n)) => {
            let result = primes::is_prime(n);
            debug!("isPrime for {n}: {result}");
            Ok(result)
        }
        ("isPrime", None) => {
            debug!("isPrime for a non u64 number");
            Ok(false)
        }
        _ => Err("MALFORMED"),
    }
}
