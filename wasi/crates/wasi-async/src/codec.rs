use std::future::Future;
use std::pin::{pin, Pin};
use std::task::{Context, Poll};

use futures::Stream;

use tracing::{error, instrument, trace};

use wasi::io::streams::StreamError;

use bytes::BytesMut;

use crate::io::AsyncRead;

pub trait Decoder {
    type Item;
    type Error: From<StreamError>;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error>;

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.decode(src) {
            Ok(Some(value)) => Ok(Some(value)),
            Ok(None) => Err(StreamError::Closed.into()),
            Err(e) => Err(e),
        }
    }
}

pub struct FramedRead<R, D> {
    read: R,
    decoder: D,
    buffer: BytesMut,
    eof: bool,
}

impl<R, D> FramedRead<R, D> {
    pub fn new(read: R, decoder: D) -> Self {
        Self {
            read,
            decoder,
            buffer: BytesMut::new(),
            eof: false,
        }
    }
}

impl<R: AsyncRead + Unpin, D: Decoder + Unpin> FramedRead<R, D>
where
    Self: Stream<Item = Result<D::Item, D::Error>>,
{
    #[instrument(skip_all)]
    fn handle_eof(&mut self) -> Poll<Option<<Self as Stream>::Item>> {
        if self.buffer.is_empty() {
            trace!("buffer is empty");
            return Poll::Ready(None);
        }

        match self.decoder.decode_eof(&mut self.buffer) {
            Ok(None) => {
                error!("decoder eof returned Ok(None)");
                return Poll::Ready(Some(Err(StreamError::Closed.into())));
            }
            Ok(Some(v)) => {
                trace!("some data");
                return Poll::Ready(Some(Ok(v)));
            }
            Err(e) => {
                trace!("error");
                return Poll::Ready(Some(Err(e)));
            }
        }
    }
}

impl<R: AsyncRead + Unpin, D: Decoder + Unpin> Stream for FramedRead<R, D> {
    type Item = Result<D::Item, D::Error>;

    #[instrument(skip_all)]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if this.eof {
            return this.handle_eof();
        }

        match this.decoder.decode(&mut this.buffer) {
            Ok(Some(value)) => return Poll::Ready(Some(Ok(value))),
            Err(e) => return Poll::Ready(Some(Err(e))),
            Ok(None) => {}
        }

        let read = &mut this.read;
        let len = this.buffer.capacity().max(1) as u64;
        let (data, eof) = {
            let f = pin!(read.read(len));
            match f.poll(cx) {
                Poll::Ready(Ok(data)) => (Some(data), false),
                Poll::Ready(Err(StreamError::Closed)) => (None, true),
                Poll::Ready(Err(e)) => return Poll::Ready(Some(Err(e.into()))),
                Poll::Pending => return Poll::Pending,
            }
        };

        if eof {
            this.eof = true;
            return this.handle_eof();
        }

        let data = data.unwrap();

        trace!("extend slice {}", data.len());
        this.buffer.extend_from_slice(&data);

        match this.decoder.decode(&mut this.buffer) {
            Ok(None) => Poll::Pending,
            Ok(Some(value)) => Poll::Ready(Some(Ok(value))),
            Err(e) => Poll::Ready(Some(Err(e))),
        }
    }
}

#[derive(Debug)]
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

    #[instrument]
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if let Some(index) = src.iter().skip(self.from_index).position(|v| *v == b'\n') {
            trace!("found new line at {index}");

            let line = src.split_to(self.from_index + index + 1);

            let mut v = Vec::with_capacity(line.len() - 1);
            v.extend_from_slice(&line[..line.len() - 1]);

            self.from_index = 0;

            Ok(Some(v))
        } else {
            self.from_index = src.len();
            src.reserve(1);

            trace!("searching");

            Ok(None)
        }
    }
}

#[derive(Debug)]
pub struct ChunksDecoder<const SIZE: usize>;

impl<const SIZE: usize> ChunksDecoder<SIZE> {
    pub fn new() -> Self {
        Self
    }
}

impl<const SIZE: usize> Default for ChunksDecoder<SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SIZE: usize> Decoder for ChunksDecoder<SIZE> {
    type Item = [u8; SIZE];
    type Error = StreamError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < SIZE {
            Ok(None)
        } else {
            let chunk = src.split_to(SIZE);
            Ok(Some(chunk[..].try_into().unwrap()))
        }
    }
}
