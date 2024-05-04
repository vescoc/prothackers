use std::ops::ControlFlow;

use bytes::BytesMut;

use crate::codec::{packets, Error, Parser, RawPacketDecoder, Validator, Writer};

#[derive(Debug, PartialEq)]
pub struct Packet {
    pub site: u32,
    pub populations: Vec<Population>,
}

impl Packet {
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn write_packet(&self) -> Vec<u8> {
        let mut writer = Writer::new(0x58);

        writer.write_u32(self.site);
        writer.write_u32(self.populations.len() as u32);
        for Population { species, count } in &self.populations {
            writer.write_str(species);
            writer.write_u32(*count);
        }

        writer.finalize()
    }
}

#[derive(Debug, PartialEq)]
pub struct Population {
    pub species: String,
    pub count: u32,
}

#[derive(Debug, PartialEq)]
pub struct PacketDecoder;

impl RawPacketDecoder for PacketDecoder {
    type Decoded<'a> = Packet;

    fn decode(data: &[u8]) -> Self::Decoded<'_> {
        let mut parser = Parser::new(data);

        parser.read_u8();
        parser.read_u32();
        let site = parser.read_u32();

        let len = parser.read_u32() as usize;
        let mut populations = Vec::with_capacity(len);
        for _ in 0..len {
            let species = parser.read_str().to_string();
            let count = parser.read_u32();

            populations.push(Population { species, count });
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

        // count
        if let ControlFlow::Break(b) = validator.validate_u32() {
            return b;
        }
    }

    if let ControlFlow::Break(b) = validator.validate_checksum() {
        return b;
    }

    let raw_packet = validator.raw_packet::<PacketDecoder>()?;

    Ok(Some(packets::Packet::SiteVisit(raw_packet.decode())))
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

        let data = [
            0x58, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x30, 0x39, 0x00, 0x00, 0x00, 0x02, 0x00,
            0x00, 0x00, 0x03, 0x64, 0x6f, 0x67, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x03,
            0x72, 0x61, 0x74, 0x00, 0x00, 0x00, 0x05, 0x8c,
        ]
        .as_slice();
        let mut reader = FramedRead::new(data, PacketCodec::new());

        let packets::Packet::SiteVisit(raw_packet) = reader.try_next().await.unwrap().unwrap()
        else {
            panic!("invalid packet");
        };

        assert_eq!(
            Packet {
                site: 12345,
                populations: vec![
                    Population {
                        species: "dog".to_string(),
                        count: 1,
                    },
                    Population {
                        species: "rat".to_string(),
                        count: 5,
                    },
                ],
            },
            raw_packet
        );
    }

    #[tokio::test]
    async fn test_write() {
        init_tracing_subscriber();

        let mut buffer = vec![];
        {
            let mut writer = FramedWrite::new(&mut buffer, PacketCodec::new());

            writer
                .send(packets::Packet::SiteVisit(Packet {
                    site: 12345,
                    populations: vec![
                        Population {
                            species: "dog".to_string(),
                            count: 1,
                        },
                        Population {
                            species: "rat".to_string(),
                            count: 5,
                        },
                    ],
                }))
                .await
                .unwrap();
        }

        let data = [
            0x58, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x30, 0x39, 0x00, 0x00, 0x00, 0x02, 0x00,
            0x00, 0x00, 0x03, 0x64, 0x6f, 0x67, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x03,
            0x72, 0x61, 0x74, 0x00, 0x00, 0x00, 0x05, 0x8c,
        ]
        .as_slice();

        assert_eq!(data, buffer);
    }
}
