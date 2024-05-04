use tokio_util::codec::Decoder;

use bytes::BytesMut;

use tracing::instrument;

use super::{Error, RawPacket};

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
    Hello(RawPacket<hello::PacketDecoder>),
    Error(RawPacket<error::PacketDecoder>),
    Ok(RawPacket<ok::PacketDecoder>),
    DialAuthority(RawPacket<dial_authority::PacketDecoder>),
    TargetPopulations(RawPacket<target_populations::PacketDecoder>),
    CreatePolicy(RawPacket<create_policy::PacketDecoder>),
    DeletePolicy(RawPacket<delete_policy::PacketDecoder>),
    PolicyResult(RawPacket<policy_result::PacketDecoder>),
    SiteVisit(RawPacket<site_visit::PacketDecoder>),
}

pub struct PacketDecoder;

impl PacketDecoder {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for PacketDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for PacketDecoder {
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
