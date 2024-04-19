use std::io;

use tracing::{debug, instrument};

use tokio_util::codec::{Decoder, Encoder};

use bytes::{Buf, BufMut, BytesMut};

use thiserror::Error;

const CHECK_PHRASE: &[u8] = b"hello";

#[derive(Debug, PartialEq)]
pub enum Codec {
    Incomplete(Vec<Operation>),
    Complete(Spec, usize, usize),
    SpecError(SpecError),
}

impl Codec {
    #[must_use]
    pub fn new() -> Self {
        Self::Incomplete(vec![])
    }
}

impl Default for Codec {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for Codec {
    type Item = u8;
    type Error = SpecError;

    #[instrument(skip(src))]
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        loop {
            match self {
                Codec::SpecError(err) => {
                    return Err(SpecError::Previous(err.to_string()));
                }
                Codec::Incomplete(operations) => {
                    if src.is_empty() {
                        src.reserve(1);
                        return Ok(None);
                    }

                    let mut iter = src[0..src.len()].iter().enumerate();
                    while let Some((advance, b)) = iter.next() {
                        match b {
                            0x00 => {
                                src.advance(advance + 1);
                                let spec = Spec(operations.clone());
                                if spec.is_valid() {
                                    *self = Codec::Complete(spec, 0, 0);
                                    debug!("got codec: {self:?}");
                                    break;
                                }

                                let operations = operations.clone();
                                *self = Codec::SpecError(SpecError::Invalid(operations.clone()));
                                return Err(SpecError::Invalid(operations));
                            }
                            0x01 => {
                                operations.push(Operation::Reversebits);
                            }
                            0x02 => {
                                if let Some((_, b)) = iter.next() {
                                    operations.push(Operation::Xor(*b));
                                } else {
                                    src.advance(advance + 1);
                                    src.reserve(2);
                                    return Ok(None);
                                }
                            }
                            0x03 => operations.push(Operation::Xorpos),
                            0x04 => {
                                if let Some((_, b)) = iter.next() {
                                    operations.push(Operation::Add(*b));
                                } else {
                                    src.advance(advance + 1);
                                    src.reserve(2);
                                    return Ok(None);
                                }
                            }
                            0x05 => operations.push(Operation::Addpos),
                            c => {
                                let c = *c;
                                src.advance(advance + 1);
                                *self = Codec::SpecError(SpecError::InvalidOpcode(c));
                                return Err(SpecError::InvalidOpcode(c));
                            }
                        }
                    }
                }
                Codec::Complete(spec, upstream_position, _) => {
                    if src.remaining() > 0 {
                        let r = Ok(Some(spec.decode(*upstream_position, src.get_u8())));
                        *upstream_position += 1;
                        return r;
                    }

                    src.reserve(1);
                    return Ok(None);
                }
            }
        }
    }
}

impl Encoder<u8> for Codec {
    type Error = SpecError;

    #[instrument(skip(dst))]
    fn encode(&mut self, value: u8, dst: &mut BytesMut) -> Result<(), Self::Error> {
        match self {
            Codec::SpecError(err) => Err(SpecError::Previous(err.to_string())),
            Codec::Incomplete(_) => Err(SpecError::Previous("incomplete".to_string())),
            Codec::Complete(spec, _, downstream_position) => {
                dst.reserve(1);
                dst.put_u8(spec.encode(*downstream_position, value));
                *downstream_position += 1;
                Ok(())
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Spec(Vec<Operation>);

#[derive(Debug, PartialEq, Clone)]
pub enum Operation {
    Reversebits,
    Xor(u8),
    Xorpos,
    Add(u8),
    Addpos,
}

impl Operation {
    #[allow(clippy::cast_possible_truncation)]
    fn encode(&self, position: usize, value: u8) -> u8 {
        match self {
            Operation::Reversebits => value.reverse_bits(),
            Operation::Xor(n) => value ^ n,
            Operation::Xorpos => ((position ^ usize::from(value)) % 256) as u8,
            Operation::Add(n) => value.wrapping_add(*n),
            Operation::Addpos => ((position + usize::from(value)) % 256) as u8,
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn decode(&self, position: usize, value: u8) -> u8 {
        match self {
            Operation::Reversebits => value.reverse_bits(),
            Operation::Xor(n) => value ^ n,
            Operation::Xorpos => ((position ^ usize::from(value)) % 256) as u8,
            Operation::Add(n) => value.wrapping_sub(*n),
            Operation::Addpos => ((usize::from(value).wrapping_sub(position)) % 256) as u8,
        }
    }
}

#[derive(Error, Debug)]
pub enum SpecError {
    #[error("End of Stream")]
    EOS,

    #[error("Invalid opcode {0}")]
    InvalidOpcode(u8),

    #[error("Invalid spec {0:?}")]
    Invalid(Vec<Operation>),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("previous error: {0}")]
    Previous(String),
}

impl PartialEq for SpecError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (SpecError::EOS, SpecError::EOS) => true,
            (SpecError::InvalidOpcode(o1), SpecError::InvalidOpcode(o2)) => o1 == o2,
            (SpecError::Invalid(v1), SpecError::Invalid(v2)) => v1 == v2,
            (SpecError::Io(err1), SpecError::Io(err2)) => err1.kind() == err2.kind(),
            (SpecError::Previous(str1), SpecError::Previous(str2)) => str1 == str2,
            _ => false,
        }
    }
}

impl Spec {
    fn is_valid(&self) -> bool {
        CHECK_PHRASE
            != CHECK_PHRASE
                .iter()
                .enumerate()
                .map(|(position, &value)| self.encode(position, value))
                .collect::<Vec<u8>>()
    }

    #[must_use]
    pub fn encode(&self, position: usize, mut value: u8) -> u8 {
        for operation in &self.0 {
            value = operation.encode(position, value);
        }
        value
    }

    #[must_use]
    pub fn decode(&self, position: usize, mut value: u8) -> u8 {
        for operation in self.0.iter().rev() {
            value = operation.decode(position, value);
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use std::pin::{pin, Pin};
    use std::task::{Context, Poll};

    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio_stream::StreamExt;
    use tokio_util::codec::{Framed, FramedRead};

    use futures::stream;
    use futures::SinkExt;

    use parking_lot::Once;

    use tracing::Instrument;

    use super::*;

    fn init_tracing_subscriber() {
        static SUBSCRIBER: Once = Once::new();
        SUBSCRIBER.call_once(tracing_subscriber::fmt::init);
    }

    struct StreamSink<W, R>(W, R);

    impl<W: AsyncWrite + Unpin, R: Unpin> AsyncWrite for StreamSink<W, R> {
        fn poll_write(
            mut self: Pin<&mut Self>,
            ctx: &mut Context,
            data: &[u8],
        ) -> Poll<Result<usize, io::Error>> {
            pin!(&mut self.as_mut().0).poll_write(ctx, data)
        }

        fn poll_flush(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Result<(), io::Error>> {
            pin!(&mut self.as_mut().0).poll_flush(ctx)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            ctx: &mut Context,
        ) -> Poll<Result<(), io::Error>> {
            pin!(&mut self.as_mut().0).poll_shutdown(ctx)
        }
    }

    impl<W: Unpin, R: AsyncRead + Unpin> AsyncRead for StreamSink<W, R> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            ctx: &mut Context,
            src: &mut ReadBuf,
        ) -> Poll<Result<(), io::Error>> {
            pin!(&mut self.as_mut().1).poll_read(ctx, src)
        }
    }

    #[tokio::test]
    async fn test_parse() {
        let mut stream = FramedRead::new([0x02, 0x01, 0x01, 0x00].as_slice(), Codec::new());

        assert_eq!(stream.next().await, None);

        assert_eq!(
            &Codec::Complete(
                Spec(vec![Operation::Xor(0x01), Operation::Reversebits]),
                0,
                0
            ),
            stream.decoder()
        );
    }

    #[tokio::test]
    async fn test_noop_ciphers() {
        let mut stream = FramedRead::new([0x00].as_slice(), Codec::new());
        assert_eq!(stream.try_next().await, Err(SpecError::Invalid(vec![])));

        let mut stream = FramedRead::new([0x02, 0x00, 0x00].as_slice(), Codec::new());
        assert_eq!(
            stream.try_next().await,
            Err(SpecError::Invalid(vec![Operation::Xor(0)]))
        );

        let mut stream = FramedRead::new([0x02, 0xab, 0x02, 0xab, 0x00].as_slice(), Codec::new());
        assert_eq!(
            stream.try_next().await,
            Err(SpecError::Invalid(vec![
                Operation::Xor(0xab),
                Operation::Xor(0xab)
            ]))
        );

        let mut stream = FramedRead::new([0x01, 0x01, 0x00].as_slice(), Codec::new());
        assert_eq!(
            stream.try_next().await,
            Err(SpecError::Invalid(vec![
                Operation::Reversebits,
                Operation::Reversebits
            ]))
        );

        let mut stream = FramedRead::new(
            [0x02, 0xa0, 0x02, 0x0b, 0x02, 0xab, 0x00].as_slice(),
            Codec::new(),
        );
        assert_eq!(
            stream.try_next().await,
            Err(SpecError::Invalid(vec![
                Operation::Xor(0xa0),
                Operation::Xor(0x0b),
                Operation::Xor(0xab)
            ]))
        );
    }

    #[tokio::test]
    async fn test_encode_1() {
        init_tracing_subscriber();

        let mut io = StreamSink(vec![], [0x02, 0x01, 0x01, 0x00].as_slice());
        let (mut sink, stream) = futures::StreamExt::split(Framed::new(&mut io, Codec::new()));

        stream.collect::<Result<Vec<_>, _>>().await.unwrap();

        sink.send_all(&mut stream::iter([0x68, 0x65, 0x6c, 0x6c, 0x6f]).map(Result::Ok))
            .await
            .unwrap();

        drop(sink);

        assert_eq!(vec![0x96, 0x26, 0xb6, 0xb6, 0x76], io.0);
    }

    #[tokio::test]
    async fn test_encode_2() {
        init_tracing_subscriber();

        let mut io = StreamSink(vec![], [0x05, 0x05, 0x00].as_slice());

        let (mut sink, stream) = futures::StreamExt::split(Framed::new(&mut io, Codec::new()));

        stream.collect::<Result<Vec<_>, _>>().await.unwrap();

        sink.send_all(&mut stream::iter([0x68, 0x65, 0x6c, 0x6c, 0x6f]).map(Result::Ok))
            .await
            .unwrap();

        drop(sink);

        assert_eq!(vec![0x68, 0x67, 0x70, 0x72, 0x77], io.0);
    }

    #[tokio::test]
    async fn test_decode_1() {
        init_tracing_subscriber();

        async move {
            let stream = FramedRead::new(
                [
                    0x02, 0x7b, 0x05, 0x01, 0x00, 0xf2, 0x20, 0xba, 0x44, 0x18, 0x84, 0xba, 0xaa,
                    0xd0, 0x26, 0x44, 0xa4, 0xa8, 0x7e,
                ]
                .as_slice(),
                Codec::new(),
            );

            assert_eq!(
                b"4x dog,5x car\n".as_slice(),
                stream.collect::<Result<Vec<_>, _>>().await.unwrap()
            );
        }
        .instrument(tracing::info_span!("test_decode_1"))
        .await;
    }

    #[tokio::test]
    async fn test_decode_2() {
        init_tracing_subscriber();

        async move {
            let stream = FramedRead::new(
                [
                    0x02, 0x7b, 0x05, 0x01, 0x00, 0xf2, 0x20, 0xba, 0x44, 0x18, 0x84, 0xba, 0xaa,
                    0xd0, 0x26, 0x44, 0xa4, 0xa8, 0x7e, 0x6a, 0x48, 0xd6, 0x58, 0x34, 0x44, 0xd6,
                    0x7a, 0x98, 0x4e, 0x0c, 0xcc, 0x94, 0x31,
                ]
                .as_slice(),
                Codec::new(),
            );

            assert_eq!(
                b"3x rat,2x cat\n".as_slice(),
                stream
                    .skip(14)
                    .collect::<Result<Vec<_>, _>>()
                    .await
                    .unwrap()
            );
        }
        .instrument(tracing::info_span!("test_decode_2"))
        .await;
    }
}
