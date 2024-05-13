#![doc = include_str!("../README.md")]

use std::array::TryFromSliceError;
use std::collections::BTreeMap;
use std::mem;
use std::num::TryFromIntError;

use futures::StreamExt;

use wasi::io::streams::StreamError;
use wasi::sockets::network::{self, IpSocketAddress};

use wasi_async::codec::{ChunksDecoder, FramedRead};
use wasi_async::io::{AsyncWrite, AsyncWriteExt};
use wasi_async::net::TcpStream;

use thiserror::Error;

use tracing::{debug, info, instrument, warn};

#[allow(warnings)]
mod bindings;

#[derive(Debug, PartialEq)]
pub enum Message {
    Insert { timestamp: i32, price: i32 },

    Query { mintime: i32, maxtime: i32 },
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("stream error {0}")]
    Stream(#[from] StreamError),

    #[error("tcp socket error {0}")]
    TcpSocket(#[from] network::ErrorCode),

    #[error("message invalid")]
    MessageInvalid,

    #[error("invalid slice: {0}")]
    Slice(#[from] TryFromSliceError),

    #[error("mean out of range: {0}")]
    Mean(#[from] TryFromIntError),
}

#[instrument(skip(stream))]
pub async fn run(address: IpSocketAddress, mut stream: TcpStream) -> Result<(), Error> {
    info!("run");

    let mut items = BTreeMap::new();

    let (read, mut write) = stream.split();
    let r = async move {
        let mut read = FramedRead::new(read, ChunksDecoder::<9>::new()).map(parse);

        while let Some(value) = read.next().await {
            match value? {
                Message::Insert { timestamp, price } => {
                    debug!("message: I {timestamp} {price}");
                    items.insert(timestamp, price);
                }

                Message::Query { mintime, maxtime } if maxtime >= mintime => {
                    let mut count = 0;
                    let mut sum = 0;
                    for (_, value) in items.range(mintime..=maxtime) {
                        count += 1;
                        sum += i64::from(*value);
                    }
                    let mean = if count > 0 { sum / count } else { 0 };
                    debug!("Q {mintime} {maxtime}: {mean}");
                    write
                        .write_all(i32::to_be_bytes(i32::try_from(mean)?).as_slice())
                        .await?;
                    write.flush().await?;
                }

                Message::Query { mintime, maxtime } => {
                    debug!("Q {mintime} {maxtime} invalid range");
                    write.write_all([0, 0, 0, 0].as_slice()).await?;
                    write.flush().await?;
                }
            }
        }

        Ok(())
    }
    .await;

    stream.close().await.ok();

    match r {
        Err(Error::Stream(StreamError::Closed)) | Ok(()) => Ok(()),
        Err(e) => Err(e),
    }
}

fn parse(chunk: Result<[u8; 9], StreamError>) -> Result<Message, Error> {
    let chunk = chunk?;

    match (
        chunk[0],
        i32::from_be_bytes(chunk[1..=mem::size_of::<i32>()].try_into()?),
        i32::from_be_bytes(chunk[1 + mem::size_of::<i32>()..].try_into()?),
    ) {
        (b'I', timestamp, price) => Ok(Message::Insert { timestamp, price }),
        (b'Q', mintime, maxtime) => Ok(Message::Query { mintime, maxtime }),
        _ => {
            warn!("invalid request");
            Err(Error::MessageInvalid)
        }
    }
}
