use std::ops::ControlFlow;

use bytes::BytesMut;

use crate::codec::{packets, Error, Parser, RawPacketDecoder, Validator};

#[derive(Debug, PartialEq)]
pub struct Packet<S> {
    pub site: u32,
    pub populations: Vec<Population<S>>,
}

#[derive(Debug, PartialEq)]
pub struct Population<S> {
    pub species: S,
    min: u32,
    max: u32,
}

#[derive(Debug, PartialEq)]
pub struct PacketDecoder;

impl RawPacketDecoder for PacketDecoder {
    type Decoded<'a> = Packet<&'a str>;

    fn decode(data: &[u8]) -> Self::Decoded<'_> {
        let mut parser = Parser::new(data);

        parser.read_u8();
        parser.read_u32();
        let site = parser.read_u32();

        let len = parser.read_u32() as usize;
        let mut populations = Vec::with_capacity(len);
        for _ in 0..len {
            let species = parser.read_str();
            let min = parser.read_u32();
            let max = parser.read_u32();

            populations.push(Population { species, min, max });
        }

        Packet { site, populations }
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

    // site
    if let ControlFlow::Break(b) = validator.validate_u32() {
        return b;
    }

    // array length
    let len = match validator.validate_u32() {
        ControlFlow::Break(b) => return b,
        ControlFlow::Continue(len) => len,
    };

    for _ in 0..len {
        // species
        if let ControlFlow::Break(b) = validator.validate_str() {
            return b;
        }

        // min
        if let ControlFlow::Break(b) = validator.validate_u32() {
            return b;
        }

        // max
        if let ControlFlow::Break(b) = validator.validate_u32() {
            return b;
        }
    }

    if let ControlFlow::Break(b) = validator.validate_checksum() {
        return b;
    }

    let raw_packet = validator.raw_packet()?;

    Ok(Some(packets::Packet::TargetPopulations(raw_packet)))
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
            0x54, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x30, 0x39, 0x00, 0x00, 0x00, 0x02, 0x00,
            0x00, 0x00, 0x03, 0x64, 0x6f, 0x67, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x03,
            0x00, 0x00, 0x00, 0x03, 0x72, 0x61, 0x74, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x0a, 0x80,
        ]
        .as_slice();
        let mut reader = FramedRead::new(data, PacketDecoder::new());

        let packets::Packet::TargetPopulations(raw_packet) =
            reader.try_next().await.unwrap().unwrap()
        else {
            panic!("invalid packet");
        };

        assert_eq!(
            Packet {
                site: 12345,
                populations: vec![
                    Population {
                        species: "dog",
                        min: 1,
                        max: 3,
                    },
                    Population {
                        species: "rat",
                        min: 0,
                        max: 10,
                    },
                ],
            },
            raw_packet.decode()
        );
    }
}
