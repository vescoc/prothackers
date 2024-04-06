use std::collections::BTreeMap;
use std::mem;

use tracing::{debug, warn};

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// Handle a client session.
///
/// # Errors
/// * Error when socket returns and error.
#[tracing::instrument(skip(stream))]
pub async fn handler(mut stream: TcpStream) -> Result<(), anyhow::Error> {
    debug!("start");

    let mut items = BTreeMap::new();

    let (read_half, mut write_half) = stream.split();
    let mut read_half = BufReader::new(read_half);

    let mut command = [0; 9];
    loop {
        read_half.read_exact(&mut command).await?;

        match (
            command[0],
            i32::from_be_bytes(command[1..=mem::size_of::<i32>()].try_into().unwrap()),
            i32::from_be_bytes(command[1 + mem::size_of::<i32>()..].try_into().unwrap()),
        ) {
            (b'I', timestamp, price) => {
                debug!("I {timestamp} {price}");
                items.insert(timestamp, price);
            }
            (b'Q', mintime, maxtime) if maxtime >= mintime => {
                let mut count = 0;
                let mut sum = 0;
                for (_, value) in items.range(mintime..=maxtime) {
                    count += 1;
                    sum += i64::from(*value);
                }
                let mean = if count > 0 { sum / count } else { 0 };
                debug!("Q {mintime} {maxtime}: {mean}");
                write_half.write_i32(i32::try_from(mean)?).await?;
            }
            (b'Q', _, _) => {
                debug!("Q invalid range");
                write_half.write_i32(0).await?;
            }
            _ => {
                warn!("invalid request");
                break;
            }
        }
    }

    write_half.flush().await?;
    write_half.shutdown().await?;

    debug!("done");

    Ok(())
}
