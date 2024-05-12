use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;

use wasi::io::streams::StreamError;

use bytes::BytesMut;

use crate::io::AsyncRead;

pub trait Decoder {
    type Item;
    type Error: From<StreamError>;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error>;
}

pub struct FramedRead<R, D> {
    read: R,
    decoder: D,
    buffer: BytesMut,
}

impl<R, D> FramedRead<R, D> {
    pub fn new(read: R, decoder: D) -> Self {
        Self {
            read,
            decoder,
            buffer: BytesMut::with_capacity(1024),
        }
    }
}

impl<R: AsyncRead + Unpin, D: Decoder + Unpin> Stream for FramedRead<R, D> {
    type Item = Result<D::Item, D::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let read = &mut this.read;
        let f = std::pin::pin!(read.read(this.buffer.capacity().max(1) as u64));
        let data = match f.poll(cx) {
            Poll::Ready(Ok(data)) => data,
            Poll::Ready(Err(StreamError::Closed)) => return Poll::Ready(None),
            Poll::Ready(Err(e)) => return Poll::Ready(Some(Err(e.into()))),
            Poll::Pending => return Poll::Pending,
        };

        this.buffer.extend_from_slice(&data);

        match this.decoder.decode(&mut this.buffer) {
            Ok(None) => Poll::Pending,
            Ok(Some(value)) => Poll::Ready(Some(Ok(value))),
            Err(e) => Poll::Ready(Some(Err(e))),
        }
    }
}

pub struct LinesDecoder {
    from_index: usize,
}

impl LinesDecoder {
    pub fn new() -> Self {
        Self { from_index: 0 }
    }
}

impl Default for LinesDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for LinesDecoder {
    type Item = Vec<u8>;
    type Error = StreamError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if let Some(index) = src.iter().skip(self.from_index).position(|v| *v == b'\n') {
            let line = src.split_to(index + 1);
            let mut v = Vec::with_capacity(line.len() - 1);
            v.extend_from_slice(&line[..line.len() - 1]);
            self.from_index = 0;
            Ok(Some(v))
        } else {
            self.from_index = src.len();
            src.reserve(1);
            Ok(None)
        }
    }
}
