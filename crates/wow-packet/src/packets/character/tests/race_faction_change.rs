//! `CMSG_CHAR_RACE_OR_FACTION_CHANGE` / `SMSG_CHAR_FACTION_CHANGE_RESULT` codec
//! (TC 3.4.3 `CharacterPackets.cpp` `CharRaceOrFactionChange::Read`,
//! `CharFactionChangeResult::Write`; client 54261 Write 0x140765acb uses the packed
//! GUID writer 0x141550bf0).

use super::*;

#[test]
fn char_race_or_faction_change_reads_cpp_bit_name_len_guid_bytes_count_name_choices() {
    let guid = ObjectGuid::create_player(1, 42);
    let mut pkt = WorldPacket::new_empty();
    pkt.write_bit(true);
    pkt.write_bits(7, 6);
    pkt.write_packed_guid(&guid);
    pkt.write_uint8(1);
    pkt.write_uint8(2);
    pkt.write_uint8(1);
    pkt.write_uint32(2);
    pkt.write_string("Newname");
    pkt.write_int32(20);
    pkt.write_int32(200);
    pkt.write_int32(10);
    pkt.write_int32(100);
    pkt.reset_read();

    let result = CharRaceOrFactionChange::read(&mut pkt).unwrap();

    assert!(result.faction_change);
    assert_eq!(result.guid, guid);
    assert_eq!(result.sex_id, 1);
    assert_eq!(result.race_id, 2);
    assert_eq!(result.initial_race_id, 1);
    assert_eq!(result.name, "Newname");
    assert_eq!(
        result.customizations,
        vec![
            ChrCustomizationChoice {
                option_id: 10,
                choice_id: 100,
            },
            ChrCustomizationChoice {
                option_id: 20,
                choice_id: 200,
            },
        ]
    );
    assert_eq!(pkt.remaining(), 0);
}

#[test]
fn char_faction_change_result_failure_writes_result_guid_and_clear_display_bit() {
    let guid = ObjectGuid::create_player(1, 42);
    let bytes = CharFactionChangeResult {
        result: 42,
        guid,
        display: None,
    }
    .to_bytes();
    let mut pkt = WorldPacket::from_bytes(&bytes);
    assert_eq!(
        pkt.server_opcode(),
        Some(ServerOpcodes::CharFactionChangeResult)
    );
    pkt.skip_opcode();
    assert_eq!(pkt.read_uint8().unwrap(), 42);
    assert_eq!(pkt.read_packed_guid().unwrap(), guid);
    assert!(!pkt.read_bit().unwrap());
    assert_eq!(pkt.remaining(), 0);
}

#[test]
fn char_faction_change_result_success_writes_display_block() {
    let guid = ObjectGuid::create_player(1, 42);
    let bytes = CharFactionChangeResult {
        result: 0,
        guid,
        display: Some(CharFactionChangeDisplayInfo {
            name: "Newname".to_string(),
            sex_id: 1,
            race_id: 2,
            customizations: vec![ChrCustomizationChoice {
                option_id: 10,
                choice_id: 100,
            }],
        }),
    }
    .to_bytes();
    let mut pkt = WorldPacket::from_bytes(&bytes);
    pkt.skip_opcode();
    assert_eq!(pkt.read_uint8().unwrap(), 0);
    assert_eq!(pkt.read_packed_guid().unwrap(), guid);
    assert!(pkt.read_bit().unwrap());
    // FlushBits after HasDisplay: the name length starts a new byte.
    pkt.reset_bits();
    assert_eq!(pkt.read_bits(6).unwrap(), 7);
    assert_eq!(pkt.read_uint8().unwrap(), 1);
    assert_eq!(pkt.read_uint8().unwrap(), 2);
    assert_eq!(pkt.read_uint32().unwrap(), 1);
    assert_eq!(pkt.read_string(7).unwrap(), "Newname");
    assert_eq!(pkt.read_int32().unwrap(), 10);
    assert_eq!(pkt.read_int32().unwrap(), 100);
    assert_eq!(pkt.remaining(), 0);
}
