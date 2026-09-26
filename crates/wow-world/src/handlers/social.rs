// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Handlers for social opcodes: AddFriend, AddIgnore, DelFriend, DelIgnore, SendContactList,
//! SetContactNotes, SocialContractRequest; plus the `SMSG_FRIEND_STATUS` presence
//! notifications C++ emits outside the social handler (login, logout, character delete).
//!
//! C++ anchors (TrinityCore 3.4.3):
//! - `src/server/game/Handlers/SocialHandler.cpp` — the opcode handlers.
//! - `src/server/game/Entities/Player/SocialMgr.cpp` — `PlayerSocial::SendSocialList`
//!   :140-171, `SocialMgr::GetFriendInfo` :200-247, `SocialMgr::SendFriendStatus`
//!   :249-261, `SocialMgr::BroadcastToFriendListers` :263-288.
//! - `src/server/game/Entities/Player/Player.cpp:23036-23055` — `Player::IsVisibleGloballyFor`.
//! - `CharacterHandler.cpp:1224` (login `FRIEND_ONLINE` broadcast), `WorldSession.cpp:651`
//!   (logout `FRIEND_OFFLINE` broadcast), `Player.cpp:3953-3966` (delete → `FRIEND_REMOVED`).
//!
//! Model departure (explicit contract): RustyCore keeps no in-memory `SocialMgr::_socialMap`.
//! `BroadcastToFriendListers` resolves its listers through the persistence port
//! (`SEL_CHAR_SOCIAL` reverse lookup, friend flag applied in SQL) and delivers to the
//! sessions the [`PlayerRegistry`] still holds, with the same per-lister gates.
//!
//! Represented gaps (tracked, not silently invented): RBAC (`RBAC_PERM_WHO_SEE_ALL_SEC_LEVELS`,
//! `RBAC_PERM_TWO_SIDE_WHO_LIST`, `RBAC_PERM_TWO_SIDE_ADD_FRIEND`, `RBAC_PERM_ALLOW_GM_FRIEND`)
//! is represented as "not granted" (normal-player behavior); the account security of *other*
//! connected sessions is not published by the directory, so it is represented as `SEC_PLAYER`
//! wherever C++ compares it (`GM.InWhoList.Level` and `IsVisibleGloballyFor` GM-vs-GM
//! branch). The observing session's own security is exact.

use tracing::{info, warn};
use wow_constants::ClientOpcodes;
use wow_core::ObjectGuid;
use wow_handler::{PacketProcessing, SessionStatus};
use wow_packet::ClientPacket;
use wow_packet::ServerPacket;

use crate::session::directory::PlayerPresenceSnapshotLikeCpp;
use crate::session::registry::PacketHandlerEntry;
use wow_packet::packets::social::{
    AcceptSocialContract, AccountNotificationAcknowledged, AddFriend, AddIgnore, ContactInfo,
    ContactListPkt, DelFriend, DelIgnore, FriendStatusPkt, FriendsResult, SendContactList,
    SetContactNotes, SocialContractRequestResponse,
};

use crate::session::{WorldSession, player_team_for_race_cpp};
use wow_persistence::{
    PersistenceOutcomeLikeCpp, SocialAddCandidateLoadOutcomeLikeCpp,
    SocialContactListLoadOutcomeLikeCpp, SocialRelationshipKindLikeCpp,
};

/// C++ `FriendStatus` (`SocialMgr.h:29-36`).
const FRIEND_STATUS_OFFLINE_LIKE_CPP: u8 = 0x00;
const FRIEND_STATUS_ONLINE_LIKE_CPP: u8 = 0x01;
const FRIEND_STATUS_AFK_LIKE_CPP: u8 = 0x02;
const FRIEND_STATUS_DND_LIKE_CPP: u8 = 0x04;
const FRIEND_STATUS_RAF_LIKE_CPP: u8 = 0x08;

/// C++ `SocialFlag` (`SocialMgr.h:38-46`).
pub(crate) const SOCIAL_FLAG_FRIEND_LIKE_CPP: u32 = 0x01;
pub(crate) const SOCIAL_FLAG_IGNORED_LIKE_CPP: u32 = 0x02;
pub(crate) const SOCIAL_FLAG_ALL_LIKE_CPP: u32 = 0x07;

/// C++ `SOCIALMGR_FRIEND_LIMIT` / `SOCIALMGR_IGNORE_LIMIT` (`SocialMgr.h:88-89`).
const SOCIALMGR_FRIEND_LIMIT_LIKE_CPP: u32 = 50;
const SOCIALMGR_IGNORE_LIMIT_LIKE_CPP: u32 = 50;

/// C++ `World.cpp:1150`: `GM.InWhoList.Level` defaults to `SEC_ADMINISTRATOR` (3).
pub(crate) const GM_LEVEL_IN_WHO_LIST_LIKE_CPP: u8 = 3;
/// C++ `SEC_PLAYER` (`Common.h:40`); `AccountMgr::IsPlayerAccount(x)` is `x == SEC_PLAYER`.
pub(crate) const SEC_PLAYER_LIKE_CPP: u8 = 0;

fn normalize_player_name_like_cpp(name: &str) -> Option<String> {
    let mut lowered = String::new();
    for ch in name.chars() {
        lowered.extend(ch.to_lowercase());
    }

    let mut chars = lowered.chars();
    let first = chars.next()?;
    let mut normalized = String::new();
    normalized.extend(first.to_uppercase());
    normalized.extend(chars);
    Some(normalized)
}

/// The facts C++ reads from the observing `Player`/`WorldSession` inside
/// `SocialMgr::GetFriendInfo` and `BroadcastToFriendListers`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SocialObserverLikeCpp {
    pub guid: ObjectGuid,
    pub race: u8,
    /// `WorldSession::GetSecurity()` of the observer — exact for the own session.
    pub security: u8,
    pub account_id: u32,
    pub recruiter_id: u32,
}

/// C++ `FriendInfo` after `SocialMgr::GetFriendInfo` filled it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct FriendInfoLikeCpp {
    pub status: u8,
    pub area: u32,
    pub level: u32,
    pub class: u32,
    pub note: String,
}

/// C++ `Player::IsVisibleGloballyFor(Player const* u)` (`Player.cpp:23036-23055`),
/// where `target` is `this` and the observer is `u`.
pub(crate) fn is_visible_globally_for_like_cpp(
    target: &PlayerPresenceSnapshotLikeCpp,
    target_security: u8,
    observer_guid: ObjectGuid,
    observer_security: u8,
) -> bool {
    // Always can see self
    if target.guid == observer_guid {
        return true;
    }
    // Visible units, always are visible for all players
    if target.is_gm_visible {
        return true;
    }
    // GMs are visible for higher gms (or players are visible for gms)
    if observer_security != SEC_PLAYER_LIKE_CPP {
        return target_security <= observer_security;
    }
    // non faction visibility non-breakable for non-GMs
    false
}

/// C++ `SocialMgr::GetFriendInfo` (`SocialMgr.cpp:200-247`).
///
/// `target` is `ObjectAccessor::FindPlayer(friendGUID)`; `note_when_online` is the
/// observer's stored note for that contact, which C++ copies only after the
/// online lookup succeeded (the offline early return happens before it).
pub(crate) fn friend_info_like_cpp(
    observer: &SocialObserverLikeCpp,
    target: Option<&PlayerPresenceSnapshotLikeCpp>,
    note_when_online: &str,
) -> FriendInfoLikeCpp {
    let mut info = FriendInfoLikeCpp {
        status: FRIEND_STATUS_OFFLINE_LIKE_CPP,
        area: 0,
        level: 0,
        class: 0,
        note: String::new(),
    };
    let Some(target) = target else {
        return info;
    };
    info.note = note_when_online.to_owned();

    // PLAYER see his team only and PLAYER can't see MODERATOR, GAME MASTER, ADMINISTRATOR characters
    // MODERATOR, GAME MASTER, ADMINISTRATOR can see all
    // (RBAC_PERM_WHO_SEE_ALL_SEC_LEVELS represented as not granted; target security as SEC_PLAYER.)
    let target_security = SEC_PLAYER_LIKE_CPP;
    if target_security > GM_LEVEL_IN_WHO_LIST_LIKE_CPP {
        return info;
    }

    // player can see member of other team only if CONFIG_ALLOW_TWO_SIDE_WHO_LIST
    // (RBAC_PERM_TWO_SIDE_WHO_LIST represented as not granted.)
    if player_team_for_race_cpp(target.race) != player_team_for_race_cpp(observer.race) {
        return info;
    }

    if is_visible_globally_for_like_cpp(target, target_security, observer.guid, observer.security) {
        if target.is_dnd {
            info.status = FRIEND_STATUS_DND_LIKE_CPP;
        } else if target.is_afk {
            info.status = FRIEND_STATUS_AFK_LIKE_CPP;
        } else {
            info.status = FRIEND_STATUS_ONLINE_LIKE_CPP;
            if target.recruiter_id == observer.account_id
                || target.account_id == observer.recruiter_id
            {
                info.status |= FRIEND_STATUS_RAF_LIKE_CPP;
            }
        }

        info.area = target.zone_id;
        info.level = u32::from(target.level);
        info.class = u32::from(target.class);
    }
    info
}

/// The per-lister gates of C++ `SocialMgr::BroadcastToFriendListers`
/// (`SocialMgr.cpp:263-288`): `player` is the subject being announced,
/// `lister` the connected session that lists them as a friend.
pub(crate) fn friend_lister_receives_broadcast_like_cpp(
    player: &PlayerPresenceSnapshotLikeCpp,
    player_security: u8,
    lister: &PlayerPresenceSnapshotLikeCpp,
    lister_security: u8,
) -> bool {
    // (RBAC_PERM_WHO_SEE_ALL_SEC_LEVELS represented as not granted.)
    if player_security > GM_LEVEL_IN_WHO_LIST_LIKE_CPP {
        return false;
    }
    // (RBAC_PERM_TWO_SIDE_WHO_LIST represented as not granted.)
    if player_team_for_race_cpp(lister.race) != player_team_for_race_cpp(player.race) {
        return false;
    }
    is_visible_globally_for_like_cpp(player, player_security, lister.guid, lister_security)
}

// ── inventory registrations ───────────────────────────────────────────────────

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::AddFriend,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_add_friend",
        handler: |session, _catalogs, mut pkt| {
            Box::pin(async move {
                match AddFriend::read(&mut pkt) {
                    Ok(add) => session.handle_add_friend(add).await,
                    Err(e) => tracing::warn!("Failed to read AddFriend: {e}"),
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::AddIgnore,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_add_ignore",
        handler: |session, _catalogs, mut pkt| {
            Box::pin(async move {
                match AddIgnore::read(&mut pkt) {
                    Ok(ignore) => session.handle_add_ignore(ignore).await,
                    Err(e) => tracing::warn!("Failed to read AddIgnore: {e}"),
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::DelFriend,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_del_friend",
        handler: |session, _catalogs, mut pkt| {
            Box::pin(async move {
                match DelFriend::read(&mut pkt) {
                    Ok(del) => session.handle_del_friend(del).await,
                    Err(e) => tracing::warn!("Failed to read DelFriend: {e}"),
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::DelIgnore,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_del_ignore",
        handler: |session, _catalogs, mut pkt| {
            Box::pin(async move {
                match DelIgnore::read(&mut pkt) {
                    Ok(ignore) => session.handle_del_ignore(ignore).await,
                    Err(e) => tracing::warn!("Failed to read DelIgnore: {e}"),
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::SendContactList,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_send_contact_list",
        handler: |session, _catalogs, mut pkt| {
            Box::pin(async move {
                match SendContactList::read(&mut pkt) {
                    Ok(list) => session.handle_send_contact_list(list).await,
                    Err(e) => tracing::warn!("Failed to read SendContactList: {e}"),
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::SetContactNotes,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_set_contact_notes",
        handler: |session, _catalogs, mut pkt| {
            Box::pin(async move {
                match SetContactNotes::read(&mut pkt) {
                    Ok(contact) => session.handle_set_contact_notes(contact).await,
                    Err(e) => tracing::warn!("Failed to read SetContactNotes: {e}"),
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::SocialContractRequest,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_social_contract_request",
        handler: |session, _catalogs, mut pkt| {
            Box::pin(async move {
                match wow_packet::packets::social::SocialContractRequest::read(&mut pkt) {
                    Ok(_) => session.handle_social_contract_request().await,
                    Err(e) => tracing::warn!("Failed to read SocialContractRequest: {e}"),
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::AcceptSocialContract,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_accept_social_contract",
        handler: |session, _catalogs, mut pkt| {
            Box::pin(async move {
                match AcceptSocialContract::read(&mut pkt) {
                    Ok(accept) => session.handle_accept_social_contract(accept).await,
                    Err(e) => tracing::warn!("Failed to read AcceptSocialContract: {e}"),
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::AccountNotificationAcknowledged,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_account_notification_acknowledged",
        handler: |session, _catalogs, mut pkt| {
            Box::pin(async move {
                match AccountNotificationAcknowledged::read(&mut pkt) {
                    Ok(packet) => session.handle_account_notification_acknowledged(packet).await,
                    Err(e) => tracing::warn!("Failed to read AccountNotificationAcknowledged: {e}"),
                }
            })
        },
    }
}

// ── friend status resolution (C++ SocialMgr) ──────────────────────────────────

impl WorldSession {
    fn social_observer_like_cpp(&self) -> Option<SocialObserverLikeCpp> {
        Some(SocialObserverLikeCpp {
            guid: self.player_guid()?,
            race: self.player_race_like_cpp(),
            security: self.security,
            account_id: self.account_id,
            recruiter_id: self.recruiter_id_like_cpp(),
        })
    }

    /// C++ `ObjectAccessor::FindPlayer(guid)` followed by the presence reads.
    fn presence_of_like_cpp(&self, guid: ObjectGuid) -> Option<PlayerPresenceSnapshotLikeCpp> {
        self.player_registry()
            .and_then(|registry| registry.presence_snapshot_like_cpp(guid))
    }

    /// C++ `SocialMgr::GetFriendInfo(this player, friend_guid, ...)`.
    pub(crate) fn resolve_friend_info_like_cpp(
        &self,
        friend_guid: ObjectGuid,
        note_when_online: &str,
    ) -> FriendInfoLikeCpp {
        let Some(observer) = self.social_observer_like_cpp() else {
            return FriendInfoLikeCpp::default();
        };
        let target = self.presence_of_like_cpp(friend_guid);
        friend_info_like_cpp(&observer, target.as_ref(), note_when_online)
    }

    /// C++ `WorldPackets::Social::FriendStatus::Initialize` (`SocialPackets.cpp:72-83`).
    fn friend_status_packet_like_cpp(
        &self,
        result: FriendsResult,
        guid: ObjectGuid,
        info: FriendInfoLikeCpp,
    ) -> FriendStatusPkt {
        FriendStatusPkt {
            result,
            guid,
            account_guid: ObjectGuid::EMPTY,
            virtual_realm_address: self.virtual_realm_address(),
            status: info.status,
            area_id: info.area as i32,
            level: info.level as i32,
            class_id: info.class,
            notes: info.note,
        }
    }

    /// C++ `SocialMgr::SendFriendStatus(player, result, friendGuid, broadcast = false)`:
    /// one `SMSG_FRIEND_STATUS` to this session.
    pub(crate) fn send_friend_status_like_cpp(
        &self,
        result: FriendsResult,
        friend_guid: ObjectGuid,
        note_when_online: &str,
    ) {
        let info = self.resolve_friend_info_like_cpp(friend_guid, note_when_online);
        let packet = self.friend_status_packet_like_cpp(result, friend_guid, info);
        self.send_packet(&packet);
    }

    /// C++ `SocialMgr::SendFriendStatus(player, result, player->GetGUID(), broadcast = true)`
    /// followed by `BroadcastToFriendListers` (`SocialMgr.cpp:249-288`).
    ///
    /// Login (`CharacterHandler.cpp:1224`, `FRIEND_ONLINE`) and logout
    /// (`WorldSession.cpp:651`, `FRIEND_OFFLINE`) call this while the player is
    /// still registered, so `GetFriendInfo(self, self)` sees itself online — the
    /// logout packet deliberately carries the live status bits, as in C++.
    pub(crate) async fn broadcast_friend_status_like_cpp(&self, result: FriendsResult) {
        let Some(my_guid) = self.player_guid() else {
            return;
        };
        let Some(port) = self.social_persistence_port_like_cpp() else {
            return;
        };
        let Some(registry) = self.player_registry().cloned() else {
            return;
        };

        // `FriendInfo fi; GetFriendInfo(player, friendGuid = player->GetGUID(), fi);`
        let info = self.resolve_friend_info_like_cpp(my_guid, "");
        let bytes = self
            .friend_status_packet_like_cpp(result, my_guid, info)
            .to_bytes();

        let listers = match port
            .listers_of_like_cpp(my_guid.counter() as u64, SOCIAL_FLAG_FRIEND_LIKE_CPP)
            .await
        {
            Ok(listers) => listers,
            Err(reason) => {
                warn!("BroadcastToFriendListers reverse lookup failed: {reason}");
                return;
            }
        };
        if listers.is_empty() {
            return;
        }
        // C++ `ASSERT(player)`: the announced player is the connected one.
        let Some(me) = registry.presence_snapshot_like_cpp(my_guid) else {
            return;
        };

        for lister in listers {
            let lister_guid = ObjectGuid::create_player(0, lister as i64);
            let Some(target) = registry.presence_snapshot_like_cpp(lister_guid) else {
                continue;
            };
            if !friend_lister_receives_broadcast_like_cpp(
                &me,
                self.security,
                &target,
                SEC_PLAYER_LIKE_CPP,
            ) {
                continue;
            }
            // Opcodes.cpp:1392 routes SMSG_FRIEND_STATUS to CONNECTION_TYPE_REALM.
            let _ = registry.send_current_realm_packet(target.registration, bytes.clone());
        }
    }

    /// C++ `Player::DeleteFromDB` (`Player.cpp:3953-3966`): every online character
    /// that listed the deleted one drops the entry and receives `FRIEND_REMOVED`
    /// through `SendFriendStatus(playerFriend, FRIEND_REMOVED, playerguid)` — a
    /// direct send computed from the lister's point of view, so the deleted
    /// (offline) target yields an all-zero `FriendInfo`.
    pub(crate) fn notify_listers_of_deleted_character_like_cpp(
        &self,
        deleted_guid: ObjectGuid,
        listers: &[u64],
    ) {
        let Some(registry) = self.player_registry() else {
            return;
        };
        let vra = self.virtual_realm_address();
        for lister in listers {
            let lister_guid = ObjectGuid::create_player(0, *lister as i64);
            let Some(target) = registry.presence_snapshot_like_cpp(lister_guid) else {
                continue;
            };
            // `GetFriendInfo(playerFriend, playerguid)`: FindPlayer(deleted) is null → zeros.
            let packet = FriendStatusPkt {
                result: FriendsResult::Removed,
                guid: deleted_guid,
                account_guid: ObjectGuid::EMPTY,
                virtual_realm_address: vra,
                status: FRIEND_STATUS_OFFLINE_LIKE_CPP,
                area_id: 0,
                level: 0,
                class_id: 0,
                notes: String::new(),
            };
            let _ = registry.send_current_realm_packet(target.registration, packet.to_bytes());
        }
    }
}

// ── handler implementations ───────────────────────────────────────────────────

impl WorldSession {
    /// C++ `PlayerSocial::SendSocialList(player, flags)` (`SocialMgr.cpp:140-171`).
    pub(crate) async fn send_contact_list_like_cpp(&mut self, flags: u32) {
        let my_guid = match self.player_guid() {
            Some(g) => g,
            None => return,
        };

        let port = match self.social_persistence_port_like_cpp() {
            Some(port) => port,
            None => {
                // C++ sends `ContactList` even when the loaded social map is
                // empty; it never follows it with a name-query response.
                self.send_packet_realm(&ContactListPkt {
                    flags,
                    contacts: Vec::new(),
                });
                return;
            }
        };

        // C++ `PlayerSocial::SendSocialList` iterates the loaded social map and
        // writes only entries matching the requested `SocialFlag` bitmask.
        let rows = match port.load_contacts_like_cpp(my_guid.counter(), flags).await {
            SocialContactListLoadOutcomeLikeCpp::Loaded(rows) => rows,
            SocialContactListLoadOutcomeLikeCpp::Failed { reason } => {
                warn!("SendContactList persistence error: {}", reason);
                Vec::new()
            }
        };

        let vra = self.virtual_realm_address();

        let mut contacts: Vec<ContactInfo> = Vec::new();
        let mut friends_count = 0_u32;
        let mut ignored_count = 0_u32;

        for row in rows {
            // Check client limit for friends list
            if row.type_flags & SOCIAL_FLAG_FRIEND_LIKE_CPP != 0 {
                friends_count += 1;
                if friends_count > SOCIALMGR_FRIEND_LIMIT_LIKE_CPP {
                    continue;
                }
            }
            // Check client limit for ignore list
            if row.type_flags & SOCIAL_FLAG_IGNORED_LIKE_CPP != 0 {
                ignored_count += 1;
                if ignored_count > SOCIALMGR_IGNORE_LIMIT_LIKE_CPP {
                    continue;
                }
            }

            let friend_guid = ObjectGuid::create_player(0, row.friend_guid);
            // `GetFriendInfo(player, guid, entry)` refreshes Status/Area/Level/Class
            // in place: offline contacts carry zeros, the stored Note always survives.
            let mut info = self.resolve_friend_info_like_cpp(friend_guid, "");
            info.note = row.note;

            contacts.push(ContactInfo {
                guid: friend_guid,
                wow_account_guid: ObjectGuid::EMPTY,
                virtual_realm_address: vra,
                native_realm_address: vra,
                type_flags: row.type_flags,
                note: info.note,
                status: info.status,
                area_id: info.area,
                level: info.level,
                class_id: info.class,
                is_mobile: false,
            });
        }

        self.send_packet_realm(&ContactListPkt { flags, contacts });
    }

    /// CMSG_ADD_FRIEND (0x36d8) — C++ `WorldSession::HandleAddFriendOpcode`.
    pub async fn handle_add_friend(&mut self, packet: AddFriend) {
        let Some(name) = normalize_player_name_like_cpp(&packet.name) else {
            return;
        };
        let notes = packet.notes;

        let my_guid = match self.player_guid() {
            Some(g) => g,
            None => return,
        };

        let port = match self.social_persistence_port_like_cpp() {
            Some(port) => port,
            None => return,
        };

        let candidate = match port
            .load_add_candidate_like_cpp(name.clone(), SocialRelationshipKindLikeCpp::Friend)
            .await
        {
            SocialAddCandidateLoadOutcomeLikeCpp::Found(candidate) => candidate,
            SocialAddCandidateLoadOutcomeLikeCpp::NotFound => {
                self.send_friend_status_like_cpp(FriendsResult::NotFound, ObjectGuid::EMPTY, "");
                return;
            }
            SocialAddCandidateLoadOutcomeLikeCpp::Failed { reason } => {
                warn!("AddFriend DB error looking up '{}': {}", name, reason);
                return;
            }
        };
        let friend_guid = ObjectGuid::create_player(0, candidate.guid);

        // Can't add yourself
        if friend_guid == my_guid {
            self.send_friend_status_like_cpp(FriendsResult::Self_, friend_guid, "");
            return;
        }

        // C++: WorldSession::HandleAddFriendOpcode rejects enemy-faction
        // contacts unless RBAC_PERM_TWO_SIDE_ADD_FRIEND is present. RustyCore
        // does not yet have AccountMgr/RBAC runtime, so normal-player behavior
        // is represented conservatively and the GM bypass remains a tracked gap.
        let player_team = player_team_for_race_cpp(self.player_race_like_cpp());
        let friend_team = player_team_for_race_cpp(candidate.race);
        if player_team != friend_team {
            self.send_friend_status_like_cpp(FriendsResult::Enemy, friend_guid, "");
            return;
        }

        let relationship = port
            .load_relationship_state_like_cpp(
                my_guid.counter(),
                candidate.guid,
                SocialRelationshipKindLikeCpp::Friend,
            )
            .await;
        if relationship.already_present {
            self.send_friend_status_like_cpp(FriendsResult::Already, friend_guid, "");
            return;
        }
        if relationship.relationship_count >= i64::from(SOCIALMGR_FRIEND_LIMIT_LIKE_CPP) {
            self.send_friend_status_like_cpp(FriendsResult::ListFull, friend_guid, "");
            return;
        }

        // AddToSocialList ORs the flag into an existing social row; preserve
        // ignore/mute bits instead of dropping this request with INSERT IGNORE.
        match port
            .add_relationship_like_cpp(
                my_guid.counter(),
                candidate.guid,
                SocialRelationshipKindLikeCpp::Friend,
                notes.clone(),
            )
            .await
        {
            PersistenceOutcomeLikeCpp::Applied { .. } => {}
            PersistenceOutcomeLikeCpp::Failed { reason }
            | PersistenceOutcomeLikeCpp::Unknown { reason } => {
                warn!("AddFriend insert error: {}", reason);
                return;
            }
        }

        // `Player* pFriend = FindPlayer(friendGuid); if (pFriend && pFriend->IsVisibleGloballyFor(GetPlayer()))`
        let is_online = self
            .presence_of_like_cpp(friend_guid)
            .is_some_and(|target| {
                is_visible_globally_for_like_cpp(
                    &target,
                    SEC_PLAYER_LIKE_CPP,
                    my_guid,
                    self.security,
                )
            });
        let result = if is_online {
            FriendsResult::AddedOnline
        } else {
            FriendsResult::AddedOffline
        };

        // `SetFriendNote` ran before `SendFriendStatus`, so the note is part of
        // the entry `GetFriendInfo` copies when the friend is online.
        self.send_friend_status_like_cpp(result, friend_guid, &notes);
        info!(
            "Player {:?} added friend {:?} ({})",
            my_guid, friend_guid, name
        );
    }

    /// Handle CMSG_ADD_IGNORE.
    ///
    /// C++ ref: `WorldSession::HandleAddIgnoreOpcode`.
    ///
    /// This represents the per-character ignore list (`SOCIAL_FLAG_IGNORED`).
    /// Account-level ignore remains parked until Rust owns `character_social.accountGuid`
    /// and an in-memory `PlayerSocial::_ignoredAccounts` equivalent.
    pub async fn handle_add_ignore(&mut self, ignore: AddIgnore) {
        let my_guid = match self.player_guid() {
            Some(g) => g,
            None => return,
        };

        let port = match self.social_persistence_port_like_cpp() {
            Some(port) => port,
            None => return,
        };

        let Some(name) = normalize_player_name_like_cpp(&ignore.name) else {
            return;
        };

        let candidate = match port
            .load_add_candidate_like_cpp(name.clone(), SocialRelationshipKindLikeCpp::Ignored)
            .await
        {
            SocialAddCandidateLoadOutcomeLikeCpp::Found(candidate) => candidate,
            SocialAddCandidateLoadOutcomeLikeCpp::NotFound => {
                self.send_friend_status_like_cpp(
                    FriendsResult::IgnoreNotFound,
                    ObjectGuid::EMPTY,
                    "",
                );
                return;
            }
            SocialAddCandidateLoadOutcomeLikeCpp::Failed { reason } => {
                warn!("AddIgnore DB error looking up '{}': {}", name, reason);
                return;
            }
        };
        let ignore_guid = ObjectGuid::create_player(0, candidate.guid);

        if ignore_guid == my_guid {
            self.send_friend_status_like_cpp(FriendsResult::IgnoreSelf, ignore_guid, "");
            return;
        }

        let relationship = port
            .load_relationship_state_like_cpp(
                my_guid.counter(),
                candidate.guid,
                SocialRelationshipKindLikeCpp::Ignored,
            )
            .await;
        if relationship.already_present {
            self.send_friend_status_like_cpp(FriendsResult::IgnoreAlready, ignore_guid, "");
            return;
        }
        if relationship.relationship_count >= i64::from(SOCIALMGR_IGNORE_LIMIT_LIKE_CPP) {
            self.send_friend_status_like_cpp(FriendsResult::IgnoreFull, ignore_guid, "");
            return;
        }
        match port
            .add_relationship_like_cpp(
                my_guid.counter(),
                candidate.guid,
                SocialRelationshipKindLikeCpp::Ignored,
                String::new(),
            )
            .await
        {
            PersistenceOutcomeLikeCpp::Applied { .. } => {}
            PersistenceOutcomeLikeCpp::Failed { reason }
            | PersistenceOutcomeLikeCpp::Unknown { reason } => {
                warn!("AddIgnore insert error: {}", reason);
                return;
            }
        }

        self.send_friend_status_like_cpp(FriendsResult::IgnoreAdded, ignore_guid, "");
        info!("Player {:?} ignored {:?} ({})", my_guid, ignore_guid, name);
    }

    /// CMSG_DEL_FRIEND (0x36d9) — C++ `WorldSession::HandleDelFriendOpcode`.
    ///
    /// `QualifiedGUID::VirtualRealmAddress` is read but unused, as in C++ (`@todo`).
    pub async fn handle_del_friend(&mut self, packet: DelFriend) {
        let friend_guid = packet.player_guid;

        let my_guid = match self.player_guid() {
            Some(g) => g,
            None => return,
        };

        let port = match self.social_persistence_port_like_cpp() {
            Some(port) => port,
            None => return,
        };
        match port
            .remove_relationship_like_cpp(
                my_guid.counter(),
                friend_guid.counter(),
                SocialRelationshipKindLikeCpp::Friend,
            )
            .await
        {
            PersistenceOutcomeLikeCpp::Applied { .. } => {}
            PersistenceOutcomeLikeCpp::Failed { reason }
            | PersistenceOutcomeLikeCpp::Unknown { reason } => {
                warn!("DelFriend persistence error: {}", reason);
                return;
            }
        }

        // The entry is gone, so `GetFriendInfo` finds no stored note.
        self.send_friend_status_like_cpp(FriendsResult::Removed, friend_guid, "");
    }

    /// Handle CMSG_DEL_IGNORE.
    ///
    /// C++ ref: `WorldSession::HandleDelIgnoreOpcode` delegates to
    /// `PlayerSocial::RemoveFromSocialList(..., SOCIAL_FLAG_IGNORED)`, which
    /// clears only the ignored bit and deletes the row only when no social flags
    /// remain.
    pub async fn handle_del_ignore(&mut self, ignore: DelIgnore) {
        let my_guid = match self.player_guid() {
            Some(g) => g,
            None => return,
        };

        let port = match self.social_persistence_port_like_cpp() {
            Some(port) => port,
            None => return,
        };

        let target_guid = ignore.player_guid;
        let target_counter = target_guid.counter();

        match port
            .remove_relationship_like_cpp(
                my_guid.counter(),
                target_counter,
                SocialRelationshipKindLikeCpp::Ignored,
            )
            .await
        {
            PersistenceOutcomeLikeCpp::Applied { .. } => {}
            PersistenceOutcomeLikeCpp::Failed { reason }
            | PersistenceOutcomeLikeCpp::Unknown { reason } => {
                warn!("DelIgnore persistence error: {}", reason);
                return;
            }
        }

        self.send_friend_status_like_cpp(FriendsResult::IgnoreRemoved, target_guid, "");
    }

    /// Handle CMSG_SET_CONTACT_NOTES.
    ///
    /// C++ ref: `WorldSession::HandleSetContactNotesOpcode` delegates to
    /// `PlayerSocial::SetFriendNote`, which silently returns if the contact is
    /// not present and truncates the stored note to 48 UTF-8 chars.
    pub async fn handle_set_contact_notes(&mut self, contact: SetContactNotes) {
        let my_guid = match self.player_guid() {
            Some(g) => g,
            None => return,
        };

        let port = match self.social_persistence_port_like_cpp() {
            Some(port) => port,
            None => return,
        };

        let note: String = contact.notes.chars().take(48).collect();
        match port
            .set_contact_note_like_cpp(my_guid.counter(), contact.player_guid.counter(), note)
            .await
        {
            PersistenceOutcomeLikeCpp::Applied { .. } => {}
            PersistenceOutcomeLikeCpp::Failed { reason }
            | PersistenceOutcomeLikeCpp::Unknown { reason } => {
                warn!("SetContactNotes update error: {}", reason);
            }
        }
    }

    /// Handle CMSG_SOCIAL_CONTRACT_REQUEST.
    ///
    /// C++ ref: `WorldSession::HandleSocialContractRequest` sends a
    /// `SocialContractRequestResponse` with `ShowSocialContract = false`.
    pub async fn handle_social_contract_request(&mut self) {
        self.send_packet(&SocialContractRequestResponse {
            show_social_contract: false,
        });
    }

    /// Handle CMSG_ACCEPT_SOCIAL_CONTRACT.
    ///
    /// C++ ref: `WorldSession::HandleAcceptSocialContract` currently logs the
    /// acceptance and leaves account-data persistence as a future hook.
    pub async fn handle_accept_social_contract(&mut self, _accept: AcceptSocialContract) {
        // Account-data persistence remains parked until Rust owns the account
        // data layer. Matching current C++ behavior here means no response.
    }

    /// Handle CMSG_ACCOUNT_NOTIFICATION_ACKNOWLEDGED.
    ///
    /// C++ ref: `WorldSession::HandleAccountNotificationAcknowledged` logs the
    /// notification id and leaves DB read-state persistence as a future hook.
    pub async fn handle_account_notification_acknowledged(
        &mut self,
        _packet: AccountNotificationAcknowledged,
    ) {
        // Matching current C++ behavior here means no response and no state
        // mutation; account-notification persistence is not implemented there.
    }

    /// CMSG_SEND_CONTACT_LIST (0x36d7) — C++ `WorldSession::HandleContactListOpcode`.
    pub async fn handle_send_contact_list(&mut self, packet: SendContactList) {
        self.send_contact_list_like_cpp(packet.flags).await;
    }
}

#[cfg(test)]
#[path = "social/tests/mod.rs"]
mod tests;
