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
pub(crate) const WEBSITE_TYPE_RENAME_LIKE_CPP: u8 = 5;
pub(crate) const WEBSITE_TYPE_FACTION_LIKE_CPP: u8 = 9;
pub(crate) const WEBSITE_TYPE_RACE_LIKE_CPP: u8 = 10;
pub(crate) const WEBSITE_TYPE_DELETED_CHARACTER_LIKE_CPP: u8 = 15;
pub(crate) const WEBSITE_TYPE_CUSTOMIZATION_LIKE_CPP: u8 = 22;
pub(crate) const WEBSITE_TYPE_CHARACTER_BOOST_LIKE_CPP: u8 = 29;

/// LegionCore `Battlepay::ProductChoiceTypeVas` (`BattlePayMgr.h:547`). The 54261
/// store treats a product info whose ChoiceType is one of these as a VAS product
/// (client `0x141a49440`) and maps it to `Enum.VasServiceType` (`0x141a45845`:
/// 7 -> NameChange 0, 8 -> FactionChange 1, 9 -> AppearanceChange 2,
/// 10 -> RaceChange 3, 15 -> CharacterTransfer 4, 16 -> FactionTransfer 5).
pub(crate) const CHOICE_TYPE_VAS_NAME_CHANGE_LIKE_CPP: u8 = 7;
pub(crate) const CHOICE_TYPE_VAS_FACTION_CHANGE_LIKE_CPP: u8 = 8;
pub(crate) const CHOICE_TYPE_VAS_APPEARANCE_CHANGE_LIKE_CPP: u8 = 9;
pub(crate) const CHOICE_TYPE_VAS_RACE_CHANGE_LIKE_CPP: u8 = 10;
pub(crate) const CHOICE_TYPE_VAS_CHARACTER_TRANSFER_LIKE_CPP: u8 = 15;
pub(crate) const CHOICE_TYPE_VAS_FACTION_TRANSFER_LIKE_CPP: u8 = 16;

/// `JamBattlePayProduct.Type` of a character upgrade: the 54261 client counts a
/// distribution as a boost only when its product has Type 1 and reads the boost
/// type from the u32 after ItemId (`0x14169aaa0`, `0x1416989c0`, `0x141a45786`).
pub(crate) const PRODUCT_TYPE_CHARACTER_UPGRADE_LIKE_CPP: u8 = 1;

/// Player `AtLoginFlags` (TC 3.4.3 `Player.h:533-542`; 0x400 and 0x800 are the
/// LegionCore `AT_LOGIN_CHARACTER_BOOST` / `AT_LOGIN_BOOST_REVOKED` extensions,
/// `Player.h:577-591`, unused by TrinityCore 3.4.3).
pub(crate) mod at_login {
    pub(crate) const RENAME: u16 = 0x001;
    pub(crate) const CUSTOMIZE: u16 = 0x008;
    pub(crate) const CHANGE_FACTION: u16 = 0x040;
    pub(crate) const CHANGE_RACE: u16 = 0x080;
    pub(crate) const CHARACTER_BOOST: u16 = 0x400;
}

/// A character boost of WoW Classic 3.4.3: the `CharacterServiceInfo.db2` row
/// (54261 build, rows 119/131/145: BoostType 5/7/9 with levels 58/70/80) and the
/// `CharacterLoadout.db2` purpose holding its gear (purpose 10/12/15: item level
/// 52/125/187 sets, one loadout per class; inferred from the item levels, the
/// client exposes no purpose name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BoostDefinitionLikeCpp {
    pub boost_type: u32,
    pub level: u8,
    pub loadout_purpose: i32,
}

pub(crate) const BOOST_DEFINITIONS_LIKE_CPP: [BoostDefinitionLikeCpp; 3] = [
    BoostDefinitionLikeCpp {
        boost_type: 5,
        level: 58,
        loadout_purpose: 10,
    },
    BoostDefinitionLikeCpp {
        boost_type: 7,
        level: 70,
        loadout_purpose: 12,
    },
    BoostDefinitionLikeCpp {
        boost_type: 9,
        level: 80,
        loadout_purpose: 15,
    },
];

/// LegionCore `BattlepayManager::GetBoostLevel` adapted: the level is read from the
/// product's ScriptName ("80", "70" or "58"); a boost product without one of them
/// is the WotLK Classic level-70 upgrade.
pub(crate) fn boost_definition_for_script_like_cpp(script_name: &str) -> BoostDefinitionLikeCpp {
    let pick = |level| {
        BOOST_DEFINITIONS_LIKE_CPP
            .iter()
            .copied()
            .find(|definition| definition.level == level)
            .expect("boost level table")
    };
    if script_name.contains("80") {
        pick(80)
    } else if script_name.contains("58") {
        pick(58)
    } else {
        pick(70)
    }
}

/// `Enum.VasError` values of the 54261 client (Lua enum table registered at
/// `0x140e6ce91..`, values read from the registration code).
pub(crate) mod vas_error {
    pub(crate) const CHARACTER_HAS_VAS_PENDING: u32 = 4;
    pub(crate) const INVALID_DESTINATION_ACCOUNT: u32 = 6;
    pub(crate) const INVALID_SOURCE_ACCOUNT: u32 = 7;
    pub(crate) const CANNOT_MOVE_GUILD_MASTER: u32 = 20012;
    pub(crate) const MAX_CHARACTERS_ON_SERVER: u32 = 20013;
    pub(crate) const UNDER_MIN_LEVEL_REQ: u32 = 20021;
    pub(crate) const INELIGIBLE_TARGET_REALM: u32 = 20022;
    pub(crate) const CHAR_LOCKED: u32 = 20026;
    pub(crate) const ALREADY_RENAME_FLAGGED: u32 = 20055;
    pub(crate) const CUSTOMIZE_ALREADY_REQUESTED: u32 = 20057;
    pub(crate) const BATTLEPAY_DELIVERY_PENDING: u32 = 20078;
}

/// `Enum.VasPurchaseProgress` of the 54261 client (same registration block).
pub(crate) mod vas_progress {
    pub(crate) const INVALID: u32 = 0;
}

/// `Enum.VasQueueStatus.UnderAnHour` (transfers are applied immediately).
pub(crate) const VAS_QUEUE_UNDER_AN_HOUR_LIKE_CPP: u8 = 0;

/// Minimum character level of every VAS service in the 54261 glue/store UI
/// (`CheckAddVASErrorCode(Enum.VasError.UnderMinLevelReq)` for level < 10).
pub(crate) const VAS_MIN_CHARACTER_LEVEL_LIKE_CPP: u8 = 10;

/// `CONFIG_CHARACTERS_PER_REALM` default (TC `CharactersPerRealm = 10`).
pub(crate) const CHARACTERS_PER_REALM_LIKE_CPP: usize = 10;
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
    /// `Bpay.Boost.Money`: copper added by a boost (LegionCore `ApplyBoost`
    /// `ModifyMoney(5000000)`).
    pub boost_money: u64,
    /// `CharactersPerRealm` (transfer destination capacity).
    pub characters_per_realm: usize,
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
            boost_money: 5_000_000,
            characters_per_realm: CHARACTERS_PER_REALM_LIKE_CPP,
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
