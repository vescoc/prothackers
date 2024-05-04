use std::ops::ControlFlow;

use bytes::BytesMut;

use crate::codec::{packets, Error, Parser, RawPacketDecoder, Validator};

#[derive(Debug, PartialEq)]
pub enum PolicyAction {
    Conserve,
    Cull,
}

#[derive(Debug, PartialEq)]
pub struct Packet<S> {
    pub species: S,
    pub action: PolicyAction,
}

#[derive(Debug, PartialEq)]
pub struct PacketDecoder;

impl RawPacketDecoder for PacketDecoder {
    type Decoded<'a> = Packet<&'a str>;

    fn decode(data: &[u8]) -> Self::Decoded<'_> {
        let mut parser = Parser::new(data);

        parser.read_u8();
        parser.read_u32();
        let species = parser.read_str();
        let action = if parser.read_u8() == 0x90 {
            PolicyAction::Cull
        } else {
            PolicyAction::Conserve
        };

        Packet { species, action }
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

    // species
    if let ControlFlow::Break(b) = validator.validate_str() {
        return b;
    }

    // action
    match validator.validate_u8() {
        ControlFlow::Break(b) => return b,
        ControlFlow::Continue(0x90 | 0xa0) => {}
        ControlFlow::Continue(_) => return Err(Error::InvalidPacket),
    }

    if let ControlFlow::Break(b) = validator.validate_checksum() {
        return b;
    }

    let raw_packet = validator.raw_packet()?;

    Ok(Some(packets::Packet::CreatePolicy(raw_packet)))
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
            0x55, 0x00, 0x00, 0x00, 0x0e, 0x00, 0x00, 0x00, 0x03, 0x64, 0x6f, 0x67, 0xa0, 0xc0,
        ]
        .as_slice();
        let mut reader = FramedRead::new(data, PacketDecoder::new());

        let packets::Packet::CreatePolicy(raw_packet) = reader.try_next().await.unwrap().unwrap()
        else {
            panic!("invalid packet");
        };

        assert_eq!(
            Packet {
                species: "dog",
                action: PolicyAction::Conserve
            },
            raw_packet.decode()
        );
    }
}
