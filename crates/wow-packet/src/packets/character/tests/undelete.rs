//! Character undelete codec regressions (TC 3.4.3 `CharacterPackets.cpp:491-513`).

use super::*;
use crate::packets::misc::UndeleteCooldownStatusResponse;

#[test]
fn undelete_character_reads_client_token_then_packed_guid() {
    let guid = ObjectGuid::create_player(1, 77);
    let mut pkt = WorldPacket::new_empty();
    pkt.write_int32(-5);
    pkt.write_packed_guid(&guid);
    pkt.reset_read();

    let read = UndeleteCharacter::read(&mut pkt).unwrap();
    assert_eq!(read.client_token, -5);
    assert_eq!(read.character_guid, guid);
}

#[test]
fn undelete_character_response_writes_token_result_guid() {
    let guid = ObjectGuid::create_player(1, 77);
    let bytes = UndeleteCharacterResponse {
        client_token: 9,
        result: undelete_result::ERROR_NAME_TAKEN_BY_THIS_ACCOUNT,
        character_guid: guid,
    }
    .to_bytes();
    let mut pkt = WorldPacket::from_bytes(&bytes);
    assert_eq!(
        pkt.server_opcode(),
        Some(ServerOpcodes::UndeleteCharacterResponse)
    );
    pkt.skip_opcode();
    assert_eq!(pkt.read_int32().unwrap(), 9);
    assert_eq!(pkt.read_uint32().unwrap(), 4);
    assert_eq!(pkt.read_packed_guid().unwrap(), guid);
    assert_eq!(pkt.remaining(), 0);
}

#[test]
fn undelete_result_values_match_shared_defines() {
    assert_eq!(undelete_result::OK, 0);
    assert_eq!(undelete_result::ERROR_COOLDOWN, 1);
    assert_eq!(undelete_result::ERROR_CHAR_CREATE, 2);
    assert_eq!(undelete_result::ERROR_DISABLED, 3);
    assert_eq!(undelete_result::ERROR_NAME_TAKEN_BY_THIS_ACCOUNT, 4);
    assert_eq!(undelete_result::ERROR_UNKNOWN, 5);
}

#[test]
fn undelete_cooldown_status_writes_bit_then_two_uint32() {
    let bytes = UndeleteCooldownStatusResponse {
        on_cooldown: true,
        max_cooldown: 2_592_000,
        current_cooldown: 3600,
    }
    .to_bytes();
    let mut pkt = WorldPacket::from_bytes(&bytes);
    assert_eq!(
        pkt.server_opcode(),
        Some(ServerOpcodes::UndeleteCooldownStatusResponse)
    );
    pkt.skip_opcode();
    assert!(pkt.read_bit().unwrap());
    assert_eq!(pkt.read_uint32().unwrap(), 2_592_000);
    assert_eq!(pkt.read_uint32().unwrap(), 3600);
    assert_eq!(pkt.remaining(), 0);
}

#[test]
fn deleted_enumeration_sets_only_the_is_deleted_characters_bit() {
    let result = || EnumCharactersResult {
        success: true,
        characters: vec![],
        race_unlock_data: vec![],
    };
    let normal = result().to_bytes();
    let deleted = DeletedEnumCharactersResult(result()).to_bytes();
    assert_eq!(normal.len(), deleted.len());
    // Opcode (2 bytes), then Success (bit 7) and IsDeletedCharacters (bit 6).
    assert_eq!(normal[2] & 0xC0, 0x80);
    assert_eq!(deleted[2] & 0xC0, 0xC0);
    assert_eq!(normal[3..], deleted[3..]);
}
