//! SQLx-free contracts for the BattlePay character services, character boosts and
//! character transfers.
//!
//! Port of the LegionCore 7.3.5 distribution statements (`LOGIN_*_BPAY_DISTRIBUTION*`,
//! `LoginDatabase.cpp:222-229`), `CHAR_UPD_ADD_AT_LOGIN_FLAG` /
//! `CHAR_UPD_CHARACTER_BOOST_QUEUED` (`CharacterDatabase.cpp:524,549`), the transfer
//! lookups `LOGIN_SEL_BNET_VAS_TRANSFER_TARGET_*` (`LoginDatabase.cpp:148-149`) and
//! `CompleteVasCharacterTransfer`'s same-realm account move
//! (`BattlePayHandler.cpp:98`). Every character-side write commits in one Character
//! DB transaction with the `character_battlepay_delivery` receipt of the paid order,
//! and every account-side write that consumes a paid order commits in one Login DB
//! transaction with the `battlepay_purchase` `Paid -> Delivered` transition, so a
//! paid order is serviced exactly once (see `docs/migration/battlepay-343-protocol.md`,
//! section 9).

use crate::{
    BattlePayDeliveryReceiptLikeCpp, PersistenceFutureLikeCpp, PersistenceOutcomeLikeCpp,
    PlayerInventoryPersistenceRequestLikeCpp,
};

/// `auth.battlepay_distribution.status` (LegionCore `Battlepay::DistributionStatus`).
pub const BATTLE_PAY_DISTRIBUTION_STATUS_AVAILABLE_LIKE_CPP: u8 = 1;
pub const BATTLE_PAY_DISTRIBUTION_STATUS_ASSIGNED_LIKE_CPP: u8 = 2;
pub const BATTLE_PAY_DISTRIBUTION_STATUS_FINISHED_LIKE_CPP: u8 = 4;

/// One `auth.battlepay_distribution` row (LegionCore `LOGIN_SEL_BPAY_DISTRIBUTIONS`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDistributionRowLikeCpp {
    pub id: u64,
    pub product_id: u32,
    pub status: u8,
    pub revoked: bool,
    pub realm_id: u32,
    pub character_guid: u64,
    pub specialization_id: u16,
}

/// A paid order turned into a distribution (LegionCore `AddDistribution` from
/// `ProcessDelivery`): the distribution row is created from the order row and the
/// order moves `Paid -> Delivered` in the same Login DB transaction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDistributionGrantLikeCpp {
    pub distribution_id: u64,
    pub external_id: String,
    pub web_order_id: String,
}

/// LegionCore `LOGIN_UPD_BPAY_DISTRIBUTION_ASSIGNED` arguments.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDistributionAssignLikeCpp {
    pub distribution_id: u64,
    pub account_id: u32,
    pub realm_id: u32,
    pub character_guid: u64,
    pub specialization_id: u16,
    pub choice_id: u16,
}

/// The game accounts of a Battle.net account found by e-mail
/// (LegionCore `LOGIN_SEL_BNET_VAS_TRANSFER_TARGET_BY_EMAIL`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayBnetGameAccountsLikeCpp {
    pub battlenet_account_id: u32,
    /// `(account.id, account.username)`, in `battlenet_index` order.
    pub game_accounts: Vec<(u32, String)>,
}

/// Login database capability of the character services.
pub trait BattlePayDistributionPersistencePortLikeCpp: Send + Sync {
    /// LegionCore `LOGIN_SEL_BPAY_DISTRIBUTIONS` (`status < 4 OR revoked = 1`).
    fn load_distributions_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<BattlePayDistributionRowLikeCpp>, String>>;

    /// Distribution insert + order delivered, one transaction. `Failed` when the
    /// order is not `Paid` any more (nothing written).
    fn grant_distribution_like_cpp(
        &self,
        grant: BattlePayDistributionGrantLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// LegionCore `LOGIN_UPD_BPAY_DISTRIBUTION_ASSIGNED` (`status 1 -> 2`);
    /// `Applied { rows: 0 }` means the distribution was not available.
    fn assign_distribution_like_cpp(
        &self,
        assign: BattlePayDistributionAssignLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// Compensation of [`Self::assign_distribution_like_cpp`] when the character
    /// side could not be queued (`status 2 -> 1`).
    fn unassign_distribution_like_cpp(
        &self,
        distribution_id: u64,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// LegionCore `LOGIN_SEL_BPAY_DISTRIBUTION_PENDING_BY_CHAR`, scoped to the realm.
    fn load_pending_distribution_like_cpp(
        &self,
        character_guid: u64,
        realm_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayDistributionRowLikeCpp>, String>>;

    /// LegionCore `LOGIN_UPD_BPAY_DISTRIBUTION_FINISHED` (`status 2 -> 4`).
    fn finish_distribution_like_cpp(
        &self,
        distribution_id: u64,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// "Restore deleted character" service: clear the Battle.net account's
    /// character-undelete cooldown (TC `LOGIN_UPD_LAST_CHAR_UNDELETE` column) and
    /// mark the order delivered, one transaction.
    fn grant_undelete_like_cpp(
        &self,
        battlenet_account_id: u32,
        external_id: String,
        web_order_id: String,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// LegionCore `LOGIN_SEL_BNET_VAS_TRANSFER_TARGET_ACCOUNT`.
    fn load_account_battlenet_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<u32>, String>>;

    /// LegionCore `LOGIN_SEL_BNET_VAS_TRANSFER_TARGET_BY_EMAIL`.
    fn load_bnet_game_accounts_like_cpp(
        &self,
        email: String,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayBnetGameAccountsLikeCpp>, String>>;
}

/// A character of the account as the services read it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayCharacterRowLikeCpp {
    pub guid: u64,
    pub account_id: u32,
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub level: u8,
    pub at_login_flags: u16,
    pub online: bool,
    pub logout_time: u64,
    pub guild_id: u64,
    /// The guild's leader guid (0 without a guild).
    pub guild_leader_guid: u64,
}

/// Same-realm account move of `CompleteVasCharacterTransfer`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayCharacterTransferLikeCpp {
    pub character_guid: u64,
    pub from_account_id: u32,
    pub to_account_id: u32,
    /// `AT_LOGIN_CHANGE_FACTION` for the faction-transfer bundle, else 0.
    pub add_at_login_flags: u16,
}

/// Loadout items, money and receipt of a boost applied at login.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayBoostCompletionLikeCpp {
    pub character_guid: u64,
    pub remove_at_login_flags: u16,
    pub money: u64,
}

/// Character database capability of the character services.
pub trait BattlePayCharacterServicePersistencePortLikeCpp: Send + Sync {
    /// Characters of the account that are not deleted.
    fn load_account_characters_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<BattlePayCharacterRowLikeCpp>, String>>;

    fn load_character_like_cpp(
        &self,
        character_guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayCharacterRowLikeCpp>, String>>;

    /// Receipt + `at_login |= flags` of the owned character, one transaction
    /// (LegionCore `CharacterService::{SetRename,ChangeFaction,ChangeRace,Customize}`).
    fn persist_service_delivery_like_cpp(
        &self,
        receipt: BattlePayDeliveryReceiptLikeCpp,
        at_login_flags: u16,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// Receipt + guild membership removal + account move, one transaction.
    fn persist_transfer_delivery_like_cpp(
        &self,
        receipt: BattlePayDeliveryReceiptLikeCpp,
        transfer: BattlePayCharacterTransferLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// LegionCore `CHAR_UPD_CHARACTER_BOOST_QUEUED`: level, xp 0 and the boost
    /// at-login flag of an offline, owned character below the level.
    fn queue_character_boost_like_cpp(
        &self,
        character_guid: u64,
        account_id: u32,
        level: u8,
        at_login_flags: u16,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// Loadout item rows, money and flag removal of a boost applied at login, with
    /// the boost receipt (keyed by the distribution), one transaction.
    fn persist_boost_completion_like_cpp(
        &self,
        receipt: BattlePayDeliveryReceiptLikeCpp,
        completion: BattlePayBoostCompletionLikeCpp,
        inventory: Vec<PlayerInventoryPersistenceRequestLikeCpp>,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;
}
