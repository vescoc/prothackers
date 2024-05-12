#![doc = include_str!("../README.md")]

use wasi::io::streams::StreamError;
use wasi::sockets::network::{ErrorCode, IpSocketAddress};

use thiserror::Error;
use tracing::{info, instrument};

use wasi_async::net::TcpStream;

#[allow(warnings)]
mod bindings;

#[derive(Error, Debug)]
pub enum Error {
    #[error("stream error: {0}")]
    Stream(#[from] StreamError),

    #[error("tcp socket error: {0}")]
    TcpSocket(#[from] ErrorCode),
}

#[instrument(skip(stream))]
pub async fn run(address: IpSocketAddress, mut stream: TcpStream) -> Result<(), Error> {
    info!("run: {address:?}");

    let (mut read, mut write) = stream.split();
    loop {
        let data = read.read(1024).await?;
        if data.is_empty() {
            break;
        }

        write.write_all(&data).await?;
        write.flush().await?;
    }

    Ok(stream.close().await?)
}
