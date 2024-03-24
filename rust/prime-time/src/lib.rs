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
