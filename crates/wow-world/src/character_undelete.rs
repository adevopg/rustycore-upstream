//! Character deletion method and character undelete (TrinityCore 3.4.3,
//! tag TDB343.24081).
//!
//! C++ anchors:
//! - `Player::DeleteFromDB` method selection (`Player.cpp:3790-3822`) and the
//!   `CHAR_DELETE_UNLINK` arm (`Player.cpp:4193-4200`);
//! - `WorldSession::HandleGetUndeleteCooldownStatus` + callback and
//!   `HandleCharUndeleteOpcode` (`CharacterHandler.cpp:2612-2735`);
//! - config `CharDelete.Method/MinLevel/DeathKnight.MinLevel/DemonHunter.MinLevel`
//!   and `FeatureSystem.CharacterUndelete.{Enabled,Cooldown}` (`World.cpp:1450-1453,
//!   1598-1599`).
//!
//! Session owns packet admission and publication; this module owns the decisions
//! and the ordered persistence steps, over the narrow persistence port.

use std::sync::Arc;

use wow_packet::packets::character::undelete_result;
use wow_packet::packets::misc::UndeleteCooldownStatusResponse;
use wow_persistence::{
    CharacterAdministrationLoadOutcomeLikeCpp as LoadOutcome,
    CharacterAdministrationMutationOutcomeLikeCpp as MutationOutcome,
    CharacterDeleteCandidateLikeCpp, CharacterUndeletePersistencePortLikeCpp,
};

/// C++ `CharDeleteMethod::CHAR_DELETE_REMOVE`.
pub const CHAR_DELETE_REMOVE_LIKE_CPP: u32 = 0;
/// C++ `CharDeleteMethod::CHAR_DELETE_UNLINK`.
pub const CHAR_DELETE_UNLINK_LIKE_CPP: u32 = 1;

const CLASS_DEATH_KNIGHT_LIKE_CPP: u8 = 6;
const CLASS_DEMON_HUNTER_LIKE_CPP: u8 = 12;

/// `World*Configs` values read by the delete/undelete paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterDeletionConfigLikeCpp {
    /// `CharDelete.Method` (`CONFIG_CHARDELETE_METHOD`), default 0.
    pub delete_method: u32,
    /// `CharDelete.MinLevel`, default 0.
    pub min_level: u32,
    /// `CharDelete.DeathKnight.MinLevel`, default 0.
    pub death_knight_min_level: u32,
    /// `CharDelete.DemonHunter.MinLevel`, default 0.
    pub demon_hunter_min_level: u32,
    /// `FeatureSystem.CharacterUndelete.Cooldown` in seconds, default 2592000.
    pub undelete_cooldown_secs: u32,
}

impl Default for CharacterDeletionConfigLikeCpp {
    fn default() -> Self {
        Self {
            delete_method: CHAR_DELETE_REMOVE_LIKE_CPP,
            min_level: 0,
            death_knight_min_level: 0,
            demon_hunter_min_level: 0,
            undelete_cooldown_secs: 2_592_000,
        }
    }
}

/// Process-owned deletion policy plus its persistence port, borrowed by handlers
/// from `SessionHandlerCatalogsLikeCpp` (the `WorldSession` field set is frozen).
pub struct CharacterDeletionServiceLikeCpp {
    config: CharacterDeletionConfigLikeCpp,
    port: Option<Arc<dyn CharacterUndeletePersistencePortLikeCpp>>,
}

impl CharacterDeletionServiceLikeCpp {
    pub fn new(
        config: CharacterDeletionConfigLikeCpp,
        port: Arc<dyn CharacterUndeletePersistencePortLikeCpp>,
    ) -> Self {
        Self {
            config,
            port: Some(port),
        }
    }

    /// TC defaults and no database: deletes use the RustyCore remove path.
    pub fn disabled() -> Self {
        Self {
            config: CharacterDeletionConfigLikeCpp::default(),
            port: None,
        }
    }

    pub fn config(&self) -> CharacterDeletionConfigLikeCpp {
        self.config
    }

    pub fn port(&self) -> Option<&Arc<dyn CharacterUndeletePersistencePortLikeCpp>> {
        self.port.as_ref()
    }
}

impl Default for CharacterDeletionServiceLikeCpp {
    fn default() -> Self {
        Self::disabled()
    }
}

/// `Player::DeleteFromDB` method selection (`deleteFinally = false`): the
/// configured method, forced to `CHAR_DELETE_REMOVE` when the cached character is
/// below the class-specific minimum level. An uncached character keeps the
/// configured method, like C++.
pub fn delete_method_like_cpp(
    config: &CharacterDeletionConfigLikeCpp,
    candidate: Option<CharacterDeleteCandidateLikeCpp>,
) -> u32 {
    let Some(candidate) = candidate else {
        return config.delete_method;
    };
    let min_level = match candidate.class {
        CLASS_DEATH_KNIGHT_LIKE_CPP => config.death_knight_min_level,
        CLASS_DEMON_HUNTER_LIKE_CPP => config.demon_hunter_min_level,
        _ => config.min_level,
    };
    if u32::from(candidate.level) < min_level {
        CHAR_DELETE_REMOVE_LIKE_CPP
    } else {
        config.delete_method
    }
}

/// `HandleUndeleteCooldownStatusCallback`: seconds left on the cooldown.
pub fn undelete_cooldown_remaining_like_cpp(
    last_undelete: u32,
    max_cooldown: u32,
    now: u32,
) -> u32 {
    let end = u64::from(last_undelete) + u64::from(max_cooldown);
    if end > u64::from(now) {
        (end - u64::from(now)) as u32
    } else {
        0
    }
}

/// `HandleCharUndeleteOpcode` first callback: `lastUndelete && lastUndelete +
/// maxCooldown > now`.
pub fn undelete_on_cooldown_like_cpp(last_undelete: u32, max_cooldown: u32, now: u32) -> bool {
    last_undelete != 0 && u64::from(last_undelete) + u64::from(max_cooldown) > u64::from(now)
}

/// `HandleGetUndeleteCooldownStatus` + callback. A missing row or a failed query
/// both mean "no result" (cooldown 0), as the C++ callback cannot tell them apart.
pub async fn undelete_cooldown_status_like_cpp(
    port: Option<&dyn CharacterUndeletePersistencePortLikeCpp>,
    battlenet_account_id: u32,
    max_cooldown: u32,
    now: u32,
) -> UndeleteCooldownStatusResponse {
    let last_undelete = match port {
        Some(port) => match port
            .load_last_character_undelete_like_cpp(battlenet_account_id)
            .await
        {
            LoadOutcome::Loaded(last) => Some(last),
            LoadOutcome::NotFound => None,
            LoadOutcome::Failed { reason } => {
                tracing::warn!(battlenet_account_id, %reason, "LastCharacterUndelete query failed");
                None
            }
        },
        None => None,
    };
    let current = last_undelete
        .map(|last| undelete_cooldown_remaining_like_cpp(last, max_cooldown, now))
        .unwrap_or(0);
    UndeleteCooldownStatusResponse {
        on_cooldown: current > 0,
        max_cooldown,
        current_cooldown: current,
    }
}

/// Inputs of one `CMSG_UNDELETE_CHARACTER`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndeleteRequestLikeCpp {
    pub guid_low: u64,
    pub account_id: u32,
    pub battlenet_account_id: u32,
    /// `CONFIG_FEATURE_SYSTEM_CHARACTER_UNDELETE_ENABLED`.
    pub enabled: bool,
    /// `CONFIG_FEATURE_SYSTEM_CHARACTER_UNDELETE_COOLDOWN`.
    pub max_cooldown: u32,
    /// `CONFIG_CHARACTERS_PER_REALM`.
    pub characters_per_realm: u32,
    pub now: u32,
}

/// `HandleCharUndeleteOpcode` chain; returns the `CharacterUndeleteResult`.
///
/// RustyCore departures (no C++ equivalent can be observed): a missing port or a
/// failed query/commit answers `ERROR_UNKNOWN` instead of hanging the client;
/// the restore is only reported `OK` after its character row actually changed.
pub async fn undelete_character_like_cpp(
    port: Option<&dyn CharacterUndeletePersistencePortLikeCpp>,
    request: UndeleteRequestLikeCpp,
) -> u32 {
    if !request.enabled {
        return undelete_result::ERROR_DISABLED;
    }
    let Some(port) = port else {
        return undelete_result::ERROR_UNKNOWN;
    };

    match port
        .load_last_character_undelete_like_cpp(request.battlenet_account_id)
        .await
    {
        LoadOutcome::Loaded(last)
            if undelete_on_cooldown_like_cpp(last, request.max_cooldown, request.now) =>
        {
            return undelete_result::ERROR_COOLDOWN;
        }
        LoadOutcome::Loaded(_) | LoadOutcome::NotFound => {}
        LoadOutcome::Failed { .. } => return undelete_result::ERROR_UNKNOWN,
    }

    let info = match port.load_deleted_character_like_cpp(request.guid_low).await {
        LoadOutcome::Loaded(info) => info,
        LoadOutcome::NotFound => return undelete_result::ERROR_CHAR_CREATE,
        LoadOutcome::Failed { .. } => return undelete_result::ERROR_UNKNOWN,
    };
    if info.account_id != request.account_id {
        return undelete_result::ERROR_UNKNOWN;
    }

    match port.find_character_name_like_cpp(info.name.clone()).await {
        LoadOutcome::Loaded(()) => return undelete_result::ERROR_NAME_TAKEN_BY_THIS_ACCOUNT,
        LoadOutcome::NotFound => {}
        LoadOutcome::Failed { .. } => return undelete_result::ERROR_UNKNOWN,
    }

    match port
        .load_account_character_count_like_cpp(request.account_id)
        .await
    {
        LoadOutcome::Loaded(count) if count >= u64::from(request.characters_per_realm) => {
            return undelete_result::ERROR_CHAR_CREATE;
        }
        LoadOutcome::Loaded(_) | LoadOutcome::NotFound => {}
        LoadOutcome::Failed { .. } => return undelete_result::ERROR_UNKNOWN,
    }

    match port
        .restore_deleted_character_like_cpp(
            request.guid_low,
            info.name,
            request.account_id,
            request.battlenet_account_id,
        )
        .await
    {
        MutationOutcome::Applied => undelete_result::OK,
        MutationOutcome::Failed { reason } => {
            tracing::warn!(guid = request.guid_low, %reason, "character undelete failed");
            undelete_result::ERROR_UNKNOWN
        }
    }
}

/// Current unix time for the C++ `GameTime::GetGameTime()` comparisons.
pub(crate) fn unix_now_like_cpp() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs().min(u64::from(u32::MAX)) as u32)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "character_undelete/tests.rs"]
mod tests;
