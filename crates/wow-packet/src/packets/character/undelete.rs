//! Character undelete packets (TrinityCore 3.4.3 `CharacterPackets.{h,cpp}`,
//! tag TDB343.24081: `UndeleteCharacter::Read`, `UndeleteCharacterResponse::Write`,
//! `EnumCharactersResult::Write` with `IsDeletedCharacters`).

use wow_constants::{ClientOpcodes, ServerOpcodes};
use wow_core::ObjectGuid;

use super::EnumCharactersResult;
use crate::{ClientPacket, PacketError, ServerPacket, WorldPacket};

/// C++ `CharacterUndeleteResult` (`SharedDefines.h:6252-6259`).
pub mod undelete_result {
    pub const OK: u32 = 0;
    pub const ERROR_COOLDOWN: u32 = 1;
    pub const ERROR_CHAR_CREATE: u32 = 2;
    pub const ERROR_DISABLED: u32 = 3;
    pub const ERROR_NAME_TAKEN_BY_THIS_ACCOUNT: u32 = 4;
    pub const ERROR_UNKNOWN: u32 = 5;
}

/// C++ `WorldPackets::Character::EnumCharacters` for
/// `CMSG_ENUM_CHARACTERS_DELETED_BY_CLIENT` (empty body).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EnumCharactersDeletedByClient;

impl ClientPacket for EnumCharactersDeletedByClient {
    const OPCODE: ClientOpcodes = ClientOpcodes::EnumCharactersDeletedByClient;

    fn read(_packet: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self)
    }
}

/// C++ `WorldPackets::Character::UndeleteCharacter` (`CharacterUndeleteInfo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndeleteCharacter {
    pub client_token: i32,
    pub character_guid: ObjectGuid,
}

impl ClientPacket for UndeleteCharacter {
    const OPCODE: ClientOpcodes = ClientOpcodes::UndeleteCharacter;

    fn read(packet: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self {
            client_token: packet.read_int32()?,
            character_guid: packet.read_packed_guid()?,
        })
    }
}

/// C++ `WorldPackets::Character::UndeleteCharacterResponse`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndeleteCharacterResponse {
    pub client_token: i32,
    /// [`undelete_result`].
    pub result: u32,
    pub character_guid: ObjectGuid,
}

impl ServerPacket for UndeleteCharacterResponse {
    const OPCODE: ServerOpcodes = ServerOpcodes::UndeleteCharacterResponse;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_int32(self.client_token);
        pkt.write_uint32(self.result);
        pkt.write_packed_guid(&self.character_guid);
    }
}

/// `SMSG_ENUM_CHARACTERS_RESULT` answering
/// `CMSG_ENUM_CHARACTERS_DELETED_BY_CLIENT`: identical to [`EnumCharactersResult`]
/// except that `IsDeletedCharacters` is set.
pub struct DeletedEnumCharactersResult(pub EnumCharactersResult);

impl ServerPacket for DeletedEnumCharactersResult {
    const OPCODE: ServerOpcodes = ServerOpcodes::EnumCharactersResult;

    fn write(&self, pkt: &mut WorldPacket) {
        self.0.write_like_cpp(pkt, true);
    }
}
