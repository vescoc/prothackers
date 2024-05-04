use std::io;
use std::marker::PhantomData;
use std::ops::ControlFlow;

use bytes::{Bytes, BytesMut};

use thiserror::Error;

pub mod packets;

#[derive(Error, Debug)]
pub enum Error {
    #[error("unknown packet: 0x{0:02x}")]
    UnknownPacket(u8),

    #[error("invalid packet")]
    InvalidPacket,

    #[error("invalid checksum")]
    InvalidChecksum,

    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

struct Parser<'a>(&'a [u8]);

impl<'a> Parser<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self(data)
    }

    fn read_u8(&mut self) -> u8 {
        let (r, rem) = unsafe { self.0.split_first().unwrap_unchecked() };
        self.0 = rem;
        *r
    }

    fn read_u32(&mut self) -> u32 {
        let r = u32::from_be_bytes([self.0[0], self.0[1], self.0[2], self.0[3]]);
        self.0 = &self.0[4..];
        r
    }

    fn read_str(&mut self) -> &'a str {
        let len = u32::from_be_bytes([self.0[0], self.0[1], self.0[2], self.0[3]]) as usize;

        let r = unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.0[4..].as_ptr(), len))
        };
        self.0 = &self.0[len + 4..];

        r
    }
}

pub trait RawPacketDecoder {
    type Decoded<'a>;

    fn decode(data: &[u8]) -> Self::Decoded<'_>;
}

#[derive(Debug, PartialEq)]
pub struct RawPacket<D> {
    data: Bytes,
    _marker: PhantomData<D>,
}

impl<D> RawPacket<D> {
    fn new(data: Bytes) -> Self {
        Self {
            data,
            _marker: PhantomData,
        }
    }
}

impl<D: RawPacketDecoder> RawPacket<D> {
    pub fn decode(&self) -> D::Decoded<'_> {
        D::decode(self.data.as_ref())
    }
}

struct Validator<'a> {
    data: &'a mut BytesMut,
    cursor: usize,
    length: Option<usize>,
}

impl<'a> Validator<'a> {
    fn new(data: &'a mut BytesMut) -> Self {
        Self {
            data,
            cursor: 0,
            length: None,
        }
    }

    fn validate_u8<P>(&mut self) -> ControlFlow<Result<Option<P>, Error>, u8> {
        if let Some(length) = self.length {
            if self.cursor + 1 > length {
                return ControlFlow::Break(Err(Error::InvalidPacket));
            }
        }

        if self.data.len() > self.cursor {
            let r = self.data[self.cursor];
            self.cursor += 1;
            ControlFlow::Continue(r)
        } else {
            self.data
                .reserve((self.data.capacity() - self.data.len()).min(1));
            ControlFlow::Break(Ok(None))
        }
    }

    fn validate_type<P>(&mut self) -> ControlFlow<Result<Option<P>, Error>, u8> {
        self.validate_u8()
    }

    fn validate_length<P>(&mut self) -> ControlFlow<Result<Option<P>, Error>, usize> {
        let length = self.validate_u32()? as usize;
        self.length = Some(length);

        ControlFlow::Continue(length)
    }

    fn validate_str<P>(&mut self) -> ControlFlow<Result<Option<P>, Error>, &str> {
        let len = self.validate_u32()? as usize;

        if let Some(length) = self.length {
            if self.cursor + len > length {
                return ControlFlow::Break(Err(Error::InvalidPacket));
            }
        }

        if self.data.len() > self.cursor + len {
            let r = unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                    self.data.as_ptr().add(self.cursor),
                    len,
                ))
            };
            self.cursor += len;
            ControlFlow::Continue(r)
        } else {
            self.data
                .reserve((self.data.capacity() - self.data.len()).min(len));
            ControlFlow::Break(Ok(None))
        }
    }

    fn validate_u32<P>(&mut self) -> ControlFlow<Result<Option<P>, Error>, u32> {
        if let Some(length) = self.length {
            if self.cursor + 4 > length {
                return ControlFlow::Break(Err(Error::InvalidPacket));
            }
        }

        if self.data.len() > self.cursor + 4 {
            let r = u32::from_be_bytes([
                self.data[self.cursor],
                self.data[self.cursor + 1],
                self.data[self.cursor + 2],
                self.data[self.cursor + 3],
            ]);
            self.cursor += 4;
            ControlFlow::Continue(r)
        } else {
            self.data
                .reserve((self.data.capacity() - self.data.len()).min(4));
            ControlFlow::Break(Ok(None))
        }
    }

    fn validate_checksum<P>(&mut self) -> ControlFlow<Result<Option<P>, Error>, u8> {
        let checksum = self.validate_u8()?;
        if self
            .data
            .iter()
            .take(unsafe { self.length.unwrap_unchecked() })
            .fold(0_u8, |a, b| a.wrapping_add(*b))
            != 0
        {
            return ControlFlow::Break(Err(Error::InvalidChecksum));
        }
        ControlFlow::Continue(checksum)
    }

    fn raw_packet<D: RawPacketDecoder>(&mut self) -> Result<RawPacket<D>, Error> {
        if unsafe { self.length.unwrap_unchecked() } != self.cursor {
            return Err(Error::InvalidPacket);
        }

        let bytes = self
            .data
            .split_to(unsafe { self.length.unwrap_unchecked() })
            .freeze();

        Ok(RawPacket::new(bytes))
    }
}
