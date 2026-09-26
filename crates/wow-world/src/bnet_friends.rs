// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Battle.net (BattleTag) friends for the 3.4.3 client, served by the worldserver.
//!
//! TrinityCore 3.4.3 registers `friends.v1.FriendsService`,
//! `presence.v1.PresenceService` and `user_manager.v1.UserManagerService` as
//! `ERROR_RPC_NOT_IMPLEMENTED` stubs (`WorldserverServiceDispatcher.cpp`), so the
//! client never shows a BattleTag friends list. This module replicates the
//! LegionCore 7.3.5 design (`Battlenet::FriendsMgr`, PDB symbols `LoadFromDB`,
//! `OnSessionOpened/Closed`, `OnPlayerLogin/Logout/LevelChanged/ZoneChanged/
//! StatusChanged`, `SendInvitation/AcceptInvitation/DeclineInvitation/
//! RevokeInvitation/RemoveFriend/ViewFriends/UpdateFriendState/UpdatePresence`,
//! `BuildAccountPresence`, `BuildGameAccountPresence`, `FillStatus`,
//! `FillZoneAndLevel`, `SendPresenceToFriends`, `SendFullPresenceOf`):
//!
//! - one in-memory manager per process, loaded at startup from the auth tables
//!   `battlenet_account_friends` / `battlenet_account_friend_invitations`
//!   (`wow_persistence::BnetFriendsPersistencePortLikeCpp`);
//! - `CMSG_BATTLENET_REQUEST` calls of the three services answered through
//!   `SMSG_BATTLENET_RESPONSE` (`bnet_services.rs` dispatch);
//! - `FriendsListener` / `PresenceListener` client notifications pushed as
//!   `SMSG_BATTLENET_NOTIFICATION` to the registered sessions of every online
//!   friend, from any thread, through the sessions' packet channels.
//!
//! Message layouts: the `.proto` files under `crates/wow-proto/proto/bgs/low/pb/client/`
//! reconstructed from the TrinityCore 3.4.3 generated headers. Presence field
//! ids: [`presence_fields`] (confirmed vs. assumed is stated per constant).
//!
//! Wiring: the process installs the manager with [`install_global_like_cpp`]
//! after [`BnetFriendsMgr::load_from_db_like_cpp`]; session lifecycle hooks call
//! the `on_*_like_cpp` entry points with the `WorldSession`.

mod dispatch;
mod manager;
mod presence;
pub mod presence_fields;
mod session_port;
#[cfg(test)]
mod tests;

use std::sync::{Arc, OnceLock};

use wow_proto::bgs::protocol::EntityId;

pub use manager::{BnetFriendsLimitsLikeCpp, BnetFriendsMgr};
pub use session_port::{
    BnetAgentLikeCpp, BnetFriendsSessionLikeCpp, BnetGameAccountPresenceSnapshotLikeCpp,
};

pub(crate) use dispatch::{
    call_friends_service_method_like_cpp, call_presence_service_method_like_cpp,
    call_user_manager_service_method_like_cpp,
};

/// The two entity kinds the worldserver addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BnetEntityKindLikeCpp {
    /// `battlenet_accounts.id`.
    Account(u32),
    /// `account.id` (game account).
    GameAccount(u32),
}

/// `EntityId` of a Battle.net account (`LogonResult.account_id` convention).
pub fn account_entity_id_like_cpp(account_id: u32) -> EntityId {
    EntityId {
        high: presence_fields::ACCOUNT_ENTITY_HIGH_LIKE_CPP,
        low: u64::from(account_id),
    }
}

/// `EntityId` of a WoW game account (`LogonResult.game_account_id` convention).
pub fn game_account_entity_id_like_cpp(game_account_id: u32) -> EntityId {
    EntityId {
        high: presence_fields::GAME_ACCOUNT_ENTITY_HIGH_LIKE_CPP,
        low: u64::from(game_account_id),
    }
}

pub fn entity_kind_like_cpp(entity: &EntityId) -> Option<BnetEntityKindLikeCpp> {
    let low = u32::try_from(entity.low).ok()?;
    match entity.high {
        presence_fields::ACCOUNT_ENTITY_HIGH_LIKE_CPP => Some(BnetEntityKindLikeCpp::Account(low)),
        presence_fields::GAME_ACCOUNT_ENTITY_HIGH_LIKE_CPP => {
            Some(BnetEntityKindLikeCpp::GameAccount(low))
        }
        _ => None,
    }
}

static GLOBAL_LIKE_CPP: OnceLock<Arc<BnetFriendsMgr>> = OnceLock::new();

/// Install the process-wide manager (C++ singleton style); once per process.
pub fn install_global_like_cpp(mgr: Arc<BnetFriendsMgr>) -> Result<(), Arc<BnetFriendsMgr>> {
    GLOBAL_LIKE_CPP.set(mgr)
}

/// The installed manager, `None` until bootstrap installed one (the three
/// services then keep answering `ERROR_RPC_NOT_IMPLEMENTED` like C++).
pub fn global_like_cpp() -> Option<&'static Arc<BnetFriendsMgr>> {
    GLOBAL_LIKE_CPP.get()
}
