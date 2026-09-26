// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! CMSG_WHO (0x3683) → SMSG_WHO (0x2BAE).
//!
//! C++ anchors (TrinityCore 3.4.3):
//! - `src/server/game/Handlers/MiscHandler.cpp:85-236` — `WorldSession::HandleWhoOpcode`
//!   (limits, filters, visibility gates, `CONFIG_MAX_WHO` cap).
//! - `src/server/game/Storages/WhoListStorage.cpp:31-66` — `WhoListStorageMgr::Update`
//!   (which connected players enter the list and which facts each row carries).
//! - `src/server/game/Server/Protocol/Opcodes.cpp:1016` — `STATUS_LOGGEDIN, PROCESS_THREADSAFE`.
//! - `src/server/game/World/World.cpp:1563` — `MaxWhoListReturns` default 49;
//!   `World.cpp:1150` — `GM.InWhoList.Level` default `SEC_ADMINISTRATOR`.
//! - `src/server/game/DataStores/DBCEnums.h:51,55` — `MAX_LEVEL = 123`, `STRONG_MAX_LEVEL = 255`.
//! - `src/server/game/Miscellaneous/RaceMask.h:88-141` — `RaceMask::HasRace`/`GetRaceBit`.
//!
//! Explicit departure (behavior-equivalent): C++ answers from `sWhoListStorageMgr`, a
//! snapshot the world loop rebuilds every `CONFIG_WHO_LIST_UPDATE` interval. RustyCore has
//! no such periodic storage; the snapshot is built on demand from the [`PlayerRegistry`]
//! (`presence_snapshots_like_cpp`), so a request sees the live state instead of a state up
//! to one refresh interval old. Row membership (`FindMap()` and not `PlayerLoading()`) maps
//! to the registry's `is_in_world` placement flag.
//!
//! Represented gaps, each mirrored by an empty value rather than invented data:
//! - guild name/GUID: no `GuildMgr` exists, so every row carries an empty guild; a
//!   non-empty `Guild` filter therefore matches nobody, exactly as C++ would with empty names;
//! - `Words` are matched against the player name and guild name; the `AreaTable` reader
//!   carries no localized `AreaName`, so the third C++ term (`Utf8FitTo(areaName, word)`)
//!   cannot match;
//! - the account security of *other* sessions is not published by the directory and is
//!   represented as `SEC_PLAYER`; `RBAC_PERM_TWO_SIDE_WHO_LIST` / `RBAC_PERM_WHO_SEE_ALL_SEC_LEVELS`
//!   are represented as not granted (normal-player behavior);
//! - `VirtualRealmName`, `ShowEnemies`, `ShowArenaPlayers`, `ExactName` and `ServerInfo` are
//!   parsed and ignored, as the C++ handler's `@todo` block does.
//!
//! `CMSG_WHO_IS` (GM account lookup) is out of scope.

use wow_constants::ClientOpcodes;
use wow_core::ObjectGuid;
use wow_core::guid::HighGuid;
use wow_handler::{PacketProcessing, SessionStatus};
use wow_packet::ClientPacket;
use wow_packet::packets::query::PlayerGuidLookupData;
use wow_packet::packets::who::{WhoEntry, WhoRequestPkt, WhoResponsePkt};

use crate::handlers::social::{GM_LEVEL_IN_WHO_LIST_LIKE_CPP, SEC_PLAYER_LIKE_CPP};
use crate::session::directory::PlayerPresenceSnapshotLikeCpp;
use crate::session::registry::PacketHandlerEntry;
use crate::session::{WorldSession, player_team_for_race_cpp};

/// C++ `CONFIG_MAX_WHO` default (`World.cpp:1563`, "MaxWhoListReturns").
pub(crate) const MAX_WHO_LIKE_CPP: usize = 49;
/// "zones count, client limit = 10 (2.0.10)".
const MAX_WHO_AREAS_LIKE_CPP: usize = 10;
/// "user entered strings count, client limit=4 (checked on 2.0.10)".
const MAX_WHO_WORDS_LIKE_CPP: usize = 4;
/// `DBCEnums.h:51`.
const MAX_LEVEL_LIKE_CPP: i32 = 123;
/// `DBCEnums.h:55`.
const STRONG_MAX_LEVEL_LIKE_CPP: i32 = 255;

/// C++ `Trinity::RaceMask::GetRaceBit` (`RaceMask.h:93-135`).
pub(crate) fn race_bit_like_cpp(race: u8) -> Option<u32> {
    match race {
        // RACE_HUMAN..RACE_DRAENEI, RACE_WORGEN, RACE_PANDAREN_* .. RACE_KUL_TIRAN
        1..=11 | 22 | 24..=32 => Some(u32::from(race) - 1),
        34 => Some(11), // RACE_DARK_IRON_DWARF
        35 => Some(12), // RACE_VULPERA
        36 => Some(13), // RACE_MAGHAR_ORC
        37 => Some(14), // RACE_MECHAGNOME
        52 => Some(16), // RACE_DRACTHYR_ALLIANCE
        70 => Some(15), // RACE_DRACTHYR_HORDE
        _ => None,
    }
}

/// C++ `Trinity::RaceMask<int64>::HasRace` (`RaceMask.h:88-91,137-141`).
pub(crate) fn race_mask_has_race_like_cpp(raw: i64, race: u8) -> bool {
    race_bit_like_cpp(race).is_some_and(|bit| bit < 64 && raw & (1_i64 << bit) != 0)
}

/// The request facts `HandleWhoOpcode` derives before scanning the list
/// (strings already lowered, `MaxLevel` already widened).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WhoFilterLikeCpp {
    pub min_level: i32,
    pub max_level: i32,
    pub class_filter: i32,
    pub race_filter: i64,
    pub areas: Vec<i32>,
    pub name: String,
    pub guild: String,
    pub words: Vec<String>,
}

impl WhoFilterLikeCpp {
    pub(crate) fn from_request_like_cpp(packet: &WhoRequestPkt) -> Self {
        let request = &packet.request;
        // client send in case not set max level value 100 but Trinity supports 255 max level,
        // update it to show GMs with characters after 100 level
        let max_level = if request.max_level >= MAX_LEVEL_LIKE_CPP {
            STRONG_MAX_LEVEL_LIKE_CPP
        } else {
            request.max_level
        };
        Self {
            min_level: request.min_level,
            max_level,
            class_filter: request.class_filter,
            race_filter: request.race_filter,
            areas: packet.areas.clone(),
            name: request.name.to_lowercase(),
            guild: request.guild.to_lowercase(),
            words: request
                .words
                .iter()
                .map(|word| word.to_lowercase())
                .collect(),
        }
    }
}

/// The observing session facts `HandleWhoOpcode` reads from `_player`/`this`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WhoObserverLikeCpp {
    pub guid: ObjectGuid,
    pub race: u8,
    pub security: u8,
}

/// The per-row gates of `HandleWhoOpcode` (`MiscHandler.cpp:143-217`) applied to
/// one `WhoListPlayerInfo`, plus the `WhoListStorageMgr::Update` membership test.
pub(crate) fn who_target_matches_like_cpp(
    filter: &WhoFilterLikeCpp,
    observer: &WhoObserverLikeCpp,
    target: &PlayerPresenceSnapshotLikeCpp,
    target_security: u8,
    target_guild_name_lower: &str,
) -> bool {
    // WhoListStorageMgr::Update: `if (!FindMap() || PlayerLoading()) continue;`
    if !target.is_in_world {
        return false;
    }

    // player can see member of other team only if has RBAC_PERM_TWO_SIDE_WHO_LIST
    if player_team_for_race_cpp(target.race) != player_team_for_race_cpp(observer.race) {
        return false;
    }

    // player can see MODERATOR, GAME MASTER, ADMINISTRATOR only if has RBAC_PERM_WHO_SEE_ALL_SEC_LEVELS
    if target_security > GM_LEVEL_IN_WHO_LIST_LIKE_CPP {
        return false;
    }

    // check if target is globally visible for player
    if observer.guid != target.guid
        && !target.is_gm_visible
        && (observer.security == SEC_PLAYER_LIKE_CPP || target_security > observer.security)
    {
        return false;
    }

    // check if target's level is in level range
    let level = i32::from(target.level);
    if level < filter.min_level || level > filter.max_level {
        return false;
    }

    // check if class matches classmask
    if filter.class_filter >= 0
        && 1_i32
            .checked_shl(u32::from(target.class))
            .is_none_or(|bit| filter.class_filter & bit == 0)
    {
        return false;
    }

    // check if race matches racemask
    if !race_mask_has_race_like_cpp(filter.race_filter, target.race) {
        return false;
    }

    if !filter.areas.is_empty() && !filter.areas.contains(&(target.zone_id as i32)) {
        return false;
    }

    let target_name = target.player_name.to_lowercase();
    if !(filter.name.is_empty() || target_name.contains(filter.name.as_str())) {
        return false;
    }

    if !filter.guild.is_empty() && !target_guild_name_lower.contains(filter.guild.as_str()) {
        return false;
    }

    if !filter.words.is_empty() {
        // Area-name matching (`Utf8FitTo(aName, word)`) is a represented gap: see module doc.
        let show = filter.words.iter().any(|word| {
            !word.is_empty()
                && (target_name.contains(word.as_str())
                    || target_guild_name_lower.contains(word.as_str()))
        });
        if !show {
            return false;
        }
    }

    true
}

/// C++ `WhoEntry` construction (`MiscHandler.cpp:219-232`) with
/// `PlayerGuidLookupData::Initialize(guid, nullptr)` (`QueryPackets.cpp:172-207`).
pub(crate) fn who_entry_like_cpp(
    target: &PlayerPresenceSnapshotLikeCpp,
    virtual_realm_address: u32,
) -> WhoEntry {
    WhoEntry {
        player_data: PlayerGuidLookupData {
            is_deleted: false,
            account_id: ObjectGuid::new(
                (HighGuid::WowAccount as i64) << 58,
                i64::from(target.account_id),
            ),
            bnet_account_id: ObjectGuid::new(
                (HighGuid::BNetAccount as i64) << 58,
                i64::from(target.battlenet_account_id),
            ),
            guid_actual: target.guid,
            guild_club_member_id: 0,
            virtual_realm_address,
            race: target.race,
            sex: target.sex,
            class: target.class,
            level: target.level,
            name: target.player_name.clone(),
            declined_names: Default::default(),
        },
        // Represented gap: no GuildMgr, so the guild block stays absent (see module doc).
        guild_guid: ObjectGuid::EMPTY,
        guild_virtual_realm_address: 0,
        guild_name: String::new(),
        area_id: target.zone_id as i32,
        is_gm: target.is_game_master,
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::Who,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::ThreadSafe,
        handler_name: "handle_who",
        handler: |session, _catalogs, mut pkt| {
            Box::pin(async move {
                match WhoRequestPkt::read(&mut pkt) {
                    Ok(request) => session.handle_who(request).await,
                    Err(e) => tracing::warn!("Failed to read WhoRequestPkt: {e}"),
                }
            })
        },
    }
}

impl WorldSession {
    /// C++ `WorldSession::HandleWhoOpcode` (`MiscHandler.cpp:85-236`).
    pub async fn handle_who(&mut self, packet: WhoRequestPkt) {
        // zones count, client limit = 10 (2.0.10)
        // can't be received from real client or broken packet
        if packet.areas.len() > MAX_WHO_AREAS_LIKE_CPP {
            return;
        }

        // user entered strings count, client limit=4 (checked on 2.0.10)
        // can't be received from real client or broken packet
        if packet.request.words.len() > MAX_WHO_WORDS_LIKE_CPP {
            return;
        }

        let Some(my_guid) = self.player_guid() else {
            return;
        };

        let filter = WhoFilterLikeCpp::from_request_like_cpp(&packet);
        let observer = WhoObserverLikeCpp {
            guid: my_guid,
            race: self.player_race_like_cpp(),
            security: self.security,
        };
        let virtual_realm_address = self.virtual_realm_address();

        let mut response = WhoResponsePkt {
            request_id: packet.request_id,
            entries: Vec::new(),
        };

        if let Some(registry) = self.player_registry() {
            for target in registry.presence_snapshots_like_cpp() {
                if !who_target_matches_like_cpp(
                    &filter,
                    &observer,
                    &target,
                    SEC_PLAYER_LIKE_CPP,
                    "",
                ) {
                    continue;
                }

                response
                    .entries
                    .push(who_entry_like_cpp(&target, virtual_realm_address));

                // 50 is maximum player count sent to client - can be overridden
                // through config, but is unstable
                if response.entries.len() >= MAX_WHO_LIKE_CPP {
                    break;
                }
            }
        }

        // Opcodes.cpp:2235 routes SMSG_WHO to CONNECTION_TYPE_REALM.
        self.send_packet_realm(&response);
    }
}

#[cfg(test)]
#[path = "who/tests.rs"]
mod tests;
