//! Connected-presence projection consumed by the C++ social/who readers.
//!
//! `SocialMgr::GetFriendInfo` (`SocialMgr.cpp:200-247`),
//! `SocialMgr::BroadcastToFriendListers` (`SocialMgr.cpp:263-288`) and
//! `WhoListStorageMgr::Update` (`WhoListStorage.cpp:31-66`) all read the same
//! facts from a connected `Player`: identity, level, zone, AFK/DND flags, GM
//! mode and GM visibility. This module publishes them as one owned snapshot so
//! neither handler needs the directory iterator or the canonical map lock.

use super::*;

/// C++ `PLAYER_EXTRA_GM_INVISIBLE` (`Player.h:518`).
pub(crate) const PLAYER_EXTRA_GM_INVISIBLE_LIKE_CPP: u32 = 0x0010;

/// Owned presence facts of one connected incarnation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerPresenceSnapshotLikeCpp {
    pub registration: PlayerRegistration,
    pub guid: ObjectGuid,
    pub player_name: String,
    pub account_id: u32,
    pub battlenet_account_id: u32,
    /// C++ `WorldSession::GetRecruiterId()`.
    pub recruiter_id: u32,
    pub race: u8,
    pub class: u8,
    pub sex: u8,
    pub level: u8,
    /// C++ `Player::GetZoneId()`.
    pub zone_id: u32,
    /// C++ `Player::FindMap() != nullptr && !PlayerLoading()` as the who list sees it.
    pub is_in_world: bool,
    pub is_afk: bool,
    pub is_dnd: bool,
    /// C++ `Player::IsGameMaster()` (`PLAYER_EXTRA_GM_ON`).
    pub is_game_master: bool,
    /// C++ `Player::isGMVisible()`: `!(m_ExtraFlags & PLAYER_EXTRA_GM_INVISIBLE)`;
    /// `Player::IsVisible()` and `IsVisibleGloballyFor` start from this bit.
    pub is_gm_visible: bool,
    /// C++ `Player::GetGuildId()`; `None` when the character has no guild.
    pub guild_id: Option<u64>,
}

impl PlayerRegistry {
    /// Resolve the presence facts of the current incarnation of `guid`
    /// (C++ `ObjectAccessor::FindPlayer` followed by the reads above).
    #[must_use]
    pub fn presence_snapshot_like_cpp(
        &self,
        guid: ObjectGuid,
    ) -> Option<PlayerPresenceSnapshotLikeCpp> {
        let entry = self.entries.get(&guid)?;
        self.presence_snapshot(&guid, &entry)
    }

    /// Snapshot every connected incarnation (C++ `ObjectAccessor::GetPlayers()`
    /// as iterated by `WhoListStorageMgr::Update`). The returned order is not
    /// significant, exactly as the C++ hash map order is not.
    #[must_use]
    pub fn presence_snapshots_like_cpp(&self) -> Vec<PlayerPresenceSnapshotLikeCpp> {
        let guids: Vec<_> = self.entries.iter().map(|entry| *entry.key()).collect();
        guids
            .into_iter()
            .filter_map(|guid| self.presence_snapshot_like_cpp(guid))
            .collect()
    }

    fn presence_snapshot(
        &self,
        guid: &ObjectGuid,
        entry: &PlayerRegistryEntry,
    ) -> Option<PlayerPresenceSnapshotLikeCpp> {
        let (is_afk, is_dnd, is_game_master, is_gm_visible, zone_id, guild_id) = self
            .canonical_at(
                *guid,
                entry.placement.map_id,
                entry.placement.instance_id,
                |player| {
                    (
                        player.has_player_flag(crate::session::PLAYER_FLAGS_AFK_LIKE_CPP),
                        player.has_player_flag(crate::session::PLAYER_FLAGS_DND_LIKE_CPP),
                        player.is_game_master_like_cpp(),
                        player.extra_flags() & PLAYER_EXTRA_GM_INVISIBLE_LIKE_CPP == 0,
                        player.gameplay_state().world_local.zone_id_like_cpp(),
                        player.guild_state_like_cpp().guild_id,
                    )
                },
            )?;
        Some(PlayerPresenceSnapshotLikeCpp {
            registration: PlayerRegistration {
                guid: *guid,
                generation: entry.generation,
            },
            guid: *guid,
            player_name: entry.identity.player_name.clone(),
            account_id: entry.identity.account_id,
            battlenet_account_id: entry.identity.battlenet_account_id,
            recruiter_id: entry.identity.recruiter_id,
            race: entry.identity.race,
            class: entry.identity.class,
            sex: entry.identity.sex,
            level: entry.placement.level,
            zone_id,
            is_in_world: entry.placement.is_in_world,
            is_afk,
            is_dnd,
            is_game_master,
            is_gm_visible,
            guild_id,
        })
    }
}
