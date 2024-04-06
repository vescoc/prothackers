use tracing::debug;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// A simple echo.
///
/// # Errors
/// * Error when the under socket returns an error.
#[tracing::instrument(skip(stream))]
pub async fn echo(mut stream: TcpStream) -> Result<(), anyhow::Error> {
    debug!("start");

    let (mut read_half, mut write_half) = stream.split();

    let mut buffer = [0; 1024];
    loop {
        match read_half.read(&mut buffer).await {
            Ok(0) => break,
            Ok(n) => write_half.write_all(&buffer[..n]).await?,
            Err(err) => return Err(err.into()),
        }
    }

    write_half.flush().await?;
    write_half.shutdown().await?;

    debug!("end");

    Ok(())
}
