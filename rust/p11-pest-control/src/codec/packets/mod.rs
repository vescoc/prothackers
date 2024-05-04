use tokio_util::codec::{Decoder, Encoder};

use bytes::BytesMut;

use tracing::instrument;

use super::Error;

pub mod create_policy;
pub mod delete_policy;
pub mod dial_authority;
pub mod error;
pub mod hello;
pub mod ok;
pub mod policy_result;
pub mod site_visit;
pub mod target_populations;

#[derive(Debug, PartialEq)]
pub enum Packet {
    Hello(hello::Packet),
    Error(error::Packet),
    Ok(ok::Packet),
    DialAuthority(dial_authority::Packet),
    TargetPopulations(target_populations::Packet),
    CreatePolicy(create_policy::Packet),
    DeletePolicy(delete_policy::Packet),
    PolicyResult(policy_result::Packet),
    SiteVisit(site_visit::Packet),
}

pub struct PacketCodec;

impl PacketCodec {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for PacketCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for PacketCodec {
    type Item = Packet;
    type Error = Error;

    #[instrument(skip_all)]
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match src.first() {
            Some(0x50) => hello::read_packet(src),
            Some(0x51) => error::read_packet(src),
            Some(0x52) => ok::read_packet(src),
            Some(0x53) => dial_authority::read_packet(src),
            Some(0x54) => target_populations::read_packet(src),
            Some(0x55) => create_policy::read_packet(src),
            Some(0x56) => delete_policy::read_packet(src),
            Some(0x57) => policy_result::read_packet(src),
            Some(0x58) => site_visit::read_packet(src),
            Some(c) => Err(Error::UnknownPacket(*c)),
            None => Ok(None),
        }
    }
}

impl Encoder<Packet> for PacketCodec {
    type Error = Error;

    fn encode(&mut self, packet: Packet, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let raw_packet = match packet {
            Packet::Hello(packet) => packet.write_packet(),
            Packet::Error(packet) => packet.write_packet(),
            Packet::Ok(packet) => packet.write_packet(),
            Packet::DialAuthority(packet) => packet.write_packet(),
            Packet::TargetPopulations(packet) => packet.write_packet(),
            Packet::CreatePolicy(packet) => packet.write_packet(),
            Packet::DeletePolicy(packet) => packet.write_packet(),
            Packet::PolicyResult(packet) => packet.write_packet(),
            Packet::SiteVisit(packet) => packet.write_packet(),
        };

        dst.extend_from_slice(&raw_packet);

        Ok(())
    }
}
