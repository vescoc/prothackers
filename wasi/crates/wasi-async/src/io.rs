use std::future::Future;

use wasi::io::streams::StreamError;

pub trait AsyncRead {
    fn read(&mut self, len: u64) -> impl Future<Output = Result<Vec<u8>, StreamError>>;
}

pub trait AsyncWrite {
    fn write(&mut self, data: &[u8]) -> impl Future<Output = Result<u64, StreamError>>;

    fn flush(&mut self) -> impl Future<Output = Result<(), StreamError>>;

    fn close(&mut self) -> impl Future<Output = Result<(), StreamError>>;
}

pub trait AsyncWriteExt: AsyncWrite {
    fn write_all(&mut self, mut data: &[u8]) -> impl Future<Output = Result<(), StreamError>> {
        async move {
            while !data.is_empty() {
                let len = self.write(data).await?;
                assert!(len > 0, "write_all len zero");

                if len as usize == data.len() {
                    break;
                }

                data = &data[len as usize..];
            }

            Ok(())
        }
    }
}

impl<T: AsyncWrite> AsyncWriteExt for T {}
