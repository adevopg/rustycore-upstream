//! BattlePay request handling (LegionCore `Handlers/BattlePayHandler.cpp`).
//!
//! Every function is generic over [`BattlePaySessionLikeCpp`], the narrow view of a
//! world session the shop needs, so the purchase state machine is unit tested
//! against a fake session and fake persistence ports.
//!
//! Durability (the saga that replaces LegionCore's `ChangeTokenCount` +
//! `ProcessDelivery`, and `LOGIN_UPD_BPAY_PURCHASE_DELIVERED` + `ProcessDelivery`):
//!
//! 1. Charge (token mode): one Login DB transaction debits the wallet only if the
//!    balance covers the price, writes the `account_donate_token_log` row and a
//!    `battlepay_purchase` row already `Paid` (1). Web mode: the web tier marks the
//!    `Created` (0) row `Paid`.
//! 2. Deliver: the items (or the at-login flag of a character service, or the
//!    account move of a transfer) and a `character_battlepay_delivery` receipt
//!    keyed by the order's `external_id` commit in one Character DB transaction.
//!    A receipt that already exists means the order was delivered: nothing is
//!    granted again. Boosts and undelete services live in the Login DB: the
//!    distribution row (or the cooldown reset) commits with step 3 instead.
//! 3. Mark: `battlepay_purchase` `Paid -> Delivered` (2).
//!
//! A crash after 1 leaves a `Paid` row that the next product-list request of the
//! account delivers ([`deliver_paid_purchases`]); a crash after 2 leaves a `Paid`
//! row whose receipt exists, so the retry only performs 3. Charging twice needs two
//! confirmations of two distinct `StartPurchase`s, and a replayed confirmation is
//! refused by the purchase lock.

use std::collections::HashMap;
use std::future::Future;

use rand::Rng;
use tracing::{debug, info, warn};
use wow_core::{ObjectGuid, ObjectGuidGenerator};
use wow_packet::ServerPacket;
use wow_packet::packets::battlepay::{
    BattlePayAckFailed, BattlePayAckFailedResponse, BattlePayConfirmPurchase,
    BattlePayConfirmPurchaseResponse, BattlePayDeliveryEnded, BattlePayDeliveryStarted,
    BattlePayGetProductListResponse, BattlePayGetPurchaseListResponse, BattlePayMountDelivered,
    BattlePayPurchase, BattlePayPurchaseUpdate, BattlePayStartCheckout, BattlePayStartPurchase,
    BattlePayStartPurchaseResponse, DisplayPromotion,
};
use wow_packet::packets::item::ItemInstance;
use wow_persistence::{
    BattlePayDeliveryReceiptLikeCpp, BattlePayPurchaseInsertLikeCpp, BattlePayPurchaseRowLikeCpp,
    BattlePayTokenChargeLikeCpp, BattlePayTokenChargeOutcomeLikeCpp, PersistenceOutcomeLikeCpp,
    PlayerInventoryPersistenceRequestLikeCpp,
};

use super::catalog::{BattlePayProductLikeCpp, ProductListViewerLikeCpp};
use super::constants::*;
use super::product_kind::ProductKindLikeCpp;
use super::service::{
    ActivePurchaseLikeCpp, BattlePayServiceLikeCpp, VasTransferTargetLikeCpp, WebCheckoutLikeCpp,
};
use super::web::finish_web_purchase;

/// The logged-in character, when the session is in the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BattlePayPlayerLikeCpp {
    pub guid: ObjectGuid,
    /// `1 << (class - 1)`.
    pub class_mask: u32,
    pub class: u8,
    pub level: u8,
}

/// Session identity the shop reads (LegionCore `WorldSession` getters).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BattlePayIdentityLikeCpp {
    pub account_id: u32,
    pub battlenet_account_id: u32,
    pub realm_id: u32,
    /// `realm.Id.Region` (`SMSG_BATTLE_PAY_START_CHECKOUT.GameServiceRegionID`).
    pub region_id: u32,
    /// `realm.Id.GetAddress()` of this realm.
    pub virtual_realm_address: u32,
    /// `realm.Name` (the VAS character list is filtered by it).
    pub realm_name: String,
    pub security: u8,
    /// Numeric `LocaleConstant` of the session locale.
    pub locale: u8,
    pub ip: String,
    pub player: Option<BattlePayPlayerLikeCpp>,
}

/// The narrow session capability the shop consumes.
pub(crate) trait BattlePaySessionLikeCpp: Send {
    fn battle_pay_identity(&self) -> BattlePayIdentityLikeCpp;
    /// All BattlePay SMSG are `CONNECTION_TYPE_REALM`.
    fn send_battle_pay_packet<P: ServerPacket>(&self, packet: &P);
    /// LegionCore `BattlepayManager::AlreadyOwnProduct(itemId)`.
    fn battle_pay_item_owned(&self, item_id: u32) -> bool;
    /// Per-player item gates of LegionCore `ProductFilter` (allowable class).
    fn battle_pay_item_allowed(&self, item_id: u32) -> bool;
    /// `ITEM_CLASS_MISCELLANEOUS` / `ITEM_SUBCLASS_JUNK_MOUNT`.
    fn battle_pay_item_is_mount(&self, item_id: u32) -> bool;
    /// Every `(item, quantity)` fits the bags together (checked before charging).
    fn battle_pay_can_store(&self, items: &[(u32, u32)]) -> bool;
    /// Store the items in memory and return their rows; `None` when any item could
    /// not be stored after the runtime inventory may already have changed.
    fn battle_pay_grant_items(
        &mut self,
        item_guid_generator: &ObjectGuidGenerator,
        items: &[(u32, u32)],
    ) -> impl Future<Output = Option<Vec<PlayerInventoryPersistenceRequestLikeCpp>>> + Send;
    /// `Player::GetItemCount(item)` of the player in the world.
    fn battle_pay_item_count(&self, item_id: u32) -> u32;
    /// At-login flags of the player in the world (0 without one).
    fn battle_pay_player_at_login_flags(&self) -> u16;
    /// Mirror a durable at-login change onto the player in the world, so its next
    /// save keeps it (LegionCore `Player::SetAtLoginFlag` / `RemoveAtLoginFlag`).
    fn battle_pay_set_player_at_login_flags(&mut self, flags: u16);
    /// Mirror a durable money change onto the player in the world.
    fn battle_pay_add_player_money(&mut self, copper: u64);
    /// Disconnect a session whose runtime state diverged from the database.
    fn battle_pay_quarantine(&mut self, reason: &'static str);
}

/// Result of one delivery attempt of a paid order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryOutcomeLikeCpp {
    Delivered,
    /// The receipt already existed; only the order status was (re)marked.
    AlreadyDelivered,
    /// Nothing changed; the order stays `Paid` for a later attempt.
    Deferred(&'static str),
    /// The session was disconnected (runtime/database divergence).
    Quarantined,
}

/// One paid `battlepay_purchase` order and what the client knows it as.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PaidOrderLikeCpp {
    pub external_id: String,
    pub product_id: u32,
    pub web_order_id: String,
    /// Client `PurchaseID` of the delivery packets.
    pub purchase_id: u64,
    /// `battlepay_purchase.character_guid`: service/transfer target or item receiver.
    pub character_guid: u64,
    pub transfer: Option<VasTransferTargetLikeCpp>,
}

impl PaidOrderLikeCpp {
    pub(crate) fn from_row(
        row: &BattlePayPurchaseRowLikeCpp,
        web_order_id: &str,
        purchase_id: u64,
    ) -> Self {
        Self {
            external_id: row.external_id.clone(),
            product_id: row.product_id,
            web_order_id: web_order_id.to_owned(),
            purchase_id,
            character_guid: row.character_guid,
            transfer: (row.vas_target_account != 0).then_some(VasTransferTargetLikeCpp {
                account_id: row.vas_target_account,
                battlenet_account_id: row.vas_target_bnet_account,
                realm_id: row.vas_target_realm,
            }),
        }
    }
}

pub(crate) fn random_hex_32() -> String {
    let bytes: [u8; 16] = rand::thread_rng().r#gen();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn product_items(product: &BattlePayProductLikeCpp) -> Vec<(u32, u32)> {
    product
        .items
        .iter()
        .map(|item| (item.item_id, item.quantity.max(1)))
        .collect()
}

pub(crate) fn send_start_purchase_response<S: BattlePaySessionLikeCpp>(
    session: &S,
    purchase: &ActivePurchaseLikeCpp,
    result: u32,
) {
    session.send_battle_pay_packet(&BattlePayStartPurchaseResponse {
        purchase_id: purchase.purchase_id,
        purchase_result: result,
        client_token: purchase.client_token,
    });
}

/// LegionCore `SendPurchaseUpdate` (`UnkInt` = server token, now the third u64).
pub(crate) fn send_purchase_update<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    purchase: &ActivePurchaseLikeCpp,
    result: u32,
) {
    let mut wallet_name = service.config.wallet_name.clone();
    wallet_name.truncate(255);
    session.send_battle_pay_packet(&BattlePayPurchaseUpdate {
        purchases: vec![BattlePayPurchase {
            purchase_id: purchase.purchase_id,
            status: purchase.status,
            result_code: result,
            product_id: purchase.product_id,
            unk3: u64::from(purchase.server_token),
            wallet_name,
            ..BattlePayPurchase::default()
        }],
    });
}

pub(crate) fn send_ack_failed<S: BattlePaySessionLikeCpp>(
    session: &S,
    purchase: &ActivePurchaseLikeCpp,
    result: u32,
) {
    session.send_battle_pay_packet(&BattlePayAckFailed {
        purchase_id: purchase.purchase_id,
        server_token: purchase.server_token,
        status: purchase.status,
        result,
    });
}

async fn token_balances(service: &BattlePayServiceLikeCpp, account_id: u32) -> HashMap<u8, i64> {
    match service
        .account
        .load_token_balances_like_cpp(account_id)
        .await
    {
        Ok(rows) => rows.into_iter().collect(),
        Err(error) => {
            warn!(account = account_id, %error, "BattlePay: token balances unavailable");
            HashMap::new()
        }
    }
}

/// LegionCore `BattlepayManager::SendProductList`. Returns whether the shop is open.
pub(crate) async fn send_product_list<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
) -> bool {
    let identity = session.battle_pay_identity();
    if !service.config.is_available_for_like_cpp(identity.security) {
        session.send_battle_pay_packet(&BattlePayGetProductListResponse {
            result: PRODUCT_LIST_LOCKED_LIKE_CPP,
            ..BattlePayGetProductListResponse::default()
        });
        return false;
    }
    let balances = token_balances(service, identity.account_id).await;
    let owned = |item_id| session.battle_pay_item_owned(item_id);
    let allowed = |item_id| session.battle_pay_item_allowed(item_id);
    let viewer = ProductListViewerLikeCpp {
        in_world: identity.player.is_some(),
        locale: identity.locale,
        class_mask: identity.player.map_or(0, |player| player.class_mask),
        web_checkout: service.config.web_checkout,
        currency_id: service.config.currency_id_like_cpp(),
        token_balances: &balances,
        owned: &owned,
        item_allowed: &allowed,
    };
    let response = service.catalog.product_list_like_cpp(&viewer);
    session.send_battle_pay_packet(&response);
    true
}

/// LegionCore `WorldSession::SendDisplayPromo`, called from
/// `InitializeSessionCallback` right after the tutorial flags: `SMSG_DISPLAY_PROMOTION`
/// (promotion 0) and, when the shop is available, the distribution list. RustyCore
/// sends both only for an available shop so a disabled shop leaves the login
/// packet sequence unchanged.
pub(crate) async fn send_session_init<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
) {
    if !service
        .config
        .is_available_for_like_cpp(session.battle_pay_identity().security)
    {
        return;
    }
    session.send_battle_pay_packet(&DisplayPromotion { promotion_id: 0 });
    super::boost::send_distribution_list(session, service).await;
}

/// `CMSG_BATTLE_PAY_GET_PRODUCT_LIST` (LegionCore `HandleGetProductList`).
///
/// LegionCore only ran `DeliverPaidWebPurchases` in web mode; paid token orders
/// (a crash between charge and delivery) use the same recovery here.
pub(crate) async fn handle_get_product_list<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    item_guid_generator: &ObjectGuidGenerator,
) {
    if send_product_list(session, service).await {
        // Safety net for a client that missed the session-init copy (for example a
        // shop enabled while it sat at character select).
        super::boost::send_distribution_list(session, service).await;
        deliver_paid_purchases(session, service, item_guid_generator).await;
    }
}

/// `CMSG_BATTLE_PAY_REQUEST_PRICE_INFO` (LegionCore `HandleBattlePayRequestPriceInfo`).
pub(crate) async fn handle_request_price_info<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
) {
    send_product_list(session, service).await;
}

/// `CMSG_BATTLE_PAY_GET_PURCHASE_LIST`: LegionCore answers an empty list.
pub(crate) fn handle_get_purchase_list<S: BattlePaySessionLikeCpp>(session: &S) {
    session.send_battle_pay_packet(&BattlePayGetPurchaseListResponse::default());
}

/// Refusal of a `StartPurchase` (LegionCore `SendStartPurchaseResponse(error)`).
fn deny_start<S: BattlePaySessionLikeCpp>(
    session: &S,
    identity: &BattlePayIdentityLikeCpp,
    purchase: &ActivePurchaseLikeCpp,
    result: u32,
    why: &str,
) {
    info!(
        account = identity.account_id,
        product = purchase.product_id,
        result,
        "BattlePay: purchase refused: {why}"
    );
    send_start_purchase_response(session, purchase, result);
}

/// Purchase checks that depend on what the product delivers. `Ok` carries the
/// character the order is recorded for.
fn check_purchase_target<S: BattlePaySessionLikeCpp>(
    session: &S,
    identity: &BattlePayIdentityLikeCpp,
    product: &BattlePayProductLikeCpp,
    requested_target: ObjectGuid,
) -> Result<ObjectGuid, (u32, &'static str)> {
    let in_world = identity.player.map(|player| player.guid);
    match product.kind_like_cpp() {
        ProductKindLikeCpp::Items | ProductKindLikeCpp::Service(_) => {
            // Deliveries go to the character in the world. LegionCore accepted any
            // character of the account; the 3.4.3 flow never names another one in
            // game. Services bought outside the VAS flow are LegionCore's in-world
            // `CharacterService` deliveries (`ProcessDelivery`, `if (player)`).
            let Some(player) = in_world else {
                return Err((error::PURCHASE_DENIED, "no character in the world"));
            };
            if !requested_target.is_empty() && requested_target != player {
                return Err((error::PURCHASE_DENIED, "target is not the active character"));
            }
            if let ProductKindLikeCpp::Service(kind) = product.kind_like_cpp() {
                if kind
                    .already_flagged_error_like_cpp(session.battle_pay_player_at_login_flags())
                    .is_some()
                {
                    return Err((error::PURCHASE_DENIED, "service already pending"));
                }
                return Ok(player);
            }
            let items = product_items(product);
            if !session.battle_pay_can_store(&items) {
                return Err((error::PURCHASE_DENIED, "not enough free bag slots"));
            }
            if items
                .iter()
                .any(|(item_id, _)| session.battle_pay_item_owned(*item_id))
            {
                return Err((error::PURCHASE_DENIED, "already owned"));
            }
            Ok(player)
        }
        // Account-wide deliveries; LegionCore `CanBuy` of a boost is always true.
        ProductKindLikeCpp::Boost(_) | ProductKindLikeCpp::RestoreDeletedCharacter => {
            Ok(in_world.unwrap_or(ObjectGuid::EMPTY))
        }
        ProductKindLikeCpp::Transfer { .. } => Err((
            error::PURCHASE_DENIED,
            "a transfer is only sold through the VAS flow",
        )),
        ProductKindLikeCpp::Unsupported => {
            Err((error::PURCHASE_DENIED, "product type is not supported"))
        }
    }
}

/// `CMSG_BATTLE_PAY_START_PURCHASE` (LegionCore `MakePurchase`).
pub(crate) async fn handle_start_purchase<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    request: BattlePayStartPurchase,
) {
    let identity = session.battle_pay_identity();
    if !service.config.is_available_for_like_cpp(identity.security) {
        return;
    }
    let mut purchase = ActivePurchaseLikeCpp {
        purchase_id: 0,
        client_token: request.client_token,
        server_token: 0,
        product_id: request.product_id,
        current_price: 0,
        status: purchase_status::LOADING,
        target_character: request.target_character,
        lock: false,
        web: None,
        transfer: None,
    };
    let Some(product) = service.catalog.product(request.product_id) else {
        return deny_start(
            session,
            &identity,
            &purchase,
            error::PURCHASE_DENIED,
            "unknown product",
        );
    };
    let Some(group) = service.catalog.group_for_product(request.product_id) else {
        return deny_start(
            session,
            &identity,
            &purchase,
            error::PURCHASE_DENIED,
            "product is in no group",
        );
    };
    // LegionCore `MakePurchase`: "a VAS service is not bought here: give the screen
    // what it expects". The 54261 store waits for STORE_CHARACTER_LIST_RECEIVED and
    // then sends CMSG_BATTLE_PAY_START_VAS_PURCHASE.
    if product.is_vas_like_cpp() && product.is_deliverable_like_cpp() {
        super::vas::send_vas_lists_for_purchase(
            session,
            service,
            &identity,
            product,
            request.client_token,
        )
        .await;
        return;
    }
    let target = match check_purchase_target(session, &identity, product, request.target_character)
    {
        Ok(target) => target,
        Err((result, why)) => return deny_start(session, &identity, &purchase, result, why),
    };
    purchase.current_price = product.current_price;
    purchase.target_character = target;
    service.set_purchase(identity.account_id, purchase.clone());

    if !service.config.web_checkout {
        let balance = token_balances(service, identity.account_id)
            .await
            .get(&group.token_type)
            .copied()
            .unwrap_or(0);
        if balance < fixed_point_to_tokens_like_cpp(purchase.current_price) {
            return deny_start(
                session,
                &identity,
                &purchase,
                error::INSUFFICIENT_BALANCE,
                "insufficient balance",
            );
        }
    }
    begin_confirmation(session, service, &identity, purchase).await;
}

/// Accepted start: purchase ids, `StartPurchaseResponse` + `PurchaseUpdate`, then
/// the wallet confirmation or the web checkout (LegionCore `MakePurchase` tail and
/// `HandleBattlePayStartVasPurchaseCallback`).
pub(crate) async fn begin_confirmation<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    identity: &BattlePayIdentityLikeCpp,
    mut purchase: ActivePurchaseLikeCpp,
) {
    purchase.purchase_id = service.next_purchase_id_like_cpp();
    purchase.server_token = rand::thread_rng().gen_range(0..=0x0FFF_FFFF);
    service.set_purchase(identity.account_id, purchase.clone());
    send_start_purchase_response(session, &purchase, error::OK);
    send_purchase_update(session, service, &purchase, error::OK);

    if service.config.web_checkout {
        start_web_checkout(session, service, identity, purchase).await;
        return;
    }
    session.send_battle_pay_packet(&BattlePayConfirmPurchase {
        purchase_id: purchase.purchase_id,
        server_token: purchase.server_token,
    });
}

fn order_insert_like_cpp(
    identity: &BattlePayIdentityLikeCpp,
    purchase: &ActivePurchaseLikeCpp,
    external_id: String,
    price: String,
    currency: String,
    payment_ref: String,
) -> BattlePayPurchaseInsertLikeCpp {
    let transfer = purchase.transfer.unwrap_or_default();
    BattlePayPurchaseInsertLikeCpp {
        external_id,
        signature: random_hex_32(),
        battlenet_account_id: identity.battlenet_account_id,
        account_id: identity.account_id,
        realm_id: identity.realm_id,
        character_guid: purchase.target_character.counter() as u64,
        product_id: purchase.product_id,
        price,
        currency,
        ip: identity.ip.clone(),
        payment_ref,
        vas_target_account: transfer.account_id,
        vas_target_bnet_account: transfer.battlenet_account_id,
        vas_target_realm: transfer.realm_id,
    }
}

/// LegionCore `StartWebCheckout`: register the order and open the checkout.
async fn start_web_checkout<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    identity: &BattlePayIdentityLikeCpp,
    mut purchase: ActivePurchaseLikeCpp,
) {
    let web = WebCheckoutLikeCpp {
        external_id: random_hex_32(),
        signature: random_hex_32(),
        pending: true,
    };
    let mut insert = order_insert_like_cpp(
        identity,
        &purchase,
        web.external_id.clone(),
        fixed_point_to_decimal_like_cpp(purchase.current_price),
        service.config.currency_code.clone(),
        String::new(),
    );
    insert.signature = web.signature.clone();
    let outcome = service.account.insert_web_purchase_like_cpp(insert).await;
    if !outcome.is_applied() {
        warn!(
            account = identity.account_id,
            ?outcome,
            "BattlePay: web order was not recorded"
        );
        purchase.status = purchase_status::FINISH;
        purchase.lock = true;
        service.set_purchase(identity.account_id, purchase.clone());
        send_purchase_update(session, service, &purchase, error::OTHER);
        return;
    }
    purchase.web = Some(web.clone());
    service.set_purchase(identity.account_id, purchase.clone());
    session.send_battle_pay_packet(&BattlePayStartCheckout {
        product_id: purchase.product_id,
        game_service_region_id: identity.region_id,
        game_account_id: u64::from(identity.account_id),
        server_validation_signature: web.signature.clone(),
        external_transaction_id: web.external_id.clone(),
        subscription: false,
    });
    info!(
        account = identity.account_id,
        product = purchase.product_id,
        order = %web.external_id,
        price = %fixed_point_to_decimal_like_cpp(purchase.current_price),
        currency = %service.config.currency_code,
        "BattlePay: web checkout started"
    );
}

/// `CMSG_BATTLE_PAY_CONFIRM_PURCHASE_RESPONSE` (LegionCore `HandleBattlePayConfirmPurchase`).
pub(crate) async fn handle_confirm_purchase_response<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    item_guid_generator: &ObjectGuidGenerator,
    request: BattlePayConfirmPurchaseResponse,
) {
    let identity = session.battle_pay_identity();
    if !service.config.is_available_for_like_cpp(identity.security) {
        return;
    }
    let Some(mut purchase) = service.purchase(identity.account_id) else {
        return;
    };
    let deny = |purchase: &ActivePurchaseLikeCpp, result: u32, why: &str| {
        info!(
            account = identity.account_id,
            product = purchase.product_id,
            result,
            "BattlePay: confirmation refused: {why}"
        );
        send_purchase_update(session, service, purchase, result);
    };
    if purchase.lock {
        return deny(
            &purchase,
            error::PURCHASE_DENIED,
            "purchase already confirmed",
        );
    }
    if purchase.purchase_id == 0
        || purchase.server_token != request.server_token
        || !request.confirm_purchase
        || purchase.current_price != request.client_current_price_fixed_point
    {
        return deny(
            &purchase,
            error::PURCHASE_DENIED,
            "token, price or confirmation mismatch",
        );
    }
    let Some(group) = service.catalog.group_for_product(purchase.product_id) else {
        return deny(&purchase, error::PURCHASE_DENIED, "product is in no group");
    };
    let token_type = group.token_type;
    let Some(product) = service.catalog.product(purchase.product_id) else {
        return deny(&purchase, error::PURCHASE_DENIED, "unknown product");
    };
    if service.config.web_checkout || !product.is_deliverable_like_cpp() {
        return deny(&purchase, error::PURCHASE_DENIED, "not a wallet purchase");
    }
    let kind = product.kind_like_cpp();

    purchase.lock = true;
    purchase.status = purchase_status::FINISH;
    service.set_purchase(identity.account_id, purchase.clone());

    if kind.needs_player_in_world() {
        let items = product_items(product);
        if identity.player.is_none() {
            return deny(
                &purchase,
                error::PURCHASE_DENIED,
                "no character in the world",
            );
        }
        if !session.battle_pay_can_store(&items) {
            return deny(
                &purchase,
                error::PURCHASE_DENIED,
                "not enough free bag slots",
            );
        }
        if items
            .iter()
            .any(|(item_id, _)| session.battle_pay_item_owned(*item_id))
        {
            return deny(&purchase, error::PURCHASE_DENIED, "already owned");
        }
    }

    let tokens = fixed_point_to_tokens_like_cpp(purchase.current_price);
    let external_id = random_hex_32();
    let charge = BattlePayTokenChargeLikeCpp {
        token_type,
        amount: tokens,
        buy_type: BUY_TYPE_BATTLE_PAY_SHOP_LIKE_CPP,
        purchase: order_insert_like_cpp(
            &identity,
            &purchase,
            external_id.clone(),
            tokens.to_string(),
            TOKEN_ORDER_CURRENCY_LIKE_CPP.to_owned(),
            format!("tokens:{token_type}"),
        ),
    };
    match service.account.charge_tokens_like_cpp(charge).await {
        BattlePayTokenChargeOutcomeLikeCpp::Charged => {
            info!(
                account = identity.account_id,
                product = purchase.product_id,
                tokens,
                order = %external_id,
                "BattlePay: wallet charged"
            );
        }
        BattlePayTokenChargeOutcomeLikeCpp::InsufficientBalance { balance } => {
            debug!(
                account = identity.account_id,
                balance, tokens, "BattlePay: balance too low"
            );
            return deny(
                &purchase,
                error::INSUFFICIENT_BALANCE,
                "insufficient balance",
            );
        }
        BattlePayTokenChargeOutcomeLikeCpp::Failed { reason } => {
            warn!(account = identity.account_id, %reason, "BattlePay: wallet charge rolled back");
            return deny(&purchase, error::PAYMENT_FAILED, "wallet charge failed");
        }
        BattlePayTokenChargeOutcomeLikeCpp::Unknown { reason } => {
            // If it committed, the Paid row is delivered by the next recovery pass.
            warn!(
                account = identity.account_id,
                order = %external_id,
                %reason,
                "BattlePay: wallet charge outcome unknown; the order is recovered on the next store refresh"
            );
            return deny(&purchase, error::OTHER, "wallet charge outcome unknown");
        }
    }

    let order = PaidOrderLikeCpp {
        external_id: external_id.clone(),
        product_id: purchase.product_id,
        web_order_id: String::new(),
        purchase_id: purchase.purchase_id,
        character_guid: purchase.target_character.counter() as u64,
        transfer: purchase.transfer,
    };
    // Order matters, as in LegionCore `HandleBattlePayConfirmPurchase` (`SendPurchaseUpdate`
    // then `ProcessDelivery`): the purchase update must reach the client BEFORE any delivery
    // packet. A boost delivery sends `BattlePayDistributionUpdate`, whose Lua handler
    // (`PRODUCT_DISTRIBUTIONS_UPDATED` -> `StoreFrame_OnCharacterBoostDelivered`) clears
    // `JustOrderedBoost`/`JustFinishedOrdering` but not `JustOrderedProduct`; a purchase update
    // arriving afterwards then re-arms `JustFinishedOrdering` and the "Purchase sent" panel
    // reappears every time the store is opened.
    send_purchase_update(session, service, &purchase, error::OK);
    let outcome = deliver_order_like_cpp(session, service, item_guid_generator, &order).await;
    match outcome {
        DeliveryOutcomeLikeCpp::Delivered | DeliveryOutcomeLikeCpp::AlreadyDelivered => {}
        DeliveryOutcomeLikeCpp::Deferred(why) => {
            warn!(
                account = identity.account_id,
                order = %external_id,
                "BattlePay: paid order not delivered yet ({why}); it is retried on the next store refresh"
            );
        }
        DeliveryOutcomeLikeCpp::Quarantined => {}
    }
}

/// Mark a delivered order (`Paid -> Delivered`); the receipt already prevents a
/// second delivery if this is lost.
pub(crate) async fn mark_delivered(service: &BattlePayServiceLikeCpp, order: &PaidOrderLikeCpp) {
    match service
        .account
        .mark_purchase_delivered_like_cpp(order.external_id.clone(), order.web_order_id.clone())
        .await
    {
        PersistenceOutcomeLikeCpp::Applied { .. } => {}
        outcome => warn!(
            order = %order.external_id,
            ?outcome,
            "BattlePay: order delivered but not marked; the receipt prevents a second delivery"
        ),
    }
}

/// Whether the order's receipt exists in this realm's Character DB.
pub(crate) async fn receipt_exists(
    service: &BattlePayServiceLikeCpp,
    order: &PaidOrderLikeCpp,
) -> Result<bool, DeliveryOutcomeLikeCpp> {
    service
        .delivery
        .delivery_receipt_exists_like_cpp(order.external_id.clone())
        .await
        .map_err(|error| {
            warn!(order = %order.external_id, %error, "BattlePay: delivery receipt lookup failed");
            DeliveryOutcomeLikeCpp::Deferred("delivery receipt lookup failed")
        })
}

/// Deliver one `Paid` order exactly once per realm (LegionCore `ProcessDelivery`).
pub(crate) async fn deliver_order_like_cpp<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    item_guid_generator: &ObjectGuidGenerator,
    order: &PaidOrderLikeCpp,
) -> DeliveryOutcomeLikeCpp {
    let Some(product) = service
        .catalog
        .product(order.product_id)
        .filter(|product| product.is_deliverable_like_cpp())
    else {
        return DeliveryOutcomeLikeCpp::Deferred("unknown or undeliverable product");
    };
    match product.kind_like_cpp() {
        ProductKindLikeCpp::Items => {
            deliver_items(session, service, item_guid_generator, order, product).await
        }
        ProductKindLikeCpp::Service(kind) => {
            super::vas::deliver_service(session, service, order, kind).await
        }
        ProductKindLikeCpp::Transfer { faction_change } => {
            super::transfer::deliver_transfer(session, service, order, faction_change).await
        }
        ProductKindLikeCpp::Boost(_) => {
            super::boost::deliver_boost(session, service, order, product).await
        }
        ProductKindLikeCpp::RestoreDeletedCharacter => {
            super::vas::deliver_undelete(session, service, order).await
        }
        ProductKindLikeCpp::Unsupported => {
            DeliveryOutcomeLikeCpp::Deferred("unknown or undeliverable product")
        }
    }
}

/// `WebsiteType::Item` / `ItemMount`: the items go to the character in the world.
async fn deliver_items<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    item_guid_generator: &ObjectGuidGenerator,
    order: &PaidOrderLikeCpp,
    product: &BattlePayProductLikeCpp,
) -> DeliveryOutcomeLikeCpp {
    let identity = session.battle_pay_identity();
    let Some(player) = identity.player else {
        return DeliveryOutcomeLikeCpp::Deferred("no character in the world");
    };
    let items = product_items(product);
    let already_delivered = match receipt_exists(service, order).await {
        Ok(exists) => exists,
        Err(outcome) => return outcome,
    };

    if !already_delivered {
        if !session.battle_pay_can_store(&items) {
            return DeliveryOutcomeLikeCpp::Deferred("not enough free bag slots");
        }
        // Ignored by the 54261 client (handler slot is a `ret` stub); sent per spec.
        session.send_battle_pay_packet(&BattlePayDeliveryStarted {
            distribution_id: order.purchase_id,
        });
        let Some(inventory) = session
            .battle_pay_grant_items(item_guid_generator, &items)
            .await
        else {
            session.battle_pay_quarantine("BattlePay item delivery diverged; relog required");
            return DeliveryOutcomeLikeCpp::Quarantined;
        };
        let receipt = BattlePayDeliveryReceiptLikeCpp {
            external_id: order.external_id.clone(),
            account_id: identity.account_id,
            character_guid: player.guid.counter() as u64,
            product_id: order.product_id,
        };
        match service
            .delivery
            .persist_delivery_like_cpp(receipt, inventory)
            .await
        {
            PersistenceOutcomeLikeCpp::Applied { .. } => {}
            outcome => {
                // Nothing durable was granted (or it cannot be told): the order
                // stays Paid and the receipt decides the next attempt.
                warn!(order = %order.external_id, ?outcome, "BattlePay: delivery did not commit");
                session.battle_pay_quarantine("BattlePay delivery did not commit; relog required");
                return DeliveryOutcomeLikeCpp::Quarantined;
            }
        }
    }

    mark_delivered(service, order).await;
    if already_delivered {
        info!(order = %order.external_id, "BattlePay: order was already delivered; status repaired");
        return DeliveryOutcomeLikeCpp::AlreadyDelivered;
    }

    // SMSG_BATTLE_PAY_MOUNT_DELIVERED makes the UI refresh the owned state.
    if items
        .iter()
        .any(|(item_id, _)| session.battle_pay_item_is_mount(*item_id))
    {
        session.send_battle_pay_packet(&BattlePayMountDelivered {
            product_id: order.product_id,
        });
    }
    session.send_battle_pay_packet(&BattlePayDeliveryEnded {
        distribution_id: order.purchase_id,
        items: items
            .iter()
            .map(|(item_id, _)| ItemInstance {
                item_id: *item_id as i32,
                ..ItemInstance::default()
            })
            .collect(),
    });
    info!(
        account = identity.account_id,
        product = order.product_id,
        order = %order.external_id,
        "BattlePay: order delivered"
    );
    DeliveryOutcomeLikeCpp::Delivered
}

/// LegionCore `DeliverPaidWebPurchases`: deliver every `Paid` order of the account
/// that this realm created (web payments whose notification was lost, and wallet
/// charges interrupted before delivery). Item orders wait for a character in the
/// world; services, transfers, boosts and undeletes are delivered from character
/// select as well.
pub(crate) async fn deliver_paid_purchases<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    item_guid_generator: &ObjectGuidGenerator,
) {
    let identity = session.battle_pay_identity();
    let rows = match service
        .account
        .load_paid_purchases_like_cpp(identity.account_id, identity.realm_id)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            warn!(account = identity.account_id, %error, "BattlePay: paid orders unavailable");
            return;
        }
    };
    for row in rows {
        let needs_player = service
            .catalog
            .product(row.product_id)
            .is_none_or(|product| product.kind_like_cpp().needs_player_in_world());
        if needs_player && identity.player.is_none() {
            continue;
        }
        let pending = service.purchase(identity.account_id).filter(|purchase| {
            purchase.pending_web_external_id() == Some(row.external_id.as_str())
        });
        let purchase_id = pending
            .as_ref()
            .map(|purchase| purchase.purchase_id)
            .unwrap_or_else(|| service.next_purchase_id_like_cpp());
        let order = PaidOrderLikeCpp::from_row(&row, "", purchase_id);
        // Purchase update before the delivery packets (LegionCore order; see the note in
        // `handle_confirm_purchase`).
        if let Some(mut purchase) = pending {
            finish_web_purchase(
                session,
                service,
                identity.account_id,
                &mut purchase,
                error::OK,
            );
        }
        let outcome = deliver_order_like_cpp(session, service, item_guid_generator, &order).await;
        match outcome {
            DeliveryOutcomeLikeCpp::Delivered | DeliveryOutcomeLikeCpp::AlreadyDelivered => {}
            DeliveryOutcomeLikeCpp::Deferred(why) => {
                debug!(order = %row.external_id, "BattlePay: paid order deferred: {why}");
            }
            DeliveryOutcomeLikeCpp::Quarantined => return,
        }
    }
}

/// `CMSG_BATTLE_PAY_ACK_FAILED_RESPONSE` (LegionCore `HandleBattlePayAckFailedResponse`).
pub(crate) fn handle_ack_failed_response<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    request: BattlePayAckFailedResponse,
) {
    let account_id = session.battle_pay_identity().account_id;
    let released = service.update_purchase(account_id, |purchase| {
        if purchase.server_token != request.server_token {
            return false;
        }
        purchase.lock = false;
        if let Some(web) = purchase.web.as_mut() {
            web.pending = false;
        }
        purchase.status = purchase_status::LOADING;
        true
    });
    if released != Some(true) {
        debug!(
            account = account_id,
            token = request.server_token,
            "BattlePay: ack of an unknown failure"
        );
    }
}
