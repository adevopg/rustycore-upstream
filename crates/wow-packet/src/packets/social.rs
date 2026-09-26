// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Social packet definitions.
//!
//! C++ anchors (TrinityCore 3.4.3, `src/server/game/Server/Packets/SocialPackets.cpp`):
//! `SendContactList::Read` :22-25, `ContactInfo` writer :41-58, `ContactList::Write`
//! :60-70, `FriendStatus::Write` :85-101, `QualifiedGUID` reader :103-109,
//! `AddFriend::Read` :111-117, `DelFriend::Read` :119-122, `SetContactNotes::Read`
//! :124-128, `AddIgnore::Read` :130-135, `DelIgnore::Read` :137-140.

use crate::{ClientPacket, PacketError, ServerPacket, WorldPacket};
use wow_constants::{ClientOpcodes, ServerOpcodes};
use wow_core::ObjectGuid;

/// C++ `operator>>(ByteBuffer&, QualifiedGUID&)` (`SocialPackets.cpp:103-109`):
/// the 3.4.3 client writes `VirtualRealmAddress` **before** the packed GUID.
fn read_qualified_guid_like_cpp(
    packet: &mut WorldPacket,
) -> Result<(ObjectGuid, u32), PacketError> {
    let virtual_realm_address = packet.read_uint32()?;
    let guid = packet.read_packed_guid()?;
    Ok((guid, virtual_realm_address))
}

/// CMSG_SEND_CONTACT_LIST.
///
/// C++ `WorldPackets::Social::SendContactList::Read` reads one `uint32 Flags`
/// (`SocialFlag` bitmask).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendContactList {
    pub flags: u32,
}

impl ClientPacket for SendContactList {
    const OPCODE: ClientOpcodes = ClientOpcodes::SendContactList;

    fn read(packet: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self {
            flags: packet.read_uint32()?,
        })
    }
}

/// CMSG_ADD_FRIEND.
///
/// C++ `WorldPackets::Social::AddFriend::Read` reads a 9-bit name length, a
/// 9-bit notes length, then the name and the notes strings in that order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddFriend {
    pub name: String,
    pub notes: String,
}

impl ClientPacket for AddFriend {
    const OPCODE: ClientOpcodes = ClientOpcodes::AddFriend;

    fn read(packet: &mut WorldPacket) -> Result<Self, PacketError> {
        let name_len = packet.read_bits(9)? as usize;
        let notes_len = packet.read_bits(9)? as usize;
        let name = packet.read_string(name_len)?;
        let notes = packet.read_string(notes_len)?;
        Ok(Self { name, notes })
    }
}

/// CMSG_DEL_FRIEND.
///
/// C++ `WorldPackets::Social::DelFriend::Read` reads a `QualifiedGUID`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelFriend {
    pub player_guid: ObjectGuid,
    pub virtual_realm_address: u32,
}

impl ClientPacket for DelFriend {
    const OPCODE: ClientOpcodes = ClientOpcodes::DelFriend;

    fn read(packet: &mut WorldPacket) -> Result<Self, PacketError> {
        let (player_guid, virtual_realm_address) = read_qualified_guid_like_cpp(packet)?;
        Ok(Self {
            player_guid,
            virtual_realm_address,
        })
    }
}

/// FriendsResult enum values (byte).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendsResult {
    DbError = 0x00,
    ListFull = 0x01,
    Online = 0x02,
    Offline = 0x03,
    NotFound = 0x04,
    Removed = 0x05,
    AddedOnline = 0x06,
    AddedOffline = 0x07,
    Already = 0x08,
    Self_ = 0x09,
    Enemy = 0x0A,
    IgnoreFull = 0x0B,
    IgnoreSelf = 0x0C,
    IgnoreNotFound = 0x0D,
    IgnoreAlready = 0x0E,
    IgnoreAdded = 0x0F,
    IgnoreRemoved = 0x10,
    IgnoreAmbiguous = 0x11,
    MuteFull = 0x12,
    MuteSelf = 0x13,
    MuteNotFound = 0x14,
    MuteAlready = 0x15,
    MuteAdded = 0x16,
    MuteRemoved = 0x17,
    MuteAmbiguous = 0x18,
    Unknown = 0x1C,
}

/// CMSG_ADD_IGNORE.
///
/// C++ `WorldPackets::Social::AddIgnore::Read` reads a 9-bit name length,
/// then an account GUID, then the name string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddIgnore {
    pub name: String,
    pub account_guid: ObjectGuid,
}

impl ClientPacket for AddIgnore {
    const OPCODE: ClientOpcodes = ClientOpcodes::AddIgnore;

    fn read(packet: &mut WorldPacket) -> Result<Self, PacketError> {
        let name_len = packet.read_bits(9)? as usize;
        let account_guid = packet.read_packed_guid()?;
        let name = packet.read_string(name_len)?;
        Ok(Self { name, account_guid })
    }
}

/// CMSG_DEL_IGNORE.
///
/// C++ `WorldPackets::Social::DelIgnore::Read` reads a `QualifiedGUID`
/// (virtual realm address, then `ObjectGuid`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelIgnore {
    pub player_guid: ObjectGuid,
    pub virtual_realm_address: u32,
}

impl ClientPacket for DelIgnore {
    const OPCODE: ClientOpcodes = ClientOpcodes::DelIgnore;

    fn read(packet: &mut WorldPacket) -> Result<Self, PacketError> {
        let (player_guid, virtual_realm_address) = read_qualified_guid_like_cpp(packet)?;
        Ok(Self {
            player_guid,
            virtual_realm_address,
        })
    }
}

/// CMSG_SET_CONTACT_NOTES.
///
/// C++ `WorldPackets::Social::SetContactNotes::Read` reads a `QualifiedGUID`
/// (realm address, then GUID), then a 10-bit note length and the note string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetContactNotes {
    pub player_guid: ObjectGuid,
    pub virtual_realm_address: u32,
    pub notes: String,
}

impl ClientPacket for SetContactNotes {
    const OPCODE: ClientOpcodes = ClientOpcodes::SetContactNotes;

    fn read(packet: &mut WorldPacket) -> Result<Self, PacketError> {
        let (player_guid, virtual_realm_address) = read_qualified_guid_like_cpp(packet)?;
        let notes_len = packet.read_bits(10)? as usize;
        let notes = packet.read_string(notes_len)?;
        Ok(Self {
            player_guid,
            virtual_realm_address,
            notes,
        })
    }
}

/// CMSG_ACCOUNT_NOTIFICATION_ACKNOWLEDGED.
///
/// C++ `WorldPackets::Account::AccountNotificationAcknowledged::Read`
/// reads a single `uint32 NotificationId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountNotificationAcknowledged {
    pub notification_id: u32,
}

impl ClientPacket for AccountNotificationAcknowledged {
    const OPCODE: ClientOpcodes = ClientOpcodes::AccountNotificationAcknowledged;

    fn read(packet: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self {
            notification_id: packet.read_uint32()?,
        })
    }
}

/// CMSG_SOCIAL_CONTRACT_REQUEST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocialContractRequest;

impl ClientPacket for SocialContractRequest {
    const OPCODE: ClientOpcodes = ClientOpcodes::SocialContractRequest;

    fn read(_packet: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self)
    }
}

/// CMSG_ACCEPT_SOCIAL_CONTRACT.
///
/// C++ `WorldPackets::Account::AcceptSocialContract::Read` is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptSocialContract;

impl ClientPacket for AcceptSocialContract {
    const OPCODE: ClientOpcodes = ClientOpcodes::AcceptSocialContract;

    fn read(_packet: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self)
    }
}

/// SMSG_SOCIAL_CONTRACT_REQUEST_RESPONSE.
pub struct SocialContractRequestResponse {
    pub show_social_contract: bool,
}

impl ServerPacket for SocialContractRequestResponse {
    const OPCODE: ServerOpcodes = ServerOpcodes::SocialContractRequestResponse;

    fn write(&self, w: &mut WorldPacket) {
        w.write_bit(self.show_social_contract);
        w.flush_bits();
    }
}

/// SMSG_FRIEND_STATUS (0x278d)
pub struct FriendStatusPkt {
    pub result: FriendsResult,
    pub guid: ObjectGuid,
    pub account_guid: ObjectGuid,
    pub virtual_realm_address: u32,
    /// FriendStatus bitmask: OFFLINE=0x00, ONLINE=0x01, AFK=0x02, DND=0x04, RAF=0x08.
    pub status: u8,
    pub area_id: i32,
    pub level: i32,
    pub class_id: u32,
    pub notes: String,
}

impl ServerPacket for FriendStatusPkt {
    const OPCODE: ServerOpcodes = ServerOpcodes::FriendStatus;

    fn write(&self, w: &mut WorldPacket) {
        w.write_uint8(self.result as u8);
        w.write_packed_guid(&self.guid);
        w.write_packed_guid(&self.account_guid);
        w.write_uint32(self.virtual_realm_address);
        w.write_uint8(self.status);
        w.write_int32(self.area_id);
        w.write_int32(self.level);
        w.write_uint32(self.class_id);
        let note_bytes = self.notes.as_bytes();
        w.write_bits(note_bytes.len() as u32, 10);
        w.write_bit(false); // Mobile = false
        w.flush_bits();
        w.write_bytes(note_bytes);
    }
}

/// A single contact entry for SMSG_CONTACT_LIST.
pub struct ContactInfo {
    pub guid: ObjectGuid,
    pub wow_account_guid: ObjectGuid,
    pub virtual_realm_address: u32,
    pub native_realm_address: u32,
    /// SocialFlag: 1=friend, 2=ignored, 4=muted
    pub type_flags: u32,
    pub note: String,
    /// FriendStatus bitmask: OFFLINE=0x00, ONLINE=0x01, AFK=0x02, DND=0x04, RAF=0x08.
    pub status: u8,
    pub area_id: u32,
    pub level: u32,
    pub class_id: u32,
    pub is_mobile: bool,
}

impl ContactInfo {
    pub fn write(&self, w: &mut WorldPacket) {
        w.write_packed_guid(&self.guid);
        w.write_packed_guid(&self.wow_account_guid);
        w.write_uint32(self.virtual_realm_address);
        w.write_uint32(self.native_realm_address);
        w.write_uint32(self.type_flags);
        w.write_uint8(self.status);
        w.write_int32(self.area_id as i32);
        w.write_int32(self.level as i32);
        w.write_uint32(self.class_id);
        let note_bytes = self.note.as_bytes();
        w.write_bits(note_bytes.len() as u32, 10);
        w.write_bit(self.is_mobile);
        w.flush_bits();
        w.write_bytes(note_bytes);
    }
}

/// SMSG_CONTACT_LIST (0x278c)
pub struct ContactListPkt {
    /// SocialFlag bitmask requested
    pub flags: u32,
    pub contacts: Vec<ContactInfo>,
}

impl ServerPacket for ContactListPkt {
    const OPCODE: ServerOpcodes = ServerOpcodes::ContactList;

    fn write(&self, w: &mut WorldPacket) {
        w.write_uint32(self.flags);
        w.write_bits(self.contacts.len() as u32, 8);
        w.flush_bits();
        for c in &self.contacts {
            c.write(w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_ignore_reads_cpp_name_length_account_guid_name_order() {
        let account_guid = ObjectGuid::create_player(1, 0xAABBCC);
        let mut pkt = WorldPacket::new_empty();
        pkt.write_bits(6, 9);
        pkt.write_packed_guid(&account_guid);
        pkt.write_string("Thrall");

        let parsed = AddIgnore::read(&mut pkt).expect("add ignore packet");

        assert_eq!(parsed.account_guid, account_guid);
        assert_eq!(parsed.name, "Thrall");
        assert!(pkt.is_empty());
    }

    #[test]
    fn friends_result_ignore_values_match_cpp_social_mgr() {
        assert_eq!(FriendsResult::IgnoreFull as u8, 0x0B);
        assert_eq!(FriendsResult::IgnoreSelf as u8, 0x0C);
        assert_eq!(FriendsResult::IgnoreNotFound as u8, 0x0D);
        assert_eq!(FriendsResult::IgnoreAlready as u8, 0x0E);
        assert_eq!(FriendsResult::IgnoreAdded as u8, 0x0F);
        assert_eq!(FriendsResult::IgnoreRemoved as u8, 0x10);
    }

    /// C++ `SocialPackets.cpp:103-109`: `data >> VirtualRealmAddress; data >> Guid;`.
    fn write_qualified_guid_like_cpp(pkt: &mut WorldPacket, guid: ObjectGuid, realm: u32) {
        pkt.write_uint32(realm);
        pkt.write_packed_guid(&guid);
    }

    #[test]
    fn del_ignore_reads_cpp_qualified_guid_order() {
        let player_guid = ObjectGuid::create_player(1, 0x00CCBBAA);
        let mut pkt = WorldPacket::new_empty();
        write_qualified_guid_like_cpp(&mut pkt, player_guid, 0xAABBCCDD);

        let parsed = DelIgnore::read(&mut pkt).expect("del ignore packet");

        assert_eq!(parsed.player_guid, player_guid);
        assert_eq!(parsed.virtual_realm_address, 0xAABBCCDD);
        assert!(pkt.is_empty());
    }

    #[test]
    fn qualified_guid_rejects_the_reversed_guid_first_layout() {
        // Negative: a GUID-first buffer is not what the 3.4.3 client sends; the
        // realm-address-first reader must not silently produce the same GUID.
        let player_guid = ObjectGuid::create_player(1, 0x00CCBBAA);
        let mut pkt = WorldPacket::new_empty();
        pkt.write_packed_guid(&player_guid);
        pkt.write_uint32(0xAABBCCDD);

        let parsed = DelIgnore::read(&mut pkt);
        assert!(parsed.is_err() || parsed.unwrap().player_guid != player_guid);
    }

    #[test]
    fn del_friend_reads_cpp_qualified_guid_order() {
        let player_guid = ObjectGuid::create_player(1, 0x00ABCDEF);
        let mut pkt = WorldPacket::new_empty();
        write_qualified_guid_like_cpp(&mut pkt, player_guid, 0x01000001);

        let parsed = DelFriend::read(&mut pkt).expect("del friend packet");

        assert_eq!(parsed.player_guid, player_guid);
        assert_eq!(parsed.virtual_realm_address, 0x01000001);
        assert!(pkt.is_empty());
    }

    #[test]
    fn add_friend_reads_cpp_name_length_notes_length_name_notes_order() {
        let mut pkt = WorldPacket::new_empty();
        pkt.write_bits(5, 9);
        pkt.write_bits(4, 9);
        pkt.write_string("Jaina");
        pkt.write_string("raid");

        let parsed = AddFriend::read(&mut pkt).expect("add friend packet");

        assert_eq!(parsed.name, "Jaina");
        assert_eq!(parsed.notes, "raid");
        assert!(pkt.is_empty());
    }

    #[test]
    fn add_friend_with_empty_notes_reads_like_cpp() {
        let mut pkt = WorldPacket::new_empty();
        pkt.write_bits(6, 9);
        pkt.write_bits(0, 9);
        pkt.write_string("Thrall");

        let parsed = AddFriend::read(&mut pkt).expect("add friend packet");

        assert_eq!(parsed.name, "Thrall");
        assert!(parsed.notes.is_empty());
        assert!(pkt.is_empty());
    }

    #[test]
    fn add_friend_truncated_name_is_an_error_not_a_partial_packet() {
        let mut pkt = WorldPacket::new_empty();
        pkt.write_bits(6, 9);
        pkt.write_bits(0, 9);
        pkt.write_string("Thr");

        assert!(AddFriend::read(&mut pkt).is_err());
    }

    #[test]
    fn send_contact_list_reads_uint32_flags_like_cpp() {
        let mut pkt = WorldPacket::from_bytes(&7_u32.to_le_bytes());

        let parsed = SendContactList::read(&mut pkt).expect("send contact list packet");

        assert_eq!(parsed.flags, 7);
        assert!(pkt.is_empty());
    }

    #[test]
    fn set_contact_notes_reads_cpp_qualified_guid_length_notes_order() {
        let player_guid = ObjectGuid::create_player(1, 0x102030);
        let mut pkt = WorldPacket::new_empty();
        write_qualified_guid_like_cpp(&mut pkt, player_guid, 0x01020304);
        pkt.write_bits(11, 10);
        pkt.write_string("raid leader");

        let parsed = SetContactNotes::read(&mut pkt).expect("set contact notes packet");

        assert_eq!(parsed.player_guid, player_guid);
        assert_eq!(parsed.virtual_realm_address, 0x01020304);
        assert_eq!(parsed.notes, "raid leader");
        assert!(pkt.is_empty());
    }

    #[test]
    fn friend_status_writes_cpp_field_order_with_mobile_bit() {
        let guid = ObjectGuid::create_player(1, 77);
        let packet = FriendStatusPkt {
            result: FriendsResult::Online,
            guid,
            account_guid: ObjectGuid::EMPTY,
            virtual_realm_address: 0x01000001,
            status: 0x01,
            area_id: 1519,
            level: 80,
            class_id: 8,
            notes: "hi".into(),
        };
        let mut body = WorldPacket::from_bytes(&packet.to_bytes()[2..]);

        assert_eq!(body.read_uint8().unwrap(), 0x02);
        assert_eq!(body.read_packed_guid().unwrap(), guid);
        assert_eq!(body.read_packed_guid().unwrap(), ObjectGuid::EMPTY);
        assert_eq!(body.read_uint32().unwrap(), 0x01000001);
        assert_eq!(body.read_uint8().unwrap(), 0x01);
        assert_eq!(body.read_int32().unwrap(), 1519);
        assert_eq!(body.read_int32().unwrap(), 80);
        assert_eq!(body.read_uint32().unwrap(), 8);
        assert_eq!(body.read_bits(10).unwrap(), 2);
        assert!(!body.read_bit().unwrap(), "Mobile is always false");
        assert_eq!(body.read_string(2).unwrap(), "hi");
        assert!(body.is_empty());
    }

    #[test]
    fn contact_list_writes_flags_8bit_count_then_contacts_like_cpp() {
        let guid = ObjectGuid::create_player(1, 42);
        let packet = ContactListPkt {
            flags: 1,
            contacts: vec![ContactInfo {
                guid,
                wow_account_guid: ObjectGuid::EMPTY,
                virtual_realm_address: 5,
                native_realm_address: 5,
                type_flags: 1,
                note: String::new(),
                status: 0,
                area_id: 0,
                level: 0,
                class_id: 0,
                is_mobile: false,
            }],
        };
        let mut body = WorldPacket::from_bytes(&packet.to_bytes()[2..]);

        assert_eq!(body.read_uint32().unwrap(), 1);
        assert_eq!(body.read_bits(8).unwrap(), 1);
        assert_eq!(body.read_packed_guid().unwrap(), guid);
        assert_eq!(body.read_packed_guid().unwrap(), ObjectGuid::EMPTY);
        assert_eq!(body.read_uint32().unwrap(), 5);
        assert_eq!(body.read_uint32().unwrap(), 5);
        assert_eq!(body.read_uint32().unwrap(), 1);
        assert_eq!(body.read_uint8().unwrap(), 0);
        assert_eq!(body.read_int32().unwrap(), 0);
        assert_eq!(body.read_int32().unwrap(), 0);
        assert_eq!(body.read_uint32().unwrap(), 0);
        assert_eq!(body.read_bits(10).unwrap(), 0);
        assert!(!body.read_bit().unwrap());
        assert!(body.is_empty());
    }

    #[test]
    fn social_contract_response_writes_single_false_bit_like_cpp() {
        let response = SocialContractRequestResponse {
            show_social_contract: false,
        };
        let bytes = response.to_bytes();

        assert_eq!(bytes.last().copied(), Some(0));
    }

    #[test]
    fn accept_social_contract_reads_empty_like_cpp() {
        let mut packet = WorldPacket::new_empty();
        let parsed = AcceptSocialContract::read(&mut packet).expect("accept social contract");

        assert_eq!(parsed, AcceptSocialContract);
        assert!(packet.is_empty());
    }

    #[test]
    fn account_notification_acknowledged_reads_uint32_like_cpp() {
        let mut packet = WorldPacket::from_bytes(&0xAABBCCDD_u32.to_le_bytes());
        let parsed = AccountNotificationAcknowledged::read(&mut packet)
            .expect("account notification acknowledged");

        assert_eq!(parsed.notification_id, 0xAABBCCDD);
        assert!(packet.is_empty());
    }
}
