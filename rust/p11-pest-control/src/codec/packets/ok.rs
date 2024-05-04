use std::ops::ControlFlow;

use bytes::BytesMut;

use crate::codec::{packets, Error, Parser, RawPacketDecoder, Validator};

#[derive(Debug, PartialEq)]
pub struct Packet;

#[derive(Debug, PartialEq)]
pub struct PacketDecoder;

impl RawPacketDecoder for PacketDecoder {
    type Decoded<'a> = Packet;

    fn decode(data: &[u8]) -> Self::Decoded<'_> {
        let mut parser = Parser::new(data);

        parser.read_u8();
        parser.read_u32();

        Packet
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

    if let ControlFlow::Break(b) = validator.validate_checksum() {
        return b;
    }

    let raw_packet = validator.raw_packet()?;

    Ok(Some(packets::Packet::Ok(raw_packet)))
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

        let data = [0x52, 0x00, 0x00, 0x00, 0x06, 0xa8].as_slice();
        let mut reader = FramedRead::new(data, PacketDecoder::new());

        let packets::Packet::Ok(raw_packet) = reader.try_next().await.unwrap().unwrap() else {
            panic!("invalid packet");
        };

        assert_eq!(Packet, raw_packet.decode());
    }
}
