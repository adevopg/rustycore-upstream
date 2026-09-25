//! SQLx-free persistence contract for character-list administration.
//!
//! This is the persistence half of C++ `CharacterHandler.cpp`: protocol and
//! gameplay validation remain in `wow-world`, while the MariaDB adapter owns
//! prepared statements, row decoding and the rename/customize transactions.

use crate::PersistenceFutureLikeCpp;
use crate::{
    CharacterRaceOrFactionChangeCandidateLikeCpp, CharacterRaceOrFactionChangeCommitLikeCpp,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterCustomizationPersistenceLikeCpp {
    pub option_id: i32,
    pub choice_id: i32,
}

/// One initial item created by C++ `Player::Create` and written by
/// `Player::SaveToDB` -> `_SaveInventory` / `Item::SaveToDB` in the
/// `CharacterHandler::HandleCharCreateOpcode` transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterCreateItemPersistenceLikeCpp {
    pub item_guid: u64,
    pub item_id: u32,
    pub count: u32,
    pub durability: u32,
    /// `ItemData::DynamicFlags` (new-item and binding flags).
    pub dynamic_flags: u32,
    /// C++ `PlayerInfo::itemContext`.
    pub item_context: u8,
    /// Containing bag item GUID, or 0 for `INVENTORY_SLOT_BAG_0`.
    pub bag_guid: u64,
    pub slot: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CharacterCreatePersistenceRequestLikeCpp {
    pub guid: u64,
    pub account_id: u32,
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub sex: u8,
    pub rest_state: u8,
    pub map_id: i32,
    pub position: [f32; 4],
    pub create_time: i64,
    pub health: u32,
    pub power1: u32,
    pub last_login_build: u32,
    pub customizations: Vec<CharacterCustomizationPersistenceLikeCpp>,
    /// C++ `Player::SaveToDB` "cache equipment" string.
    pub equipment_cache: String,
    /// Initial items in C++ storage order; containers precede their contents.
    pub items: Vec<CharacterCreateItemPersistenceLikeCpp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterRenameCandidateLikeCpp {
    pub old_name: String,
    pub at_login_flags: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterCustomizeCandidateLikeCpp {
    pub old_name: String,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub at_login_flags: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CharacterAdministrationLoadOutcomeLikeCpp<T> {
    Loaded(T),
    NotFound,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CharacterAdministrationMutationOutcomeLikeCpp {
    Applied,
    Failed { reason: String },
}

/// One cohesive capability for character-list create/delete/rename/customize.
/// It deliberately exposes semantic operations rather than statements or a
/// generic transaction recorder.
pub trait CharacterAdministrationPersistencePortLikeCpp: Send + Sync {
    fn find_character_name_like_cpp(
        &self,
        name: &str,
    ) -> PersistenceFutureLikeCpp<'_, CharacterAdministrationLoadOutcomeLikeCpp<()>>;

    fn load_account_character_count_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, CharacterAdministrationLoadOutcomeLikeCpp<u64>>;

    fn create_character_like_cpp(
        &self,
        request: CharacterCreatePersistenceRequestLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, CharacterAdministrationMutationOutcomeLikeCpp>;

    fn delete_owned_character_like_cpp(
        &self,
        guid: u64,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, CharacterAdministrationMutationOutcomeLikeCpp>;

    fn load_rename_candidate_like_cpp(
        &self,
        guid: u64,
        new_name: &str,
    ) -> PersistenceFutureLikeCpp<
        '_,
        CharacterAdministrationLoadOutcomeLikeCpp<CharacterRenameCandidateLikeCpp>,
    >;

    fn commit_rename_like_cpp(
        &self,
        guid: u64,
        new_name: &str,
        at_login_flags: u16,
    ) -> PersistenceFutureLikeCpp<'_, CharacterAdministrationMutationOutcomeLikeCpp>;

    fn load_customize_candidate_like_cpp(
        &self,
        guid: u64,
    ) -> PersistenceFutureLikeCpp<
        '_,
        CharacterAdministrationLoadOutcomeLikeCpp<CharacterCustomizeCandidateLikeCpp>,
    >;

    fn commit_customize_like_cpp(
        &self,
        guid: u64,
        name: &str,
        at_login_flags: u16,
        customizations: Vec<CharacterCustomizationPersistenceLikeCpp>,
    ) -> PersistenceFutureLikeCpp<'_, CharacterAdministrationMutationOutcomeLikeCpp>;

    /// C++ `CHAR_SEL_CHAR_RACE_OR_FACTION_CHANGE_INFOS` plus the `CharacterCache`
    /// entry read by `HandleCharRaceOrFactionChangeCallback`. Adapters without the
    /// capability answer `Failed` (the handler then sends `CHAR_CREATE_ERROR`).
    fn load_race_or_faction_change_candidate_like_cpp(
        &self,
        guid: u64,
    ) -> PersistenceFutureLikeCpp<
        '_,
        CharacterAdministrationLoadOutcomeLikeCpp<CharacterRaceOrFactionChangeCandidateLikeCpp>,
    > {
        let _ = guid;
        Box::pin(async {
            CharacterAdministrationLoadOutcomeLikeCpp::Failed {
                reason: "race/faction change is not supported by this adapter".to_owned(),
            }
        })
    }

    /// C++ `CHAR_SEL_CHAR_REP_BY_FACTION` (synchronous in C++, read before the
    /// transaction is built). `NotFound` when the character has no row.
    fn load_reputation_standing_like_cpp(
        &self,
        guid: u64,
        faction_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, CharacterAdministrationLoadOutcomeLikeCpp<i32>> {
        let _ = (guid, faction_id);
        Box::pin(async {
            CharacterAdministrationLoadOutcomeLikeCpp::Failed {
                reason: "reputation reads are not supported by this adapter".to_owned(),
            }
        })
    }

    /// The single race/faction change transaction of C++.
    fn commit_race_or_faction_change_like_cpp(
        &self,
        request: CharacterRaceOrFactionChangeCommitLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, CharacterAdministrationMutationOutcomeLikeCpp> {
        let _ = request;
        Box::pin(async {
            CharacterAdministrationMutationOutcomeLikeCpp::Failed {
                reason: "race/faction change is not supported by this adapter".to_owned(),
            }
        })
    }
}
