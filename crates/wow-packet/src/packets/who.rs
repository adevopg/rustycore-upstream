// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Who packet definitions: CMSG_WHO (0x3683) and SMSG_WHO (0x2BAE).
//!
//! C++ anchors (TrinityCore 3.4.3, `src/server/game/Server/Packets/WhoPackets.cpp`):
//! `WhoWord` reader :34-40, `WhoRequestServerInfo` reader :42-51, `WhoRequest`
//! reader :53-86, `WhoRequestPkt::Read` :88-99, `WhoEntry` writer :101-116,
//! `WhoResponse` writer :118-127, `WhoResponsePkt::Write` :129-135.
//! `PlayerGuidLookupData` writer: `QueryPackets.cpp:209-233`.
//!
//! `CMSG_WHO_IS` / `SMSG_WHO_IS` (GM account lookup) are out of scope here.

use crate::packets::query::PlayerGuidLookupData;
use crate::{ClientPacket, PacketError, ServerPacket, WorldPacket};
use wow_constants::{ClientOpcodes, ServerOpcodes};
use wow_core::ObjectGuid;

/// C++ `MAX_DECLINED_NAME_CASES`.
const MAX_DECLINED_NAME_CASES_LIKE_CPP: usize = 5;

/// C++ `WhoRequestServerInfo` (`WhoPackets.h:53-58`), present only when the
/// `hasWhoRequest` bit is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WhoRequestServerInfo {
    pub faction_group: i32,
    pub locale: i32,
    pub requester_virtual_realm_address: u32,
}

/// C++ `WhoRequest` (`WhoPackets.h:60-75`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoRequest {
    pub min_level: i32,
    pub max_level: i32,
    pub name: String,
    pub virtual_realm_name: String,
    pub guild: String,
    pub guild_virtual_realm_name: String,
    /// C++ `Trinity::RaceMask<int64>::RawValue`.
    pub race_filter: i64,
    /// C++ `int32 ClassFilter = -1` (negative means "any class").
    pub class_filter: i32,
    /// C++ `std::vector<WhoWord> Words` (3-bit count on the wire).
    pub words: Vec<String>,
    pub show_enemies: bool,
    pub show_arena_players: bool,
    pub exact_name: bool,
    pub server_info: Option<WhoRequestServerInfo>,
}

impl Default for WhoRequest {
    fn default() -> Self {
        Self {
            min_level: 0,
            max_level: 0,
            name: String::new(),
            virtual_realm_name: String::new(),
            guild: String::new(),
            guild_virtual_realm_name: String::new(),
            race_filter: 0,
            class_filter: -1,
            words: Vec::new(),
            show_enemies: false,
            show_arena_players: false,
            exact_name: false,
            server_info: None,
        }
    }
}

impl WhoRequest {
    /// C++ `operator>>(ByteBuffer&, WhoRequest&)` (`WhoPackets.cpp:53-86`).
    fn read(packet: &mut WorldPacket) -> Result<Self, PacketError> {
        let min_level = packet.read_int32()?;
        let max_level = packet.read_int32()?;
        let race_filter = packet.read_int64()?;
        let class_filter = packet.read_int32()?;

        let name_len = packet.read_bits(6)? as usize;
        let virtual_realm_name_len = packet.read_bits(9)? as usize;
        let guild_len = packet.read_bits(7)? as usize;
        let guild_virtual_realm_name_len = packet.read_bits(9)? as usize;
        let words_count = packet.read_bits(3)? as usize;

        let show_enemies = packet.read_bit()?;
        let show_arena_players = packet.read_bit()?;
        let exact_name = packet.read_bit()?;
        let has_server_info = packet.read_bit()?;
        packet.reset_bits();

        // C++ `operator>>(ByteBuffer&, WhoWord&)`: 7-bit length, string, ResetBitPos.
        let mut words = Vec::with_capacity(words_count);
        for _ in 0..words_count {
            let word_len = packet.read_bits(7)? as usize;
            words.push(packet.read_string(word_len)?);
            packet.reset_bits();
        }

        let name = packet.read_string(name_len)?;
        let virtual_realm_name = packet.read_string(virtual_realm_name_len)?;
        let guild = packet.read_string(guild_len)?;
        let guild_virtual_realm_name = packet.read_string(guild_virtual_realm_name_len)?;

        let server_info = if has_server_info {
            Some(WhoRequestServerInfo {
                faction_group: packet.read_int32()?,
                locale: packet.read_int32()?,
                requester_virtual_realm_address: packet.read_uint32()?,
            })
        } else {
            None
        };

        Ok(Self {
            min_level,
            max_level,
            name,
            virtual_realm_name,
            guild,
            guild_virtual_realm_name,
            race_filter,
            class_filter,
            words,
            show_enemies,
            show_arena_players,
            exact_name,
            server_info,
        })
    }
}

/// CMSG_WHO — C++ `WhoRequestPkt` (`WhoPackets.h:77-88`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WhoRequestPkt {
    pub request: WhoRequest,
    pub request_id: u32,
    /// 1 = Social, 2 = Chat, 3 = Item.
    pub origin: u8,
    pub is_from_addon: bool,
    /// C++ `Array<int32, 10> Areas` (4-bit count on the wire; the handler
    /// rejects more than 10 like `HandleWhoOpcode`).
    pub areas: Vec<i32>,
}

impl ClientPacket for WhoRequestPkt {
    const OPCODE: ClientOpcodes = ClientOpcodes::Who;

    /// C++ `WhoRequestPkt::Read` (`WhoPackets.cpp:88-99`).
    fn read(packet: &mut WorldPacket) -> Result<Self, PacketError> {
        let areas_count = packet.read_bits(4)? as usize;
        let is_from_addon = packet.read_bit()?;

        let request = WhoRequest::read(packet)?;
        let request_id = packet.read_uint32()?;
        let origin = packet.read_uint8()?;

        let mut areas = Vec::with_capacity(areas_count);
        for _ in 0..areas_count {
            areas.push(packet.read_int32()?);
        }

        Ok(Self {
            request,
            request_id,
            origin,
            is_from_addon,
            areas,
        })
    }
}

/// C++ `operator<<(ByteBuffer&, PlayerGuidLookupData const&)` (`QueryPackets.cpp:209-233`).
///
/// Kept beside the who writer because `WhoEntry` embeds it; the same field order
/// is what `SMSG_QUERY_PLAYER_NAMES_RESPONSE` writes inline.
pub fn write_player_guid_lookup_data_like_cpp(w: &mut WorldPacket, data: &PlayerGuidLookupData) {
    w.write_bit(data.is_deleted);
    w.write_bits(data.name.len() as u32, 6);
    for declined in data
        .declined_names
        .iter()
        .take(MAX_DECLINED_NAME_CASES_LIKE_CPP)
    {
        w.write_bits(declined.len() as u32, 7);
    }
    for declined in data
        .declined_names
        .iter()
        .take(MAX_DECLINED_NAME_CASES_LIKE_CPP)
    {
        // C++ `WriteString` appends nothing for an empty string.
        if !declined.is_empty() {
            w.write_string(declined);
        }
    }
    w.write_packed_guid(&data.account_id);
    w.write_packed_guid(&data.bnet_account_id);
    w.write_packed_guid(&data.guid_actual);
    w.write_uint64(data.guild_club_member_id);
    w.write_uint32(data.virtual_realm_address);
    w.write_uint8(data.race);
    w.write_uint8(data.sex);
    w.write_uint8(data.class);
    w.write_uint8(data.level);
    w.write_uint8(0); // Unused915
    w.write_string(&data.name);
}

/// C++ `WhoEntry` (`WhoPackets.h:90-98`).
pub struct WhoEntry {
    pub player_data: PlayerGuidLookupData,
    pub guild_guid: ObjectGuid,
    pub guild_virtual_realm_address: u32,
    pub guild_name: String,
    pub area_id: i32,
    pub is_gm: bool,
}

impl WhoEntry {
    /// C++ `operator<<(ByteBuffer&, WhoEntry const&)` (`WhoPackets.cpp:101-116`).
    fn write(&self, w: &mut WorldPacket) {
        write_player_guid_lookup_data_like_cpp(w, &self.player_data);
        w.write_packed_guid(&self.guild_guid);
        w.write_uint32(self.guild_virtual_realm_address);
        w.write_int32(self.area_id);
        w.write_bits(self.guild_name.len() as u32, 7);
        w.write_bit(self.is_gm);
        w.flush_bits();
        w.write_string(&self.guild_name);
    }
}

/// SMSG_WHO — C++ `WhoResponsePkt` (`WhoPackets.h:105-114`).
pub struct WhoResponsePkt {
    pub request_id: u32,
    pub entries: Vec<WhoEntry>,
}

impl ServerPacket for WhoResponsePkt {
    const OPCODE: ServerOpcodes = ServerOpcodes::Who;

    /// C++ `WhoResponsePkt::Write` (`WhoPackets.cpp:129-135`) and
    /// `operator<<(ByteBuffer&, WhoResponse const&)` (:118-127): a 6-bit entry
    /// count, so at most 63 entries can ever be encoded.
    fn write(&self, w: &mut WorldPacket) {
        w.write_uint32(self.request_id);
        w.write_bits(self.entries.len() as u32, 6);
        w.flush_bits();
        for entry in &self.entries {
            entry.write(w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a CMSG_WHO body exactly as `WhoRequestPkt::Read` expects it.
    #[allow(clippy::too_many_arguments)]
    fn write_who_request_like_cpp(
        areas: &[i32],
        is_from_addon: bool,
        request: &WhoRequest,
        request_id: u32,
        origin: u8,
    ) -> WorldPacket {
        let mut pkt = WorldPacket::new_empty();
        pkt.write_bits(areas.len() as u32, 4);
        pkt.write_bit(is_from_addon);
        pkt.write_int32(request.min_level);
        pkt.write_int32(request.max_level);
        pkt.write_int64(request.race_filter);
        pkt.write_int32(request.class_filter);
        pkt.write_bits(request.name.len() as u32, 6);
        pkt.write_bits(request.virtual_realm_name.len() as u32, 9);
        pkt.write_bits(request.guild.len() as u32, 7);
        pkt.write_bits(request.guild_virtual_realm_name.len() as u32, 9);
        pkt.write_bits(request.words.len() as u32, 3);
        pkt.write_bit(request.show_enemies);
        pkt.write_bit(request.show_arena_players);
        pkt.write_bit(request.exact_name);
        pkt.write_bit(request.server_info.is_some());
        pkt.flush_bits();
        for word in &request.words {
            pkt.write_bits(word.len() as u32, 7);
            pkt.flush_bits();
            pkt.write_string(word);
        }
        pkt.write_string(&request.name);
        pkt.write_string(&request.virtual_realm_name);
        pkt.write_string(&request.guild);
        pkt.write_string(&request.guild_virtual_realm_name);
        if let Some(info) = request.server_info {
            pkt.write_int32(info.faction_group);
            pkt.write_int32(info.locale);
            pkt.write_uint32(info.requester_virtual_realm_address);
        }
        pkt.write_uint32(request_id);
        pkt.write_uint8(origin);
        for area in areas {
            pkt.write_int32(*area);
        }
        pkt
    }

    #[test]
    fn who_request_reads_every_field_in_cpp_order() {
        let request = WhoRequest {
            min_level: 10,
            max_level: 80,
            name: "jaina".into(),
            virtual_realm_name: "Realm".into(),
            guild: "Kirin".into(),
            guild_virtual_realm_name: String::new(),
            race_filter: -1,
            class_filter: 1 << 8,
            words: vec!["mage".into(), "dala".into()],
            show_enemies: true,
            show_arena_players: false,
            exact_name: true,
            server_info: Some(WhoRequestServerInfo {
                faction_group: 1,
                locale: 0,
                requester_virtual_realm_address: 0x01000001,
            }),
        };
        let mut pkt = write_who_request_like_cpp(&[1519, 4395], true, &request, 0x1234, 2);

        let parsed = WhoRequestPkt::read(&mut pkt).expect("who request");

        assert_eq!(parsed.request, request);
        assert_eq!(parsed.request_id, 0x1234);
        assert_eq!(parsed.origin, 2);
        assert!(parsed.is_from_addon);
        assert_eq!(parsed.areas, vec![1519, 4395]);
        assert!(pkt.is_empty());
    }

    #[test]
    fn who_request_minimal_client_default_reads_like_cpp() {
        let request = WhoRequest {
            min_level: 0,
            max_level: 100,
            race_filter: -1,
            class_filter: -1,
            ..WhoRequest::default()
        };
        let mut pkt = write_who_request_like_cpp(&[], false, &request, 1, 1);

        let parsed = WhoRequestPkt::read(&mut pkt).expect("who request");

        assert_eq!(parsed.request, request);
        assert!(parsed.areas.is_empty());
        assert!(!parsed.is_from_addon);
        assert!(parsed.request.server_info.is_none());
        assert!(pkt.is_empty());
    }

    #[test]
    fn who_request_truncated_word_is_an_error() {
        let mut pkt = WorldPacket::new_empty();
        pkt.write_bits(0, 4);
        pkt.write_bit(false);
        pkt.write_int32(0);
        pkt.write_int32(80);
        pkt.write_int64(-1);
        pkt.write_int32(-1);
        pkt.write_bits(0, 6);
        pkt.write_bits(0, 9);
        pkt.write_bits(0, 7);
        pkt.write_bits(0, 9);
        pkt.write_bits(1, 3);
        pkt.write_bits(0, 4);
        pkt.flush_bits();
        pkt.write_bits(10, 7);
        pkt.flush_bits();
        pkt.write_string("abc");

        assert!(WhoRequestPkt::read(&mut pkt).is_err());
    }

    fn lookup(name: &str, guid: ObjectGuid) -> PlayerGuidLookupData {
        PlayerGuidLookupData {
            name: name.into(),
            guid_actual: guid,
            race: 1,
            sex: 0,
            class: 8,
            level: 80,
            virtual_realm_address: 0x01000001,
            ..Default::default()
        }
    }

    #[test]
    fn who_response_writes_request_id_6bit_count_and_entries_like_cpp() {
        let guid = ObjectGuid::create_player(1, 42);
        let packet = WhoResponsePkt {
            request_id: 0xABCD,
            entries: vec![WhoEntry {
                player_data: lookup("Jaina", guid),
                guild_guid: ObjectGuid::EMPTY,
                guild_virtual_realm_address: 0,
                guild_name: String::new(),
                area_id: 1519,
                is_gm: true,
            }],
        };
        let bytes = packet.to_bytes();
        assert_eq!(
            u16::from_le_bytes([bytes[0], bytes[1]]),
            ServerOpcodes::Who as u16
        );
        let mut body = WorldPacket::from_bytes(&bytes[2..]);

        assert_eq!(body.read_uint32().unwrap(), 0xABCD);
        assert_eq!(body.read_bits(6).unwrap(), 1);
        body.reset_bits();
        // PlayerGuidLookupData
        assert!(!body.read_bit().unwrap());
        assert_eq!(body.read_bits(6).unwrap(), 5);
        for _ in 0..5 {
            assert_eq!(body.read_bits(7).unwrap(), 0);
        }
        assert_eq!(body.read_packed_guid().unwrap(), ObjectGuid::EMPTY);
        assert_eq!(body.read_packed_guid().unwrap(), ObjectGuid::EMPTY);
        assert_eq!(body.read_packed_guid().unwrap(), guid);
        assert_eq!(body.read_uint64().unwrap(), 0);
        assert_eq!(body.read_uint32().unwrap(), 0x01000001);
        assert_eq!(body.read_uint8().unwrap(), 1);
        assert_eq!(body.read_uint8().unwrap(), 0);
        assert_eq!(body.read_uint8().unwrap(), 8);
        assert_eq!(body.read_uint8().unwrap(), 80);
        assert_eq!(body.read_uint8().unwrap(), 0);
        assert_eq!(body.read_string(5).unwrap(), "Jaina");
        // WhoEntry tail
        assert_eq!(body.read_packed_guid().unwrap(), ObjectGuid::EMPTY);
        assert_eq!(body.read_uint32().unwrap(), 0);
        assert_eq!(body.read_int32().unwrap(), 1519);
        assert_eq!(body.read_bits(7).unwrap(), 0);
        assert!(body.read_bit().unwrap());
        body.reset_bits();
        assert!(body.is_empty());
    }

    #[test]
    fn who_response_with_guild_writes_name_after_flushed_bits() {
        let guid = ObjectGuid::create_player(1, 7);
        let guild_guid = ObjectGuid::create_guild(wow_core::guid::HighGuid::Guild, 1, 99);
        let packet = WhoResponsePkt {
            request_id: 1,
            entries: vec![WhoEntry {
                player_data: lookup("Bob", guid),
                guild_guid,
                guild_virtual_realm_address: 0x01000001,
                guild_name: "Kirin Tor".into(),
                area_id: 4395,
                is_gm: false,
            }],
        };
        let bytes = packet.to_bytes();
        let mut body = WorldPacket::from_bytes(&bytes[2..]);
        body.read_uint32().unwrap();
        assert_eq!(body.read_bits(6).unwrap(), 1);
        body.reset_bits();
        body.read_bit().unwrap();
        body.read_bits(6).unwrap();
        for _ in 0..5 {
            body.read_bits(7).unwrap();
        }
        body.read_packed_guid().unwrap();
        body.read_packed_guid().unwrap();
        body.read_packed_guid().unwrap();
        body.read_uint64().unwrap();
        body.read_uint32().unwrap();
        body.read_bytes(5).unwrap();
        body.read_string(3).unwrap();
        assert_eq!(body.read_packed_guid().unwrap(), guild_guid);
        assert_eq!(body.read_uint32().unwrap(), 0x01000001);
        assert_eq!(body.read_int32().unwrap(), 4395);
        assert_eq!(body.read_bits(7).unwrap(), 9);
        assert!(!body.read_bit().unwrap());
        assert_eq!(body.read_string(9).unwrap(), "Kirin Tor");
        assert!(body.is_empty());
    }

    #[test]
    fn who_response_empty_is_request_id_then_one_zero_byte() {
        let packet = WhoResponsePkt {
            request_id: 9,
            entries: Vec::new(),
        };
        let bytes = packet.to_bytes();
        assert_eq!(&bytes[2..], &[9, 0, 0, 0, 0]);
    }
}
