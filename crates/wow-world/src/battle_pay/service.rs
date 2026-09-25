//! Process-owned BattlePay service: configuration, catalog, persistence ports and
//! the one open purchase of each account.
//!
//! LegionCore keeps `BattlepayManager::_actualTransaction` inside each
//! `WorldSession`. RustyCore's `WorldSession` field set is frozen, so the same
//! single-slot state lives here, keyed by game account (one world session per
//! account and realm). Keying by account also keeps a pending web checkout across
//! a relog, which LegionCore lost with the session object.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use wow_core::ObjectGuid;
use wow_persistence::{
    BattlePayAccountPersistencePortLikeCpp, BattlePayBnetGameAccountsLikeCpp,
    BattlePayBoostCompletionLikeCpp, BattlePayCharacterRowLikeCpp,
    BattlePayCharacterServicePersistencePortLikeCpp, BattlePayCharacterTransferLikeCpp,
    BattlePayDeliveryPersistencePortLikeCpp, BattlePayDeliveryReceiptLikeCpp,
    BattlePayDistributionAssignLikeCpp, BattlePayDistributionGrantLikeCpp,
    BattlePayDistributionPersistencePortLikeCpp, BattlePayDistributionRowLikeCpp,
    BattlePayPurchaseInsertLikeCpp, BattlePayPurchaseRowLikeCpp, BattlePaySsoTokenIssueLikeCpp,
    BattlePayTokenChargeLikeCpp, BattlePayTokenChargeOutcomeLikeCpp, PersistenceFutureLikeCpp,
    PersistenceOutcomeLikeCpp, PlayerInventoryPersistenceRequestLikeCpp,
};

use super::catalog::BattlePayCatalogLikeCpp;
use super::constants::BattlePayConfigLikeCpp;

/// Web checkout keys handed to the client in `SMSG_BATTLE_PAY_START_CHECKOUT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WebCheckoutLikeCpp {
    pub external_id: String,
    pub signature: String,
    /// LegionCore `Purchase::WebCheckoutPending`.
    pub pending: bool,
}

/// Destination of a character transfer (LegionCore `Purchase::VasTarget*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct VasTransferTargetLikeCpp {
    pub account_id: u32,
    pub battlenet_account_id: u32,
    pub realm_id: u32,
}

/// LegionCore `Battlepay::Purchase` (the fields this port uses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActivePurchaseLikeCpp {
    pub purchase_id: u64,
    pub client_token: u32,
    pub server_token: u32,
    pub product_id: u32,
    pub current_price: u64,
    pub status: u32,
    pub target_character: ObjectGuid,
    /// LegionCore `Purchase::Lock`: the order was confirmed (or failed) and must
    /// not be confirmed again until a new `StartPurchase`.
    pub lock: bool,
    pub web: Option<WebCheckoutLikeCpp>,
    /// Character transfer destination (`StartVasPurchase` of a transfer product).
    pub transfer: Option<VasTransferTargetLikeCpp>,
}

impl ActivePurchaseLikeCpp {
    pub(crate) fn pending_web_external_id(&self) -> Option<&str> {
        self.web
            .as_ref()
            .filter(|web| web.pending)
            .map(|web| web.external_id.as_str())
    }
}

/// The in-game shop service shared by every session of the process.
pub struct BattlePayServiceLikeCpp {
    pub(crate) config: BattlePayConfigLikeCpp,
    pub(crate) catalog: Arc<BattlePayCatalogLikeCpp>,
    pub(crate) account: Arc<dyn BattlePayAccountPersistencePortLikeCpp>,
    pub(crate) delivery: Arc<dyn BattlePayDeliveryPersistencePortLikeCpp>,
    pub(crate) distributions: Arc<dyn BattlePayDistributionPersistencePortLikeCpp>,
    pub(crate) characters: Arc<dyn BattlePayCharacterServicePersistencePortLikeCpp>,
    /// `CharacterLoadout.db2` + `CharacterLoadoutItem.db2`: `(class, purpose)` ->
    /// item ids of the lowest loadout id (LegionCore `GetItemLoadOutIdsBy`).
    boost_loadouts: HashMap<(u8, i32), Vec<u32>>,
    purchases: Mutex<HashMap<u32, ActivePurchaseLikeCpp>>,
    purchase_counter: AtomicU64,
    distribution_counter: AtomicU64,
}

impl BattlePayServiceLikeCpp {
    pub fn new(
        config: BattlePayConfigLikeCpp,
        catalog: Arc<BattlePayCatalogLikeCpp>,
        account: Arc<dyn BattlePayAccountPersistencePortLikeCpp>,
        delivery: Arc<dyn BattlePayDeliveryPersistencePortLikeCpp>,
    ) -> Self {
        Self {
            config,
            catalog,
            account,
            delivery,
            distributions: Arc::new(UnavailableBattlePayPersistenceLikeCpp),
            characters: Arc::new(UnavailableBattlePayPersistenceLikeCpp),
            boost_loadouts: HashMap::new(),
            purchases: Mutex::new(HashMap::new()),
            purchase_counter: AtomicU64::new(0),
            distribution_counter: AtomicU64::new(0),
        }
    }

    /// Attach the character-service ports (distributions in the Login DB,
    /// character rows in the Character DB).
    pub fn with_character_services(
        mut self,
        distributions: Arc<dyn BattlePayDistributionPersistencePortLikeCpp>,
        characters: Arc<dyn BattlePayCharacterServicePersistencePortLikeCpp>,
    ) -> Self {
        self.distributions = distributions;
        self.characters = characters;
        self
    }

    /// Disabled service with no database: every request answers "shop locked".
    pub fn disabled() -> Self {
        Self::new(
            BattlePayConfigLikeCpp::default(),
            Arc::new(BattlePayCatalogLikeCpp::default()),
            Arc::new(UnavailableBattlePayPersistenceLikeCpp),
            Arc::new(UnavailableBattlePayPersistenceLikeCpp),
        )
    }

    pub fn config(&self) -> &BattlePayConfigLikeCpp {
        &self.config
    }

    /// LegionCore `BattlepayManager::GenerateNewPurchaseID`.
    pub(crate) fn next_purchase_id_like_cpp(&self) -> u64 {
        0x1E77_8000_0000_0000 | (self.purchase_counter.fetch_add(1, Ordering::Relaxed) + 1)
    }

    /// Attach the boost gear table (`(class, purpose)` -> item ids).
    pub fn with_boost_loadouts(mut self, loadouts: HashMap<(u8, i32), Vec<u32>>) -> Self {
        self.boost_loadouts = loadouts;
        self
    }

    pub(crate) fn boost_loadout_like_cpp(&self, class: u8, purpose: i32) -> Option<&[u32]> {
        self.boost_loadouts
            .get(&(class, purpose))
            .map(Vec::as_slice)
    }

    /// LegionCore `BattlepayManager::GenerateNewDistributionId`:
    /// `(time << 20) | (++seq & 0xFFFFF)`.
    pub(crate) fn next_distribution_id_like_cpp(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let sequence = self.distribution_counter.fetch_add(1, Ordering::Relaxed) + 1;
        (now << 20) | (sequence & 0xF_FFFF)
    }

    pub(crate) fn purchase(&self, account_id: u32) -> Option<ActivePurchaseLikeCpp> {
        self.lock_purchases().get(&account_id).cloned()
    }

    pub(crate) fn set_purchase(&self, account_id: u32, purchase: ActivePurchaseLikeCpp) {
        self.lock_purchases().insert(account_id, purchase);
    }

    pub(crate) fn update_purchase<R>(
        &self,
        account_id: u32,
        update: impl FnOnce(&mut ActivePurchaseLikeCpp) -> R,
    ) -> Option<R> {
        self.lock_purchases().get_mut(&account_id).map(update)
    }

    fn lock_purchases(&self) -> std::sync::MutexGuard<'_, HashMap<u32, ActivePurchaseLikeCpp>> {
        self.purchases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Default for BattlePayServiceLikeCpp {
    fn default() -> Self {
        Self::disabled()
    }
}

/// Port implementation of [`BattlePayServiceLikeCpp::disabled`].
struct UnavailableBattlePayPersistenceLikeCpp;

const UNAVAILABLE: &str = "BattlePay persistence is not configured";

impl BattlePayAccountPersistencePortLikeCpp for UnavailableBattlePayPersistenceLikeCpp {
    fn load_token_balances_like_cpp(
        &self,
        _account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<(u8, i64)>, String>> {
        Box::pin(async { Err(UNAVAILABLE.to_owned()) })
    }

    fn charge_tokens_like_cpp(
        &self,
        _charge: BattlePayTokenChargeLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, BattlePayTokenChargeOutcomeLikeCpp> {
        Box::pin(async {
            BattlePayTokenChargeOutcomeLikeCpp::Failed {
                reason: UNAVAILABLE.to_owned(),
            }
        })
    }

    fn insert_web_purchase_like_cpp(
        &self,
        _purchase: BattlePayPurchaseInsertLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async { unavailable_outcome() })
    }

    fn load_purchase_like_cpp(
        &self,
        _external_id: String,
        _account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayPurchaseRowLikeCpp>, String>> {
        Box::pin(async { Err(UNAVAILABLE.to_owned()) })
    }

    fn load_paid_purchases_like_cpp(
        &self,
        _account_id: u32,
        _realm_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<BattlePayPurchaseRowLikeCpp>, String>> {
        Box::pin(async { Err(UNAVAILABLE.to_owned()) })
    }

    fn mark_purchase_delivered_like_cpp(
        &self,
        _external_id: String,
        _web_order_id: String,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async { unavailable_outcome() })
    }

    fn mark_purchase_failed_like_cpp(
        &self,
        _external_id: String,
        _account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async { unavailable_outcome() })
    }

    fn issue_sso_token_like_cpp(
        &self,
        _issue: BattlePaySsoTokenIssueLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, Result<String, String>> {
        Box::pin(async { Err(UNAVAILABLE.to_owned()) })
    }
}

impl BattlePayDeliveryPersistencePortLikeCpp for UnavailableBattlePayPersistenceLikeCpp {
    fn delivery_receipt_exists_like_cpp(
        &self,
        _external_id: String,
    ) -> PersistenceFutureLikeCpp<'_, Result<bool, String>> {
        Box::pin(async { Err(UNAVAILABLE.to_owned()) })
    }

    fn persist_delivery_like_cpp(
        &self,
        _receipt: BattlePayDeliveryReceiptLikeCpp,
        _inventory: Vec<PlayerInventoryPersistenceRequestLikeCpp>,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async { unavailable_outcome() })
    }
}

impl BattlePayDistributionPersistencePortLikeCpp for UnavailableBattlePayPersistenceLikeCpp {
    fn load_distributions_like_cpp(
        &self,
        _account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<BattlePayDistributionRowLikeCpp>, String>> {
        Box::pin(async { Err(UNAVAILABLE.to_owned()) })
    }

    fn grant_distribution_like_cpp(
        &self,
        _grant: BattlePayDistributionGrantLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async { unavailable_outcome() })
    }

    fn assign_distribution_like_cpp(
        &self,
        _assign: BattlePayDistributionAssignLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async { unavailable_outcome() })
    }

    fn unassign_distribution_like_cpp(
        &self,
        _distribution_id: u64,
        _account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async { unavailable_outcome() })
    }

    fn load_pending_distribution_like_cpp(
        &self,
        _character_guid: u64,
        _realm_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayDistributionRowLikeCpp>, String>> {
        Box::pin(async { Err(UNAVAILABLE.to_owned()) })
    }

    fn finish_distribution_like_cpp(
        &self,
        _distribution_id: u64,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async { unavailable_outcome() })
    }

    fn grant_undelete_like_cpp(
        &self,
        _battlenet_account_id: u32,
        _external_id: String,
        _web_order_id: String,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async { unavailable_outcome() })
    }

    fn load_account_battlenet_like_cpp(
        &self,
        _account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<u32>, String>> {
        Box::pin(async { Err(UNAVAILABLE.to_owned()) })
    }

    fn load_bnet_game_accounts_like_cpp(
        &self,
        _email: String,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayBnetGameAccountsLikeCpp>, String>>
    {
        Box::pin(async { Err(UNAVAILABLE.to_owned()) })
    }
}

impl BattlePayCharacterServicePersistencePortLikeCpp for UnavailableBattlePayPersistenceLikeCpp {
    fn load_account_characters_like_cpp(
        &self,
        _account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<BattlePayCharacterRowLikeCpp>, String>> {
        Box::pin(async { Err(UNAVAILABLE.to_owned()) })
    }

    fn load_character_like_cpp(
        &self,
        _character_guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayCharacterRowLikeCpp>, String>> {
        Box::pin(async { Err(UNAVAILABLE.to_owned()) })
    }

    fn persist_service_delivery_like_cpp(
        &self,
        _receipt: BattlePayDeliveryReceiptLikeCpp,
        _at_login_flags: u16,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async { unavailable_outcome() })
    }

    fn persist_transfer_delivery_like_cpp(
        &self,
        _receipt: BattlePayDeliveryReceiptLikeCpp,
        _transfer: BattlePayCharacterTransferLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async { unavailable_outcome() })
    }

    fn queue_character_boost_like_cpp(
        &self,
        _character_guid: u64,
        _account_id: u32,
        _level: u8,
        _at_login_flags: u16,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async { unavailable_outcome() })
    }

    fn persist_boost_completion_like_cpp(
        &self,
        _receipt: BattlePayDeliveryReceiptLikeCpp,
        _completion: BattlePayBoostCompletionLikeCpp,
        _inventory: Vec<PlayerInventoryPersistenceRequestLikeCpp>,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async { unavailable_outcome() })
    }
}

fn unavailable_outcome() -> PersistenceOutcomeLikeCpp {
    PersistenceOutcomeLikeCpp::Failed {
        reason: UNAVAILABLE.to_owned(),
    }
}
