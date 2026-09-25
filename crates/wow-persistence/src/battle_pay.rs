//! SQLx-free contracts for the in-game shop (BattlePay).
//!
//! Port of LegionCore 7.3.5 `BattlePayDataStoreMgr` (world catalog loaders,
//! `src/server/game/Globals/BattlePayData.cpp`), the `LOGIN_*_BPAY_*` /
//! `LOGIN_*_TOKEN*` statements used by `BattlePayHandler.cpp` and
//! `Player::ChangeTokenCount`, and a RustyCore-only Character DB delivery
//! receipt that makes item delivery idempotent across the Login/Character
//! database split (see `docs/migration/battlepay-343-protocol.md`, section 8).

use crate::{
    PersistenceFutureLikeCpp, PersistenceOutcomeLikeCpp, PlayerInventoryPersistenceRequestLikeCpp,
};

// ── World catalog ─────────────────────────────────────────────────────────

/// `battlepay_product`. Prices are read as whole cents (`DECIMAL(12,2)` * 100).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayProductRowLikeCpp {
    pub product_id: u32,
    pub normal_price_cents: u64,
    pub current_price_cents: u64,
    pub product_type: u8,
    /// LegionCore reads a NULL `WebsiteType` as 0 (`Field::GetUInt8`).
    pub website_type: u8,
    pub choice_type: u8,
    pub flags: u32,
    pub display_info_id: u32,
    pub class_mask: u32,
    pub script_name: String,
    pub game_time_days: u16,
}

/// `battlepay_product_item`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayProductItemRowLikeCpp {
    pub id: u32,
    pub product_id: u32,
    pub item_id: u32,
    pub quantity: u32,
    /// NULL is read as 0 (no own card).
    pub display_info_id: u32,
    pub pet_result: u8,
}

/// `battlepay_product_group`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayProductGroupRowLikeCpp {
    pub group_id: u32,
    pub name: String,
    pub icon_file_data_id: u32,
    pub display_type: u8,
    pub ordering: u32,
    pub flags: u32,
    pub token_type: u8,
    pub ingame_only: bool,
    pub owns_tokens_only: bool,
}

/// `battlepay_product_group_locales`; `locale` is the numeric `LocaleConstant`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayProductGroupLocaleRowLikeCpp {
    pub group_id: u32,
    pub locale: u32,
    pub name: String,
}

/// `battlepay_shop_entry`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayShopEntryRowLikeCpp {
    pub entry_id: u32,
    pub group_id: u32,
    pub product_id: u32,
    pub ordering: i32,
    pub flags: u32,
    pub banner_type: u8,
    pub display_info_id: u32,
}

/// `battlepay_display_info`; a NULL `FileDataID` is read as 0.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDisplayInfoRowLikeCpp {
    pub display_info_id: u32,
    pub creature_display_info_id: u32,
    pub file_data_id: u32,
    pub flags: u32,
    pub names: [String; 4],
}

/// `battlepay_display_info_locales`; NULL names are read as empty strings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDisplayInfoLocaleRowLikeCpp {
    pub display_info_id: u32,
    pub locale: u32,
    pub names: [String; 4],
}

/// `battlepay_display_info_visuals`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDisplayInfoVisualRowLikeCpp {
    pub display_info_id: u32,
    pub display_id: u32,
    pub visual_id: u32,
    pub product_name: String,
}

/// `battlepay_tokens`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayTokenTypeRowLikeCpp {
    pub token_type: u8,
    pub name: String,
    pub login_message: Option<String>,
    pub list_if_none: bool,
}

/// Every BattlePay world table, in LegionCore `BattlePayDataStoreMgr::Initialize` order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayCatalogRowsLikeCpp {
    pub display_infos: Vec<BattlePayDisplayInfoRowLikeCpp>,
    pub visuals: Vec<BattlePayDisplayInfoVisualRowLikeCpp>,
    pub products: Vec<BattlePayProductRowLikeCpp>,
    pub product_items: Vec<BattlePayProductItemRowLikeCpp>,
    pub groups: Vec<BattlePayProductGroupRowLikeCpp>,
    pub shop_entries: Vec<BattlePayShopEntryRowLikeCpp>,
    pub group_locales: Vec<BattlePayProductGroupLocaleRowLikeCpp>,
    pub display_info_locales: Vec<BattlePayDisplayInfoLocaleRowLikeCpp>,
    pub token_types: Vec<BattlePayTokenTypeRowLikeCpp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BattlePayCatalogLoadOutcomeLikeCpp {
    Loaded(BattlePayCatalogRowsLikeCpp),
    Failed { reason: String },
}

pub trait BattlePayCatalogPersistencePortLikeCpp: Send + Sync {
    fn load_rows_like_cpp(
        &self,
    ) -> PersistenceFutureLikeCpp<'_, BattlePayCatalogLoadOutcomeLikeCpp>;
}

// ── Login database: wallets, orders, SSO tokens ───────────────────────────

/// `auth.battlepay_purchase.status` (LegionCore `Battlepay::WebPurchaseStatus`).
pub const BATTLE_PAY_PURCHASE_STATUS_CREATED_LIKE_CPP: u8 = 0;
pub const BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP: u8 = 1;
pub const BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP: u8 = 2;
pub const BATTLE_PAY_PURCHASE_STATUS_FAILED_LIKE_CPP: u8 = 3;

/// Identity and price of one `auth.battlepay_purchase` row written by the realm.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayPurchaseInsertLikeCpp {
    pub external_id: String,
    pub signature: String,
    pub battlenet_account_id: u32,
    pub account_id: u32,
    pub realm_id: u32,
    pub character_guid: u64,
    pub product_id: u32,
    /// Decimal text written into `price DECIMAL(12,2)` (`"15.00"`), exact.
    pub price: String,
    /// ISO 4217 code (`Bpay.Currency`) or `TOK` for a token-wallet order.
    pub currency: String,
    pub ip: String,
    /// `payment_ref`: empty for web orders, `tokens:<type>` for wallet orders.
    pub payment_ref: String,
}

/// One token-wallet purchase: the balance decrement, the
/// `account_donate_token_log` row (LegionCore `Player::ChangeTokenCount`) and the
/// paid `battlepay_purchase` order commit in one Login DB transaction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayTokenChargeLikeCpp {
    pub token_type: u8,
    /// Whole tokens to spend (>= 0).
    pub amount: i64,
    /// `account_donate_token_log.buyType` (0 = BattlePayShop).
    pub buy_type: u8,
    pub purchase: BattlePayPurchaseInsertLikeCpp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BattlePayTokenChargeOutcomeLikeCpp {
    /// The balance was debited and the paid order row exists.
    Charged,
    /// The balance read in the same attempt is below the price; nothing written.
    InsufficientBalance { balance: i64 },
    /// The transaction definitely rolled back.
    Failed { reason: String },
    /// The COMMIT outcome is unknown; the paid row, if it exists, is recovered
    /// by the next paid-order delivery pass.
    Unknown { reason: String },
}

/// Columns of `battlepay_purchase` the realm reads back.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayPurchaseRowLikeCpp {
    pub id: u64,
    pub external_id: String,
    pub product_id: u32,
    pub status: u8,
    pub character_guid: u64,
    pub payment_ref: String,
    pub web_order_id: String,
}

/// Kind 1 (SSO) `battlenet_account_web_token` row for the checkout browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattlePaySsoTokenIssueLikeCpp {
    pub battlenet_account_id: u32,
    pub account_id: u32,
    pub realm_id: u32,
    pub character_guid: u64,
    pub ip: String,
    pub lifetime_secs: u32,
    /// 32 random bytes rendered as the 64-hex token.
    pub random_bytes: [u8; 32],
}

pub trait BattlePayAccountPersistencePortLikeCpp: Send + Sync {
    /// `SELECT tokenType, amount FROM account_tokens WHERE account_id = ?`.
    fn load_token_balances_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<(u8, i64)>, String>>;

    fn charge_tokens_like_cpp(
        &self,
        charge: BattlePayTokenChargeLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, BattlePayTokenChargeOutcomeLikeCpp>;

    /// LegionCore `LOGIN_INS_BPAY_PURCHASE` (status 0 Created).
    fn insert_web_purchase_like_cpp(
        &self,
        purchase: BattlePayPurchaseInsertLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// LegionCore `LOGIN_SEL_BPAY_PURCHASE_BY_EXTERNAL_ID`.
    fn load_purchase_like_cpp(
        &self,
        external_id: String,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayPurchaseRowLikeCpp>, String>>;

    /// LegionCore `LOGIN_SEL_BPAY_PURCHASES_PAID`, restricted to the realm that
    /// created the order (its Character DB holds the delivery receipt).
    fn load_paid_purchases_like_cpp(
        &self,
        account_id: u32,
        realm_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<BattlePayPurchaseRowLikeCpp>, String>>;

    /// LegionCore `LOGIN_UPD_BPAY_PURCHASE_DELIVERED` (`status 1 -> 2`), keyed by
    /// `external_id`. `Applied { rows }` reports whether this call moved it.
    fn mark_purchase_delivered_like_cpp(
        &self,
        external_id: String,
        web_order_id: String,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// LegionCore `LOGIN_UPD_BPAY_PURCHASE_FAILED` (`status 0 -> 3`).
    fn mark_purchase_failed_like_cpp(
        &self,
        external_id: String,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// LegionCore `Battlenet::AuthenticationService::IssueToken(0x576F57, 1)`.
    fn issue_sso_token_like_cpp(
        &self,
        issue: BattlePaySsoTokenIssueLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, Result<String, String>>;
}

// ── Character database: idempotent delivery ───────────────────────────────

/// `character_battlepay_delivery` receipt; its primary key is the order's
/// `external_id`, so an order is delivered into this realm at most once.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDeliveryReceiptLikeCpp {
    pub external_id: String,
    pub account_id: u32,
    pub character_guid: u64,
    pub product_id: u32,
}

pub trait BattlePayDeliveryPersistencePortLikeCpp: Send + Sync {
    fn delivery_receipt_exists_like_cpp(
        &self,
        external_id: String,
    ) -> PersistenceFutureLikeCpp<'_, Result<bool, String>>;

    /// Receipt plus every item row of the delivery in one Character DB transaction.
    fn persist_delivery_like_cpp(
        &self,
        receipt: BattlePayDeliveryReceiptLikeCpp,
        inventory: Vec<PlayerInventoryPersistenceRequestLikeCpp>,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;
}
