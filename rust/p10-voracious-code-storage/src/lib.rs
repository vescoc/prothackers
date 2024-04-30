#![doc = include_str!("../README.md")]

use std::future::Future;
use std::io;
use std::sync::Arc;

use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter,
};
use tokio::net::{TcpListener, TcpStream};

use tracing::{debug, info, instrument};

pub mod vcs;

use vcs::{ListEntry, Path, Vcs};

pub trait WriteLine: AsyncWriteExt + Unpin {
    fn write_line(&mut self, line: &[u8]) -> impl Future<Output = Result<(), io::Error>> {
        async move {
            self.write_all(line).await?;
            self.write_u8(b'\n').await?;
            self.flush().await
        }
    }
}

impl<T: AsyncWriteExt + Unpin> WriteLine for T {}

#[instrument(skip(listener))]
pub async fn run(listener: TcpListener) -> Result<(), io::Error> {
    let vcs = Arc::new(Vcs::new());

    loop {
        let (stream, remote_addr) = listener.accept().await?;

        info!("remote: {remote_addr:?}");

        tokio::spawn(handle_client(vcs.clone(), stream));
    }
}

async fn copy_n<'a, R, W>(
    reader: &'a mut R,
    writer: &'a mut W,
    mut size: usize,
) -> io::Result<usize>
where
    R: AsyncRead + Unpin + ?Sized,
    W: AsyncWrite + Unpin + ?Sized,
{
    let original_size = size;
    let mut written_size = 0;
    let mut buffer = [0; 4096];
    while size > 0 {
        let len = reader.read(&mut buffer[0..size.min(4096)]).await?;
        assert!(len > 0);
        writer.write_all(&buffer[0..len]).await?;
        size -= len;
        written_size += len;
    }

    writer.flush().await?;

    assert_eq!(written_size, original_size);

    Ok(written_size)
}

#[instrument(skip(stream, vcs))]
async fn handle_client(vcs: Arc<Vcs>, mut stream: TcpStream) -> Result<(), io::Error> {
    debug!("start");

    let (read, write) = stream.split();
    let mut reader = BufReader::new(read);
    let mut writer = BufWriter::new(write);

    loop {
        writer.write_line(b"READY").await?;

        let mut buffer = String::with_capacity(1024);
        if reader.read_line(&mut buffer).await? == 0 {
            break;
        }

        let buffer = buffer.trim();

        debug!("request: {buffer}");

        let mut parts = buffer.split_whitespace();
        if let Some(command) = parts.next() {
            if command.eq_ignore_ascii_case("HELP") {
                writer.write_line(b"OK usage: HELP|GET|PUT|LIST").await?;
            } else if command.eq_ignore_ascii_case("LIST") {
                match (parts.next(), parts.next()) {
                    (Some(dir), None) => {
                        if let Ok(list) = vcs.list(dir) {
                            writer
                                .write_line(format!("OK {}", list.len()).as_bytes())
                                .await?;
                            for entry in list {
                                match entry {
                                    ListEntry::Dir(name, _) => {
                                        writer
                                            .write_line(format!("{name}/ DIR").as_bytes())
                                            .await?;
                                    }
                                    ListEntry::File(name, releases) => {
                                        writer
                                            .write_line(format!("{name} r{releases}").as_bytes())
                                            .await?;
                                    }
                                }
                            }
                        } else {
                            writer.write_line(b"ERR no such file").await?;
                        }
                    }

                    _ => writer.write_line(b"ERR usage: LIST dir").await?,
                }
            } else if command.eq_ignore_ascii_case("PUT") {
                match (parts.next(), parts.next(), parts.next()) {
                    (Some(path), Some(size), None) => {
                        if let Ok(size) = size.parse::<usize>() {
                            if let Ok(mut write) = vcs.put(path) {
                                copy_n(&mut reader, &mut write, size).await?;
                                if let Ok(revision) = write.commit() {
                                    debug!("PUT {path} {revision} -> {size}");
                                    writer
                                        .write_line(format!("OK r{}", revision + 1).as_bytes())
                                        .await?;
                                } else {
                                    debug!("PUT invalid file {path} {size} <commit>");
                                    writer.write_line(b"ERR invalid file").await?;
                                }
                            } else {
                                debug!("PUT invalid file {path} {size} <put>");
                                writer.write_line(b"ERR invalid file").await?;
                            }
                        } else {
                            debug!("PUT invalid size {path} {size}");
                            writer.write_line(b"ERR invalid size").await?;
                        }
                    }

                    (Some(path), None, None) => {
                        if Path::new(path.as_bytes()).is_ok() {
                            writer
                                .write_line(b"ERR usage: PUT file length newline data")
                                .await?;
                        } else {
                            writer.write_line(b"ERR illegal file name").await?;
                        }
                    }

                    _ => {
                        writer
                            .write_line(b"ERR usage: PUT file length newline data")
                            .await?;
                    }
                }
            } else if command.eq_ignore_ascii_case("GET") {
                match (parts.next(), parts.next(), parts.next()) {
                    (Some(path), Some(revision), None) => {
                        if let Some(revision) = revision.strip_prefix('r') {
                            if let Ok(revision) = revision.parse::<usize>() {
                                if revision > 0 {
                                    let revision = revision - 1;
                                    if let Ok(mut read) = vcs.get_revision(path, revision) {
                                        let len = read.len();
                                        debug!("GET {path} {revision} -> {len}");
                                        writer.write_line(format!("OK {len}").as_bytes()).await?;
                                        copy_n(&mut read, &mut writer, len).await?;
                                    } else {
                                        debug!("GET no such file {path} {revision}");
                                        writer.write_line(b"ERR no such file").await?;
                                    }
                                } else {
                                    debug!("GET invalid revision spec {path} {revision}");
                                    writer.write_line(b"ERR invalid revision spec").await?;
                                }
                            } else {
                                debug!("GET invalid revision spec {path} {revision}");
                                writer.write_line(b"ERR invalid revision spec").await?;
                            }
                        } else {
                            debug!("GET invalid revision spec {path} {revision}");
                            writer.write_line(b"ERR invalid revision spec").await?;
                        }
                    }

                    (Some(path), None, None) => {
                        if let Ok(mut read) = vcs.get_current_revision(path) {
                            let len = read.len();
                            writer.write_line(format!("OK {len}").as_bytes()).await?;
                            copy_n(&mut read, &mut writer, len).await?;
                        } else {
                            writer.write_line(b"ERR no such file").await?;
                        }
                    }

                    _ => writer.write_line(b"ERR usage: GET file [revision]").await?,
                }
            } else {
                debug!("illegal method: {command}");
                writer
                    .write_line(format!("ERR illegal method: {command}").as_bytes())
                    .await?;
            }
        }
    }

    debug!("done");

    Ok(())
}
