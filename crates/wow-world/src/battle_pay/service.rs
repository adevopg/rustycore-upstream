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
    BattlePayAccountPersistencePortLikeCpp, BattlePayDeliveryPersistencePortLikeCpp,
    BattlePayDeliveryReceiptLikeCpp, BattlePayPurchaseInsertLikeCpp, BattlePayPurchaseRowLikeCpp,
    BattlePaySsoTokenIssueLikeCpp, BattlePayTokenChargeLikeCpp, BattlePayTokenChargeOutcomeLikeCpp,
    PersistenceFutureLikeCpp, PersistenceOutcomeLikeCpp, PlayerInventoryPersistenceRequestLikeCpp,
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
    purchases: Mutex<HashMap<u32, ActivePurchaseLikeCpp>>,
    purchase_counter: AtomicU64,
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
            purchases: Mutex::new(HashMap::new()),
            purchase_counter: AtomicU64::new(0),
        }
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

fn unavailable_outcome() -> PersistenceOutcomeLikeCpp {
    PersistenceOutcomeLikeCpp::Failed {
        reason: UNAVAILABLE.to_owned(),
    }
}
