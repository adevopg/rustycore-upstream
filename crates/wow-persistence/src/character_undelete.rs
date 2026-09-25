//! SQLx-free contract for character deletion (unlink method) and undelete.
//!
//! Persistence half of C++ `Player::DeleteFromDB` (`CHAR_DELETE_UNLINK`,
//! `Player.cpp:4193-4200` at TDB343.24081) and of
//! `WorldSession::HandleGetUndeleteCooldownStatus` / `HandleCharUndeleteOpcode`
//! (`CharacterHandler.cpp:2612-2735`). Gameplay decisions (delete method, cooldown,
//! result codes) stay in `wow-world`; the adapter owns the Login/Character
//! statements and the character identity cache update.

use crate::{
    CharacterAdministrationLoadOutcomeLikeCpp as LoadOutcome,
    CharacterAdministrationMutationOutcomeLikeCpp as MutationOutcome, PersistenceFutureLikeCpp,
};

/// `CharacterCacheEntry` fields read by `Player::DeleteFromDB` to pick the method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterDeleteCandidateLikeCpp {
    pub class: u8,
    pub level: u8,
}

/// `CHAR_SEL_CHAR_DEL_INFO_BY_GUID` columns 1 and 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedCharacterInfoLikeCpp {
    pub name: String,
    pub account_id: u32,
}

pub trait CharacterUndeletePersistencePortLikeCpp: Send + Sync {
    /// `sCharacterCache->GetCharacterCacheByGuid`; `NotFound` when uncached.
    fn load_delete_candidate_like_cpp(
        &self,
        guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<CharacterDeleteCandidateLikeCpp>>;

    /// `CHAR_UPD_DELETE_INFO` for a character owned by `account_id`, then
    /// `sCharacterCache->UpdateCharacterInfoDeleted(guid, true, "")`.
    fn unlink_owned_character_like_cpp(
        &self,
        guid: u64,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome>;

    /// `LOGIN_SEL_LAST_CHAR_UNDELETE`; `NotFound` when the account row is missing.
    fn load_last_character_undelete_like_cpp(
        &self,
        battlenet_account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<u32>>;

    /// `CHAR_SEL_CHAR_DEL_INFO_BY_GUID`.
    fn load_deleted_character_like_cpp(
        &self,
        guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<DeletedCharacterInfoLikeCpp>>;

    /// `CHAR_SEL_CHECK_NAME`: `Loaded(())` when a character uses the name.
    fn find_character_name_like_cpp(
        &self,
        name: String,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<()>>;

    /// `CHAR_SEL_SUM_CHARS`.
    fn load_account_character_count_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<u64>>;

    /// `CHAR_UPD_RESTORE_DELETE_INFO`, `LOGIN_UPD_LAST_CHAR_UNDELETE` and
    /// `sCharacterCache->UpdateCharacterInfoDeleted(guid, false, name)`.
    fn restore_deleted_character_like_cpp(
        &self,
        guid: u64,
        name: String,
        account_id: u32,
        battlenet_account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome>;

    /// RustyCore shop service (LegionCore `WebsiteType::DeletedCharacter`):
    /// `UPDATE battlenet_accounts SET LastCharacterUndelete = 0 WHERE Id = ?`,
    /// i.e. the next undelete is not on cooldown.
    fn reset_undelete_cooldown_like_cpp(
        &self,
        battlenet_account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome>;
}
