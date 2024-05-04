use std::ops::ControlFlow;

use bytes::BytesMut;

use crate::codec::{packets, Error, Parser, RawPacketDecoder, Validator, Writer};

#[derive(Debug, PartialEq)]
pub struct Packet;

impl Packet {
    #[allow(clippy::unused_self)]
    pub(crate) fn write_packet(&self) -> Vec<u8> {
        let writer = Writer::new(0x52);

        writer.finalize()
    }
}

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

    let raw_packet = validator.raw_packet::<PacketDecoder>()?;

    Ok(Some(packets::Packet::Ok(raw_packet.decode())))
}

#[cfg(test)]
mod tests {
    use futures::{SinkExt, TryStreamExt};

    use tokio_util::codec::{FramedRead, FramedWrite};

    use crate::codec::packets::PacketCodec;
    use crate::tests::init_tracing_subscriber;

    use super::*;

    #[tokio::test]
    async fn test_read() {
        init_tracing_subscriber();

        let data = [0x52, 0x00, 0x00, 0x00, 0x06, 0xa8].as_slice();
        let mut reader = FramedRead::new(data, PacketCodec::new());

        let packets::Packet::Ok(raw_packet) = reader.try_next().await.unwrap().unwrap() else {
            panic!("invalid packet");
        };

        assert_eq!(Packet, raw_packet);
    }

    #[tokio::test]
    async fn test_write() {
        init_tracing_subscriber();

        let mut buffer = vec![];
        {
            let mut writer = FramedWrite::new(&mut buffer, PacketCodec::new());

            writer.send(packets::Packet::Ok(Packet)).await.unwrap();
        }

        let data = [0x52, 0x00, 0x00, 0x00, 0x06, 0xa8].as_slice();

        assert_eq!(data, buffer);
    }
}
