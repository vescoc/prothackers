use std::ops::ControlFlow;

use bytes::BytesMut;

use crate::codec::{packets, Error, Parser, RawPacketDecoder, Validator};

#[derive(Debug, PartialEq)]
pub struct Packet<S> {
    pub protocol: S,
    pub version: u32,
}

#[derive(Debug, PartialEq)]
pub struct PacketDecoder;

impl RawPacketDecoder for PacketDecoder {
    type Decoded<'a> = Packet<&'a str>;

    fn decode(data: &[u8]) -> Self::Decoded<'_> {
        let mut parser = Parser::new(data);

        parser.read_u8();
        parser.read_u32();
        let protocol = parser.read_str();
        let version = parser.read_u32();

        Packet { protocol, version }
    }
}

pub(crate) fn read_packet(src: &mut BytesMut) -> Result<Option<packets::Packet>, Error> {
    let mut validator = Validator::new(src);

    if let ControlFlow::Break(b) = validator.validate_type() {
        return b;
    }

    if let ControlFlow::Break(b) = validator.validate_length() {
        return b;
    }

    if let ControlFlow::Break(b) = validator.validate_str() {
        return b;
    }

    if let ControlFlow::Break(b) = validator.validate_u32() {
        return b;
    }

    if let ControlFlow::Break(b) = validator.validate_checksum() {
        return b;
    }

    let raw_packet = validator.raw_packet()?;

    Ok(Some(packets::Packet::Hello(raw_packet)))
}

#[cfg(test)]
mod tests {
    use futures::TryStreamExt;

    use tokio_util::codec::FramedRead;

    use crate::codec::packets::PacketDecoder;
    use crate::tests::init_tracing_subscriber;

    use super::*;

    #[tokio::test]
    async fn test_packet() {
        init_tracing_subscriber();

        let data = [
            0x50, 0x00, 0x00, 0x00, 0x19, 0x00, 0x00, 0x00, 0x0b, 0x70, 0x65, 0x73, 0x74, 0x63,
            0x6f, 0x6e, 0x74, 0x72, 0x6f, 0x6c, 0x00, 0x00, 0x00, 0x01, 0xce,
        ]
        .as_slice();
        let mut reader = FramedRead::new(data, PacketDecoder::new());

        let packets::Packet::Hello(raw_packet) = reader.try_next().await.unwrap().unwrap() else {
            panic!("invalid packet");
        };

        assert_eq!(
            Packet {
                protocol: "pestcontrol",
                version: 1
            },
            raw_packet.decode()
        );
    }
}
