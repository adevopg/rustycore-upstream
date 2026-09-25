//! Value-added-service (VAS) character/realm list packets of client 3.4.3.54261.
//!
//! Evidence (`docs/migration/battlepay-343-protocol.md`, section 3.14): layouts
//! transcribed from the 54261 JAM readers/writers. The request token of every
//! answer must equal the client's pending store token (global `0x1431f7828`,
//! incremented by each store request), otherwise the client drops it.

use wow_constants::{ClientOpcodes, ServerOpcodes};
use wow_core::ObjectGuid;

use super::write_str_body;
use crate::{ClientPacket, PacketError, ServerPacket, WorldPacket};

/// CMSG_GET_VAS_ACCOUNT_CHARACTER_LIST 0x36f8 (client Write 0x140767150).
///
/// Wire: u32 ClientToken; u32 ChoiceType. Sent by
/// `C_StoreGlue.RequestStoreCharacterListForVasType` (0x141a4c410: DB2 VAS type
/// 2 -> 8 faction, 3 -> 10 race, 4 -> 7 name) and, with the ChoiceType echoed from
/// 0x27f2, right after a successful target realm list (0x141a4afed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GetVasAccountCharacterList {
    pub client_token: u32,
    pub choice_type: u32,
}

impl ClientPacket for GetVasAccountCharacterList {
    const OPCODE: ClientOpcodes = ClientOpcodes::GetVasAccountCharacterList;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self {
            client_token: pkt.read_uint32()?,
            choice_type: pkt.read_uint32()?,
        })
    }
}

/// CMSG_GET_VAS_TRANSFER_TARGET_REALM_LIST 0x36f9 (client Write 0x1407671a0).
///
/// Wire: u32 ClientToken; u32 ChoiceType (15 for a character transfer, 0x141a4c485).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GetVasTransferTargetRealmList {
    pub client_token: u32,
    pub choice_type: u32,
}

impl ClientPacket for GetVasTransferTargetRealmList {
    const OPCODE: ClientOpcodes = ClientOpcodes::GetVasTransferTargetRealmList;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self {
            client_token: pkt.read_uint32()?,
            choice_type: pkt.read_uint32()?,
        })
    }
}

/// CMSG_VAS_GET_SERVICE_STATUS 0x3711 (client Write 0x14076a117: opcode only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VasGetServiceStatus;

impl ClientPacket for VasGetServiceStatus {
    const OPCODE: ClientOpcodes = ClientOpcodes::VasGetServiceStatus;

    fn read(_pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self)
    }
}

/// CMSG_VAS_CHECK_TRANSFER_OK 0x3713 (client Write 0x14076a000).
///
/// Wire: u32 ClientToken; u8 (NameLen >> 3); bits(3) (NameLen & 7); flush;
/// BnetAccountName. The length is written as one raw byte followed by three bits,
/// i.e. an 11-bit length read MSB-first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VasCheckTransferOk {
    pub client_token: u32,
    pub bnet_account_name: String,
}

impl ClientPacket for VasCheckTransferOk {
    const OPCODE: ClientOpcodes = ClientOpcodes::VasCheckTransferOk;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        let client_token = pkt.read_uint32()?;
        let high = pkt.read_uint8()? as usize;
        let low = pkt.read_bits(3)? as usize;
        let bnet_account_name = pkt.read_string((high << 3) | low)?;
        Ok(Self {
            client_token,
            bnet_account_name,
        })
    }
}

/// `JamCliAccountCharacterData` (client reader 0x14070a020, stride 0x170).
///
/// Wire: packed guid WowAccount; packed guid Character; u32 VirtualRealmAddress;
/// u8 Race; u8 Class; u8 Sex; u8 Level; u64 LastLogin; u32 Unk; bits(6) NameLen;
/// bits(9) RealmNameLen; flush; Name; RealmName. The store filters by realm name
/// (`C_StoreSecure.GetCharactersForRealm`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VasAccountCharacter {
    pub wow_account_guid: ObjectGuid,
    pub character_guid: ObjectGuid,
    pub virtual_realm_address: u32,
    pub race: u8,
    pub class: u8,
    pub sex: u8,
    pub level: u8,
    pub last_login: u64,
    pub unk: u32,
    /// Max 63 bytes.
    pub name: String,
    /// Max 511 bytes.
    pub realm_name: String,
}

impl VasAccountCharacter {
    pub fn write(&self, pkt: &mut WorldPacket) {
        debug_assert!(self.name.len() < (1 << 6));
        debug_assert!(self.realm_name.len() < (1 << 9));
        pkt.write_packed_guid(&self.wow_account_guid);
        pkt.write_packed_guid(&self.character_guid);
        pkt.write_uint32(self.virtual_realm_address);
        pkt.write_uint8(self.race);
        pkt.write_uint8(self.class);
        pkt.write_uint8(self.sex);
        pkt.write_uint8(self.level);
        pkt.write_uint64(self.last_login);
        pkt.write_uint32(self.unk);
        pkt.write_bits(self.name.len() as u32, 6);
        pkt.write_bits(self.realm_name.len() as u32, 9);
        pkt.flush_bits();
        write_str_body(pkt, &self.name);
        write_str_body(pkt, &self.realm_name);
    }
}

/// SMSG_GET_VAS_ACCOUNT_CHARACTER_LIST_RESULT 0x27f1 (ctor-reader 0x1406c1310,
/// handler 0x141a4ac00 -> STORE_CHARACTER_LIST_RECEIVED).
///
/// Wire: u32 Token; u32 Result; u32 ChoiceType; u32 Count; entries. The handler
/// drops the packet unless `Token` is the pending store token and keeps the
/// characters only for `Result == 0`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GetVasAccountCharacterListResult {
    pub token: u32,
    pub result: u32,
    pub choice_type: u32,
    pub characters: Vec<VasAccountCharacter>,
}

impl ServerPacket for GetVasAccountCharacterListResult {
    const OPCODE: ServerOpcodes = ServerOpcodes::GetVasAccountCharacterListResult;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.token);
        pkt.write_uint32(self.result);
        pkt.write_uint32(self.choice_type);
        pkt.write_uint32(self.characters.len() as u32);
        for character in &self.characters {
            character.write(pkt);
        }
    }
}

/// `JamCliVASTargetRealm` (client reader loop in 0x1406fcff0, stride 0x124).
///
/// Wire: u32 VirtualRealmAddress; u32 CfgRealmsID; u32 CfgCategoriesID;
/// u32 CfgConfigsID; u32 CfgLanguagesID; u32 Unk; u8 PopulationState; u8 Unk2;
/// u32 Unk3; bits(9) NameLen; flush; Name. Names after the address follow the
/// LegionCore 7.3.5 `VasRealm` order (inferred).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VasTargetRealm {
    pub virtual_realm_address: u32,
    pub cfg_realms_id: u32,
    pub cfg_categories_id: u32,
    pub cfg_configs_id: u32,
    pub cfg_languages_id: u32,
    pub unk: u32,
    pub population_state: u8,
    pub unk2: u8,
    pub unk3: u32,
    /// Max 511 bytes.
    pub name: String,
}

impl VasTargetRealm {
    pub fn write(&self, pkt: &mut WorldPacket) {
        debug_assert!(self.name.len() < (1 << 9));
        pkt.write_uint32(self.virtual_realm_address);
        pkt.write_uint32(self.cfg_realms_id);
        pkt.write_uint32(self.cfg_categories_id);
        pkt.write_uint32(self.cfg_configs_id);
        pkt.write_uint32(self.cfg_languages_id);
        pkt.write_uint32(self.unk);
        pkt.write_uint8(self.population_state);
        pkt.write_uint8(self.unk2);
        pkt.write_uint32(self.unk3);
        pkt.write_bits(self.name.len() as u32, 9);
        pkt.flush_bits();
        write_str_body(pkt, &self.name);
    }
}

/// SMSG_GET_VAS_TRANSFER_TARGET_REALM_LIST_RESULT 0x27f2 (ctor 0x1406c1410 ->
/// reader 0x1406fcff0, handler 0x141a4ae10).
///
/// Wire: u32 Token; u32 Result; u32 ChoiceType; u32 Count; realms. After a matching
/// token the client itself requests the character list (CMSG 0x36f8) with this
/// ChoiceType, whatever the result.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GetVasTransferTargetRealmListResult {
    pub token: u32,
    pub result: u32,
    pub choice_type: u32,
    pub realms: Vec<VasTargetRealm>,
}

impl ServerPacket for GetVasTransferTargetRealmListResult {
    const OPCODE: ServerOpcodes = ServerOpcodes::GetVasTransferTargetRealmListResult;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.token);
        pkt.write_uint32(self.result);
        pkt.write_uint32(self.choice_type);
        pkt.write_uint32(self.realms.len() as u32);
        for realm in &self.realms {
            realm.write(pkt);
        }
    }
}

/// SMSG_VAS_GET_SERVICE_STATUS_RESPONSE 0x2819 (ctor-reader 0x1406cd600).
///
/// Wire: one byte, bits(4) TransferQueue then bits(4) FactionTransferQueue
/// (`Enum.VasQueueStatus`: 0 UnderAnHour .. 11 Over_7_Days).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VasGetServiceStatusResponse {
    pub transfer_queue: u8,
    pub faction_transfer_queue: u8,
}

impl ServerPacket for VasGetServiceStatusResponse {
    const OPCODE: ServerOpcodes = ServerOpcodes::VasGetServiceStatusResponse;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_bits(u32::from(self.transfer_queue & 0xF), 4);
        pkt.write_bits(u32::from(self.faction_transfer_queue & 0xF), 4);
        pkt.flush_bits();
    }
}

/// One game account of a validated Battle.net transfer target.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VasTransferGameAccount {
    pub guid: ObjectGuid,
    /// Max 2047 bytes (11-bit length).
    pub name: String,
}

/// SMSG_VAS_CHECK_TRANSFER_OK_RESPONSE 0x281c (ctor-reader 0x1406cd4c0).
///
/// Wire: u32 ClientToken; u32 Result; packed guid BnetAccount; u32 Count;
/// { packed guid WowAccount; bits(11) NameLen; flush; Name }[Count].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VasCheckTransferOkResponse {
    pub client_token: u32,
    pub result: u32,
    pub bnet_account_guid: ObjectGuid,
    pub game_accounts: Vec<VasTransferGameAccount>,
}

impl ServerPacket for VasCheckTransferOkResponse {
    const OPCODE: ServerOpcodes = ServerOpcodes::VasCheckTransferOkResponse;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.client_token);
        pkt.write_uint32(self.result);
        pkt.write_packed_guid(&self.bnet_account_guid);
        pkt.write_uint32(self.game_accounts.len() as u32);
        for account in &self.game_accounts {
            debug_assert!(account.name.len() < (1 << 11));
            pkt.write_packed_guid(&account.guid);
            pkt.write_bits(account.name.len() as u32, 11);
            pkt.flush_bits();
            write_str_body(pkt, &account.name);
        }
    }
}
