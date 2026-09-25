//! BattlePay enums, configuration and price arithmetic.
//!
//! Values come from LegionCore 7.3.5 `BattlePayMgr.h` unless stated otherwise;
//! the 3.4.3.54261 client numbers are not recoverable from its Lua
//! (`docs/migration/battlepay-343-protocol.md`, section 7, open question 1).

/// LegionCore `Battlepay::g_CurrencyPrecision`: client prices are units * 10000.
pub(crate) const CURRENCY_PRECISION_LIKE_CPP: u64 = 10_000;

/// Purchase / product-list result codes (LegionCore `Battlepay::Error`).
///
/// TODO-verify (protocol doc section 7 question 1): these are the 7.3.5
/// `Enum.StoreError` values; the 54261 client special-cases `Result` 60 and 63
/// in `SMSG_BATTLE_PAY_ACK_FAILED`, which suggests the table moved.
pub(crate) mod error {
    pub(crate) const OK: u32 = 0;
    pub(crate) const PURCHASE_DENIED: u32 = 1;
    pub(crate) const PAYMENT_FAILED: u32 = 2;
    pub(crate) const OTHER: u32 = 3;
    pub(crate) const INSUFFICIENT_BALANCE: u32 = 28;
}

/// `SMSG_BATTLE_PAY_GET_PRODUCT_LIST_RESPONSE` result (LegionCore `ProductListResult`).
pub(crate) const PRODUCT_LIST_AVAILABLE_LIKE_CPP: u32 = 0;
pub(crate) const PRODUCT_LIST_LOCKED_LIKE_CPP: u32 = 1;

/// `JamBattlePayPurchase.Status` (LegionCore `Battlepay::UpdateStatus`, TODO-verify).
pub(crate) mod purchase_status {
    pub(crate) const LOADING: u32 = 9;
    pub(crate) const FINISH: u32 = 3;
}

/// `GenerateSsoTokenResponse.Result`: 0 delivers the token, anything else makes the
/// client callback run without one (client handler `0x14148bd60`).
pub(crate) const SSO_RESULT_OK_LIKE_CPP: u32 = 0;
pub(crate) const SSO_RESULT_DENIED_LIKE_CPP: u32 = 1;

/// LegionCore `Battlepay::WebsiteType` values RustyCore can deliver.
pub(crate) const WEBSITE_TYPE_ITEM_LIKE_CPP: u8 = 3;
pub(crate) const WEBSITE_TYPE_ITEM_MOUNT_LIKE_CPP: u8 = 21;
/// LegionCore `Battlepay::MaxWebsiteType` (GameTime = 31 is the last value).
pub(crate) const WEBSITE_TYPE_MAX_LIKE_CPP: u8 = 32;
/// LegionCore `PRODUCT_TYPE_WOW_TOKEN` / `WOW_TOKEN_GROUP_ID`.
pub(crate) const PRODUCT_TYPE_WOW_TOKEN_LIKE_CPP: u8 = 2;
pub(crate) const WOW_TOKEN_GROUP_ID_LIKE_CPP: u32 = 30;
/// LegionCore `BattlepayDisplayInfoFlag::HidePrice`.
pub(crate) const DISPLAY_FLAG_HIDE_PRICE_LIKE_CPP: u32 = 0x8;
/// LegionCore `BattlepayCustomType::BattlePayShop` (`account_donate_token_log.buyType`).
pub(crate) const BUY_TYPE_BATTLE_PAY_SHOP_LIKE_CPP: u8 = 0;
/// LegionCore `ProductInfoStruct::UnkInt2 = 47` ("2 ?"), the u32 right after the
/// product-id count; kept byte-identical until a 54261 capture names it.
pub(crate) const PRODUCT_INFO_UNK1_LIKE_CPP: u32 = 47;
/// LegionCore `FeatureSystemStatus::BpayStoreProductDeliveryDelay`.
pub(crate) const PRODUCT_DELIVERY_DELAY_SECS_LIKE_CPP: u32 = 180;
/// `AccountTypes::SEC_MODERATOR`: LegionCore `BattlepayManager::IsAvailable` opens the
/// shop to moderators even with `FeatureSystem.BpayStore.Enabled = 0`.
pub(crate) const SEC_MODERATOR_LIKE_CPP: u8 = 1;
/// `battlepay_purchase.currency` of a token-wallet order.
pub(crate) const TOKEN_ORDER_CURRENCY_LIKE_CPP: &str = "TOK";

/// LegionCore `Battlepay::CurrencyFromString` (7.3.5 client currency ids).
/// TODO-verify: 54261 `C_StoreSecure.GetCurrencyInfo` ids were not dumped.
pub(crate) fn currency_id_from_code_like_cpp(code: &str) -> u32 {
    match code.to_ascii_uppercase().as_str() {
        "USD" => 1,
        "GBP" => 2,
        "KRW" => 3,
        "EUR" => 4,
        "RUB" => 5,
        "ARS" => 8,
        "CLP" => 9,
        "MXN" => 10,
        "BRL" => 11,
        "AUD" => 12,
        "CPT" => 14,
        "TPT" => 15,
        "BETA" => 16,
        "JPY" => 28,
        "CAD" => 29,
        "NZD" => 30,
        _ => 0,
    }
}

/// TrinityCore `LocaleConstant` of a client locale name; `None` for unknown names.
pub(crate) fn locale_index_from_name_like_cpp(name: &str) -> Option<u8> {
    Some(match name {
        "enUS" | "enGB" => 0,
        "koKR" => 1,
        "frFR" => 2,
        "deDE" => 3,
        "zhCN" => 4,
        "zhTW" => 5,
        "esES" => 6,
        "esMX" => 7,
        "ruRU" => 8,
        "ptBR" | "ptPT" => 10,
        "itIT" => 11,
        _ => return None,
    })
}

/// Whether a numeric `Locale` column value names a real client locale
/// (`LOCALE_none` = 9 and values >= `TOTAL_LOCALES` are skipped like C++).
pub(crate) fn is_valid_locale_index_like_cpp(locale: u32) -> bool {
    locale < 12 && locale != 9
}

/// `battlepay_product` price column in cents -> client fixed point.
pub(crate) fn cents_to_fixed_point_like_cpp(cents: u64) -> u64 {
    cents.saturating_mul(CURRENCY_PRECISION_LIKE_CPP / 100)
}

/// LegionCore `Battlepay::FixedPointToTokens`: whole tokens, rounded up.
pub(crate) fn fixed_point_to_tokens_like_cpp(fixed_point: u64) -> i64 {
    fixed_point
        .div_ceil(CURRENCY_PRECISION_LIKE_CPP)
        .min(i64::MAX as u64) as i64
}

/// Exact decimal text of a fixed-point price for `battlepay_purchase.price`.
pub(crate) fn fixed_point_to_decimal_like_cpp(fixed_point: u64) -> String {
    let cents = fixed_point / (CURRENCY_PRECISION_LIKE_CPP / 100);
    format!("{}.{:02}", cents / 100, cents % 100)
}

/// Process configuration read once at startup (`worldserver.conf`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattlePayConfigLikeCpp {
    /// `Bpay.Enabled` (RustyCore master switch, default off).
    pub enabled: bool,
    /// `FeatureSystem.BpayStore.Enabled` (LegionCore `IsAvailable` for players).
    pub store_enabled_for_players: bool,
    /// `Bpay.WebCheckout`: real-money checkout in the in-game browser.
    pub web_checkout: bool,
    /// `Bpay.Currency` ISO code (`EUR`).
    pub currency_code: String,
    /// `Bpay.WalletName`.
    pub wallet_name: String,
    /// `Browser.Enabled`.
    pub browser_enabled: bool,
    /// `Browser.TokenLifetime` in seconds.
    pub token_lifetime_secs: u32,
}

impl Default for BattlePayConfigLikeCpp {
    fn default() -> Self {
        Self {
            enabled: false,
            store_enabled_for_players: false,
            web_checkout: false,
            currency_code: "EUR".to_owned(),
            wallet_name: "Donation points".to_owned(),
            browser_enabled: false,
            token_lifetime_secs: 3600,
        }
    }
}

impl BattlePayConfigLikeCpp {
    /// LegionCore `BattlepayManager::IsAvailable` behind the `Bpay.Enabled` switch.
    pub(crate) fn is_available_for_like_cpp(&self, security: u8) -> bool {
        self.enabled && (self.store_enabled_for_players || security >= SEC_MODERATOR_LIKE_CPP)
    }

    pub(crate) fn currency_id_like_cpp(&self) -> u32 {
        currency_id_from_code_like_cpp(&self.currency_code)
    }
}
