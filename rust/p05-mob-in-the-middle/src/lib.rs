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
