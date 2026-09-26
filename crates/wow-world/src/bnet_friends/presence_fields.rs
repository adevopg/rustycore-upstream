//! Every Battle.net presence field id, friend role id and attribute name the
//! friends manager emits or reads, in one place.
//!
//! Evidence levels:
//! - CONFIRMED: `presence_types.pb.h` (`FieldKey{program, group, field,
//!   unique_id}`), `friends_types.pb.h` (`Friend.role`, `SubscribeResponse.role`)
//!   and the Battle.net 2 presence layout used by every WoW 6.x+ Battle.net
//!   reimplementation (`program "BN"`, group 1 = account, group 2 = game
//!   account; account 1 full_name / 3 game accounts / 4 battle_tag / 6
//!   last_online; game account 1 online / 3 program / 4 last_online / 5 name /
//!   7 owner account).
//! - ASSUMED: not verified against a 3.4.3.54261 capture yet. The manager logs
//!   every incoming `PresenceService.Query` key and `PresenceService.Update`
//!   operation at debug level so a live session can pin them down; fix the
//!   constant here and nothing else.

use wow_proto::bgs::protocol::presence::v1::FieldKey;

/// `FieldKey.program` of the Battle.net account / game account groups (`"BN"`).
/// CONFIRMED.
pub const PROGRAM_BN_LIKE_CPP: u32 = 0x424E;
/// `FieldKey.program` of the WoW-specific rich presence group (`"WoW"` fourcc).
/// CONFIRMED as the WoW program id (`GameLevelInfo.program` = 5730135); its
/// field layout below is ASSUMED.
pub const PROGRAM_WOW_LIKE_CPP: u32 = 0x0057_6F57;

/// `FieldKey.group` of account fields. CONFIRMED.
pub const GROUP_ACCOUNT_LIKE_CPP: u32 = 1;
/// `FieldKey.group` of game account fields. CONFIRMED.
pub const GROUP_GAME_ACCOUNT_LIKE_CPP: u32 = 2;

// ---- account entity (program BN, group 1) --------------------------------

/// Real ID full name (string). CONFIRMED. Never sent: RustyCore has no Real ID.
pub const ACCOUNT_FIELD_FULL_NAME_LIKE_CPP: u32 = 1;
/// One entry per online game account (`Variant.entity_id_value`), the list
/// index in `FieldKey.unique_id`. CONFIRMED.
pub const ACCOUNT_FIELD_GAME_ACCOUNT_LIKE_CPP: u32 = 3;
/// BattleTag (string). CONFIRMED.
pub const ACCOUNT_FIELD_BATTLE_TAG_LIKE_CPP: u32 = 4;
/// Unix time of the last logout (int) while no game account is online. CONFIRMED.
pub const ACCOUNT_FIELD_LAST_ONLINE_LIKE_CPP: u32 = 6;
/// Away (bool). ASSUMED (7/8/10 unconfirmed; the client's own `Update` reveals them).
pub const ACCOUNT_FIELD_AWAY_LIKE_CPP: u32 = 7;
/// Unix time the account went away (int). ASSUMED.
pub const ACCOUNT_FIELD_AWAY_TIME_LIKE_CPP: u32 = 8;
/// Busy / do-not-disturb (bool). ASSUMED.
pub const ACCOUNT_FIELD_BUSY_LIKE_CPP: u32 = 10;

// ---- game account entity (program BN, group 2) ---------------------------

/// Online (bool). CONFIRMED.
pub const GAME_ACCOUNT_FIELD_ONLINE_LIKE_CPP: u32 = 1;
/// Program fourcc (`Variant.fourcc_value = "WoW"`). CONFIRMED.
pub const GAME_ACCOUNT_FIELD_PROGRAM_LIKE_CPP: u32 = 3;
/// Unix time of the last logout (int). CONFIRMED.
pub const GAME_ACCOUNT_FIELD_LAST_ONLINE_LIKE_CPP: u32 = 4;
/// Game account display name; the BattleTag is what retail shows (string). CONFIRMED.
pub const GAME_ACCOUNT_FIELD_NAME_LIKE_CPP: u32 = 5;
/// Owning account (`Variant.entity_id_value`). CONFIRMED.
pub const GAME_ACCOUNT_FIELD_ACCOUNT_ID_LIKE_CPP: u32 = 7;

// ---- WoW rich presence (program WoW, group 2) — every id ASSUMED ----------

/// Character name (string). ASSUMED.
pub const WOW_FIELD_CHARACTER_NAME_LIKE_CPP: u32 = 1;
/// Realm name (string). ASSUMED.
pub const WOW_FIELD_REALM_NAME_LIKE_CPP: u32 = 2;
/// Virtual realm address (int). ASSUMED.
pub const WOW_FIELD_REALM_ADDRESS_LIKE_CPP: u32 = 3;
/// Faction (int, `ChrRaces.Alliance`: 1 Alliance / 0 Horde). ASSUMED.
pub const WOW_FIELD_FACTION_LIKE_CPP: u32 = 4;
/// Race id (int). ASSUMED.
pub const WOW_FIELD_RACE_LIKE_CPP: u32 = 5;
/// Class id (int). ASSUMED.
pub const WOW_FIELD_CLASS_LIKE_CPP: u32 = 6;
/// Level (int). ASSUMED.
pub const WOW_FIELD_LEVEL_LIKE_CPP: u32 = 7;
/// Zone id (int). ASSUMED.
pub const WOW_FIELD_ZONE_ID_LIKE_CPP: u32 = 8;
/// AFK (bool). ASSUMED.
pub const WOW_FIELD_AFK_LIKE_CPP: u32 = 9;
/// DND (bool). ASSUMED.
pub const WOW_FIELD_DND_LIKE_CPP: u32 = 10;

/// Faction values of [`WOW_FIELD_FACTION_LIKE_CPP`]. ASSUMED.
pub const WOW_FACTION_HORDE_LIKE_CPP: i64 = 0;
pub const WOW_FACTION_ALLIANCE_LIKE_CPP: i64 = 1;

/// `Friend.role` / `SubscribeResponse.role[].id` of a BattleTag friendship.
/// CONFIRMED as the only value the client offers for a BattleTag invitation
/// (`FriendInvitationParams.role` valid range 1..1); the role *names* are ASSUMED.
pub const ROLE_BATTLE_TAG_FRIEND_LIKE_CPP: u32 = 1;
pub const ROLE_BATTLE_TAG_FRIEND_NAME_LIKE_CPP: &str = "battle_tag_friend";
/// Real ID friendship role, listed so the client's role table is complete. ASSUMED.
pub const ROLE_REAL_ID_FRIEND_LIKE_CPP: u32 = 2;
pub const ROLE_REAL_ID_FRIEND_NAME_LIKE_CPP: &str = "real_id_friend";

/// `UpdateFriendStateRequest.attribute[].name` of the friend note. ASSUMED: any
/// incoming attribute whose name contains `note` is accepted as the note and its
/// exact name is echoed back, so the constant only matters for the initial
/// `SubscribeResponse`.
pub const FRIEND_NOTE_ATTRIBUTE_NAME_LIKE_CPP: &str = "friend_note";
/// `battlenet_account_friends.note` column width.
pub const FRIEND_NOTE_MAX_LENGTH_LIKE_CPP: usize = 127;

/// `SubscribeResponse.max_friends` and friends (LegionCore `FriendsMgr` limits).
pub const MAX_FRIENDS_LIKE_CPP: u32 = 200;
pub const MAX_RECEIVED_INVITATIONS_LIKE_CPP: u32 = 20;
pub const MAX_SENT_INVITATIONS_LIKE_CPP: u32 = 20;

/// `EntityId.high` of a Battle.net account (`authentication.rs` LogonResult).
pub const ACCOUNT_ENTITY_HIGH_LIKE_CPP: u64 = 0x0100_0000_0000_0000;
/// `EntityId.high` of a WoW game account (`"WoW"` in the high bits).
pub const GAME_ACCOUNT_ENTITY_HIGH_LIKE_CPP: u64 = 0x0200_0002_0057_6F57;

/// Hashable/orderable form of `presence.v1.FieldKey`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FieldKeyLikeCpp {
    pub program: u32,
    pub group: u32,
    pub field: u32,
    pub unique_id: u64,
}

impl FieldKeyLikeCpp {
    pub const fn new(program: u32, group: u32, field: u32) -> Self {
        Self {
            program,
            group,
            field,
            unique_id: 0,
        }
    }

    pub const fn with_unique_id(mut self, unique_id: u64) -> Self {
        self.unique_id = unique_id;
        self
    }

    pub const fn account(field: u32) -> Self {
        Self::new(PROGRAM_BN_LIKE_CPP, GROUP_ACCOUNT_LIKE_CPP, field)
    }

    pub const fn game_account(field: u32) -> Self {
        Self::new(PROGRAM_BN_LIKE_CPP, GROUP_GAME_ACCOUNT_LIKE_CPP, field)
    }

    pub const fn wow(field: u32) -> Self {
        Self::new(PROGRAM_WOW_LIKE_CPP, GROUP_GAME_ACCOUNT_LIKE_CPP, field)
    }

    pub fn to_proto(self) -> FieldKey {
        FieldKey {
            program: self.program,
            group: self.group,
            field: self.field,
            unique_id: (self.unique_id != 0).then_some(self.unique_id),
        }
    }

    pub fn from_proto(key: &FieldKey) -> Self {
        Self {
            program: key.program,
            group: key.group,
            field: key.field,
            unique_id: key.unique_id.unwrap_or(0),
        }
    }

    /// `"BN"/1/4` style rendering for the discovery logs.
    pub fn describe(self) -> String {
        let program = match self.program {
            PROGRAM_BN_LIKE_CPP => "BN".to_owned(),
            PROGRAM_WOW_LIKE_CPP => "WoW".to_owned(),
            other => format!("0x{other:X}"),
        };
        if self.unique_id != 0 {
            format!("{program}/{}/{}#{}", self.group, self.field, self.unique_id)
        } else {
            format!("{program}/{}/{}", self.group, self.field)
        }
    }

    /// Whether `filter` (a `Query`/`Subscribe` key list) selects this key: an
    /// empty filter selects everything, and a filter key with `unique_id` 0
    /// selects every list entry of that field.
    pub fn selected_by(self, filter: &[Self]) -> bool {
        filter.is_empty()
            || filter.iter().any(|key| {
                key.program == self.program
                    && key.group == self.group
                    && key.field == self.field
                    && (key.unique_id == 0 || key.unique_id == self.unique_id)
            })
    }
}
