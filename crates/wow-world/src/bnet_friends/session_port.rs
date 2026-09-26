//! What the friends manager needs from a world session, and the production
//! [`WorldSession`] implementation.

use wow_constants::Team;

use super::presence_fields::{WOW_FACTION_ALLIANCE_LIKE_CPP, WOW_FACTION_HORDE_LIKE_CPP};
use crate::session::WorldSession;

/// LegionCore `FriendsMgr::BuildGameAccountPresence` inputs
/// (`FillStatus` / `FillZoneAndLevel`): the character currently in the world.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BnetGameAccountPresenceSnapshotLikeCpp {
    pub character_name: String,
    pub level: u8,
    pub class: u8,
    pub race: u8,
    /// [`WOW_FACTION_ALLIANCE_LIKE_CPP`] / [`WOW_FACTION_HORDE_LIKE_CPP`].
    pub faction: i64,
    pub zone_id: u32,
    pub realm_name: String,
    /// `(Region << 24) | (Battlegroup << 16) | RealmId`.
    pub realm_address: u32,
    pub afk: bool,
    pub dnd: bool,
}

/// The agent of a Battle.net request: who asked, from which game account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BnetAgentLikeCpp {
    pub account_id: u32,
    pub game_account_id: u32,
}

/// Session capability of the friends manager. `WorldSession` implements it;
/// tests use fakes.
pub trait BnetFriendsSessionLikeCpp {
    /// `WorldSession::GetBattlenetAccountId()`.
    fn battlenet_account_id_like_cpp(&self) -> u32;
    /// `WorldSession::GetAccountId()` (the game account).
    fn game_account_id_like_cpp(&self) -> u32;
    /// The session's packet channel; the manager keeps a clone so friends'
    /// sessions can push `SMSG_BATTLENET_NOTIFICATION` from any thread, like
    /// C++ `WorldSession::SendBattlenetRequest` from `FriendsMgr`.
    fn packet_sender_like_cpp(&self) -> flume::Sender<Vec<u8>>;
    /// `None` while no character is in the world (character select).
    fn game_account_presence_like_cpp(&self) -> Option<BnetGameAccountPresenceSnapshotLikeCpp>;

    fn bnet_agent_like_cpp(&self) -> BnetAgentLikeCpp {
        BnetAgentLikeCpp {
            account_id: self.battlenet_account_id_like_cpp(),
            game_account_id: self.game_account_id_like_cpp(),
        }
    }
}

pub(crate) fn faction_of_race_like_cpp(race: u8) -> i64 {
    match crate::session::player_team_for_race_cpp(race) {
        Team::Horde => WOW_FACTION_HORDE_LIKE_CPP,
        Team::Alliance | Team::Other => WOW_FACTION_ALLIANCE_LIKE_CPP,
    }
}

impl BnetFriendsSessionLikeCpp for WorldSession {
    fn battlenet_account_id_like_cpp(&self) -> u32 {
        self.battlenet_account_id()
    }

    fn game_account_id_like_cpp(&self) -> u32 {
        self.account_id
    }

    fn packet_sender_like_cpp(&self) -> flume::Sender<Vec<u8>> {
        self.send_tx().clone()
    }

    fn game_account_presence_like_cpp(&self) -> Option<BnetGameAccountPresenceSnapshotLikeCpp> {
        self.player_guid()?;
        let character_name = self.player_name_like_cpp()?;
        let race = self.player_race_like_cpp();
        // AFK/DND live on the Player; the connected-player directory projects
        // them for chat/social lookups (`PlayerSocialRecipientSnapshot`).
        let (afk, dnd) = self
            .player_registry()
            .and_then(|registry| registry.social_recipient_by_name(&character_name))
            .map_or((false, false), |recipient| {
                (recipient.is_afk, recipient.is_dnd)
            });
        let realm_address = self.virtual_realm_address();
        Some(BnetGameAccountPresenceSnapshotLikeCpp {
            character_name,
            level: self.player_level_like_cpp(),
            class: self.player_class_like_cpp(),
            race,
            faction: faction_of_race_like_cpp(race),
            zone_id: self
                .player_zone_area_like_cpp()
                .map_or(0, |(zone, _area)| zone),
            realm_name: self
                .realm_names_for_address_like_cpp(realm_address)
                .map(|(name, _normalized)| name.to_owned())
                .unwrap_or_default(),
            realm_address,
            afk,
            dnd,
        })
    }
}
