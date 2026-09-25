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
//! 2. Deliver: the items and a `character_battlepay_delivery` receipt keyed by the
//!    order's `external_id` commit in one Character DB transaction. A receipt that
//!    already exists means the items were delivered: nothing is granted again.
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
    BattlePayAckFailed, BattlePayAckFailedResponse, BattlePayCancelOpenCheckout,
    BattlePayConfirmPurchase, BattlePayConfirmPurchaseResponse, BattlePayDeliveryEnded,
    BattlePayDeliveryStarted, BattlePayGetProductListResponse, BattlePayGetPurchaseListResponse,
    BattlePayMountDelivered, BattlePayOpenCheckout, BattlePayPurchase, BattlePayPurchaseSubmitted,
    BattlePayPurchaseUpdate, BattlePayStartCheckout, BattlePayStartPurchase,
    BattlePayStartPurchaseResponse, EnumVasPurchaseStatesResponse, GenerateSsoTokenResponse,
};
use wow_packet::packets::item::ItemInstance;
use wow_persistence::{
    BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP, BATTLE_PAY_PURCHASE_STATUS_FAILED_LIKE_CPP,
    BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP, BattlePayDeliveryReceiptLikeCpp,
    BattlePayPurchaseInsertLikeCpp, BattlePaySsoTokenIssueLikeCpp, BattlePayTokenChargeLikeCpp,
    BattlePayTokenChargeOutcomeLikeCpp, PersistenceOutcomeLikeCpp,
    PlayerInventoryPersistenceRequestLikeCpp,
};

use super::catalog::{BattlePayProductLikeCpp, ProductListViewerLikeCpp};
use super::constants::*;
use super::service::{ActivePurchaseLikeCpp, BattlePayServiceLikeCpp, WebCheckoutLikeCpp};

/// The logged-in character, when the session is in the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BattlePayPlayerLikeCpp {
    pub guid: ObjectGuid,
    /// `1 << (class - 1)`.
    pub class_mask: u32,
}

/// Session identity the shop reads (LegionCore `WorldSession` getters).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BattlePayIdentityLikeCpp {
    pub account_id: u32,
    pub battlenet_account_id: u32,
    pub realm_id: u32,
    /// `realm.Id.Region` (`SMSG_BATTLE_PAY_START_CHECKOUT.GameServiceRegionID`).
    pub region_id: u32,
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

fn random_hex_32() -> String {
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

fn send_start_purchase_response<S: BattlePaySessionLikeCpp>(
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
fn send_purchase_update<S: BattlePaySessionLikeCpp>(
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

fn send_ack_failed<S: BattlePaySessionLikeCpp>(
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

/// `CMSG_UPDATE_VAS_PURCHASE_STATES`: no VAS services are ported, so the list the
/// character screen waits for is always empty.
pub(crate) fn handle_update_vas_purchase_states<S: BattlePaySessionLikeCpp>(session: &S) {
    session.send_battle_pay_packet(&EnumVasPurchaseStatesResponse::default());
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
    };
    let deny = |purchase: &ActivePurchaseLikeCpp, result: u32, why: &str| {
        info!(
            account = identity.account_id,
            product = request.product_id,
            result,
            "BattlePay: purchase refused: {why}"
        );
        send_start_purchase_response(session, purchase, result);
    };

    // Deliveries go to the character in the world. LegionCore accepted any
    // character of the account; the 3.4.3 flow never names another one in game.
    let Some(player) = identity.player else {
        return deny(
            &purchase,
            error::PURCHASE_DENIED,
            "no character in the world",
        );
    };
    if !request.target_character.is_empty() && request.target_character != player.guid {
        return deny(
            &purchase,
            error::PURCHASE_DENIED,
            "target is not the active character",
        );
    }
    let Some(product) = service.catalog.product(request.product_id) else {
        return deny(&purchase, error::PURCHASE_DENIED, "unknown product");
    };
    let Some(group) = service.catalog.group_for_product(request.product_id) else {
        return deny(&purchase, error::PURCHASE_DENIED, "product is in no group");
    };
    if !product.is_deliverable_like_cpp() {
        return deny(
            &purchase,
            error::PURCHASE_DENIED,
            "product type is not supported",
        );
    }
    purchase.current_price = product.current_price;
    purchase.target_character = player.guid;
    service.set_purchase(identity.account_id, purchase.clone());

    if !service.config.web_checkout {
        let balance = token_balances(service, identity.account_id)
            .await
            .get(&group.token_type)
            .copied()
            .unwrap_or(0);
        if balance < fixed_point_to_tokens_like_cpp(purchase.current_price) {
            return deny(
                &purchase,
                error::INSUFFICIENT_BALANCE,
                "insufficient balance",
            );
        }
    }
    let items = product_items(product);
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

    purchase.purchase_id = service.next_purchase_id_like_cpp();
    purchase.server_token = rand::thread_rng().gen_range(0..=0x0FFF_FFFF);
    service.set_purchase(identity.account_id, purchase.clone());
    send_start_purchase_response(session, &purchase, error::OK);
    send_purchase_update(session, service, &purchase, error::OK);

    if service.config.web_checkout {
        start_web_checkout(session, service, &identity, purchase).await;
        return;
    }
    session.send_battle_pay_packet(&BattlePayConfirmPurchase {
        purchase_id: purchase.purchase_id,
        server_token: purchase.server_token,
    });
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
    let outcome = service
        .account
        .insert_web_purchase_like_cpp(BattlePayPurchaseInsertLikeCpp {
            external_id: web.external_id.clone(),
            signature: web.signature.clone(),
            battlenet_account_id: identity.battlenet_account_id,
            account_id: identity.account_id,
            realm_id: identity.realm_id,
            character_guid: purchase.target_character.counter() as u64,
            product_id: purchase.product_id,
            price: fixed_point_to_decimal_like_cpp(purchase.current_price),
            currency: service.config.currency_code.clone(),
            ip: identity.ip.clone(),
            payment_ref: String::new(),
        })
        .await;
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
    let Some(player) = identity.player else {
        return deny(
            &purchase,
            error::PURCHASE_DENIED,
            "no character in the world",
        );
    };
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

    purchase.lock = true;
    purchase.status = purchase_status::FINISH;
    service.set_purchase(identity.account_id, purchase.clone());

    let items = product_items(product);
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

    let tokens = fixed_point_to_tokens_like_cpp(purchase.current_price);
    let external_id = random_hex_32();
    let charge = BattlePayTokenChargeLikeCpp {
        token_type,
        amount: tokens,
        buy_type: BUY_TYPE_BATTLE_PAY_SHOP_LIKE_CPP,
        purchase: BattlePayPurchaseInsertLikeCpp {
            external_id: external_id.clone(),
            signature: random_hex_32(),
            battlenet_account_id: identity.battlenet_account_id,
            account_id: identity.account_id,
            realm_id: identity.realm_id,
            character_guid: player.guid.counter() as u64,
            product_id: purchase.product_id,
            price: tokens.to_string(),
            currency: TOKEN_ORDER_CURRENCY_LIKE_CPP.to_owned(),
            ip: identity.ip.clone(),
            payment_ref: format!("tokens:{token_type}"),
        },
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

    let outcome = deliver_order_like_cpp(
        session,
        service,
        item_guid_generator,
        &external_id,
        purchase.product_id,
        "",
        purchase.purchase_id,
    )
    .await;
    match outcome {
        DeliveryOutcomeLikeCpp::Delivered | DeliveryOutcomeLikeCpp::AlreadyDelivered => {
            send_purchase_update(session, service, &purchase, error::OK);
        }
        DeliveryOutcomeLikeCpp::Deferred(why) => {
            warn!(
                account = identity.account_id,
                order = %external_id,
                "BattlePay: paid order not delivered yet ({why}); it is retried on the next store refresh"
            );
            send_purchase_update(session, service, &purchase, error::OTHER);
        }
        DeliveryOutcomeLikeCpp::Quarantined => {}
    }
}

/// Deliver one `Paid` order into the character in the world, exactly once per realm.
pub(crate) async fn deliver_order_like_cpp<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    item_guid_generator: &ObjectGuidGenerator,
    external_id: &str,
    product_id: u32,
    web_order_id: &str,
    purchase_id: u64,
) -> DeliveryOutcomeLikeCpp {
    let identity = session.battle_pay_identity();
    let Some(player) = identity.player else {
        return DeliveryOutcomeLikeCpp::Deferred("no character in the world");
    };
    let Some(product) = service
        .catalog
        .product(product_id)
        .filter(|product| product.is_deliverable_like_cpp())
    else {
        return DeliveryOutcomeLikeCpp::Deferred("unknown or undeliverable product");
    };
    let items = product_items(product);
    let already_delivered = match service
        .delivery
        .delivery_receipt_exists_like_cpp(external_id.to_owned())
        .await
    {
        Ok(exists) => exists,
        Err(error) => {
            warn!(order = %external_id, %error, "BattlePay: delivery receipt lookup failed");
            return DeliveryOutcomeLikeCpp::Deferred("delivery receipt lookup failed");
        }
    };

    if !already_delivered {
        if !session.battle_pay_can_store(&items) {
            return DeliveryOutcomeLikeCpp::Deferred("not enough free bag slots");
        }
        // Ignored by the 54261 client (handler slot is a `ret` stub); sent per spec.
        session.send_battle_pay_packet(&BattlePayDeliveryStarted {
            distribution_id: purchase_id,
        });
        let Some(inventory) = session
            .battle_pay_grant_items(item_guid_generator, &items)
            .await
        else {
            session.battle_pay_quarantine("BattlePay item delivery diverged; relog required");
            return DeliveryOutcomeLikeCpp::Quarantined;
        };
        let receipt = BattlePayDeliveryReceiptLikeCpp {
            external_id: external_id.to_owned(),
            account_id: identity.account_id,
            character_guid: player.guid.counter() as u64,
            product_id,
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
                warn!(order = %external_id, ?outcome, "BattlePay: delivery did not commit");
                session.battle_pay_quarantine("BattlePay delivery did not commit; relog required");
                return DeliveryOutcomeLikeCpp::Quarantined;
            }
        }
    }

    match service
        .account
        .mark_purchase_delivered_like_cpp(external_id.to_owned(), web_order_id.to_owned())
        .await
    {
        PersistenceOutcomeLikeCpp::Applied { .. } => {}
        outcome => warn!(
            order = %external_id,
            ?outcome,
            "BattlePay: order delivered but not marked; the receipt prevents a second delivery"
        ),
    }
    if already_delivered {
        info!(order = %external_id, "BattlePay: order was already delivered; status repaired");
        return DeliveryOutcomeLikeCpp::AlreadyDelivered;
    }

    // SMSG_BATTLE_PAY_MOUNT_DELIVERED makes the UI refresh the owned state.
    if items
        .iter()
        .any(|(item_id, _)| session.battle_pay_item_is_mount(*item_id))
    {
        session.send_battle_pay_packet(&BattlePayMountDelivered { product_id });
    }
    session.send_battle_pay_packet(&BattlePayDeliveryEnded {
        distribution_id: purchase_id,
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
        product = product_id,
        order = %external_id,
        "BattlePay: order delivered"
    );
    DeliveryOutcomeLikeCpp::Delivered
}

/// LegionCore `DeliverPaidWebPurchases`: deliver every `Paid` order of the account
/// that this realm created (web payments whose notification was lost, and wallet
/// charges interrupted before delivery).
pub(crate) async fn deliver_paid_purchases<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    item_guid_generator: &ObjectGuidGenerator,
) {
    let identity = session.battle_pay_identity();
    if identity.player.is_none() {
        return;
    }
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
        let pending = service.purchase(identity.account_id).filter(|purchase| {
            purchase.pending_web_external_id() == Some(row.external_id.as_str())
        });
        let purchase_id = pending
            .as_ref()
            .map(|purchase| purchase.purchase_id)
            .unwrap_or_else(|| service.next_purchase_id_like_cpp());
        let outcome = deliver_order_like_cpp(
            session,
            service,
            item_guid_generator,
            &row.external_id,
            row.product_id,
            "",
            purchase_id,
        )
        .await;
        match outcome {
            DeliveryOutcomeLikeCpp::Delivered | DeliveryOutcomeLikeCpp::AlreadyDelivered => {
                if let Some(mut purchase) = pending {
                    finish_web_purchase(
                        session,
                        service,
                        identity.account_id,
                        &mut purchase,
                        error::OK,
                    );
                }
            }
            DeliveryOutcomeLikeCpp::Deferred(why) => {
                debug!(order = %row.external_id, "BattlePay: paid order deferred: {why}");
            }
            DeliveryOutcomeLikeCpp::Quarantined => return,
        }
    }
}

fn finish_web_purchase<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    account_id: u32,
    purchase: &mut ActivePurchaseLikeCpp,
    result: u32,
) {
    if let Some(web) = purchase.web.as_mut() {
        web.pending = false;
    }
    purchase.status = purchase_status::FINISH;
    service.set_purchase(account_id, purchase.clone());
    send_purchase_update(session, service, purchase, result);
}

/// `CMSG_BATTLE_PAY_OPEN_CHECKOUT` 0x3714: the 3.4.3 SSO token request. The client
/// keys the reply by `RequestID`, echoed as `GenerateSsoTokenResponse.Kind`.
pub(crate) async fn handle_open_checkout<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    request: BattlePayOpenCheckout,
) {
    let identity = session.battle_pay_identity();
    let mut response = GenerateSsoTokenResponse {
        kind: request.request_id,
        result: SSO_RESULT_DENIED_LIKE_CPP,
        ..GenerateSsoTokenResponse::default()
    };
    let pending = service
        .purchase(identity.account_id)
        .is_some_and(|purchase| purchase.pending_web_external_id().is_some());
    if !service.config.browser_enabled || !service.config.web_checkout || !pending {
        debug!(
            account = identity.account_id,
            pending, "BattlePay: SSO token refused"
        );
        session.send_battle_pay_packet(&response);
        return;
    }
    let issue = BattlePaySsoTokenIssueLikeCpp {
        battlenet_account_id: identity.battlenet_account_id,
        account_id: identity.account_id,
        realm_id: identity.realm_id,
        character_guid: identity
            .player
            .map_or(0, |player| player.guid.counter() as u64),
        ip: identity.ip.clone(),
        lifetime_secs: service.config.token_lifetime_secs,
        random_bytes: rand::thread_rng().r#gen(),
    };
    match service.account.issue_sso_token_like_cpp(issue).await {
        Ok(token) => {
            response.result = SSO_RESULT_OK_LIKE_CPP;
            response.token = token;
        }
        Err(error) => {
            warn!(account = identity.account_id, %error, "BattlePay: SSO token not issued")
        }
    }
    session.send_battle_pay_packet(&response);
}

/// `CMSG_BATTLE_PAY_PURCHASE_SUBMITTED` 0x371a (LegionCore
/// `HandleBattlePayPurchaseSubmitted` + callback). The web must already have
/// marked the order `Paid`; the client is not trusted for that.
pub(crate) async fn handle_purchase_submitted<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    item_guid_generator: &ObjectGuidGenerator,
    request: BattlePayPurchaseSubmitted,
) {
    if !service.config.web_checkout {
        return;
    }
    let identity = session.battle_pay_identity();
    let Some(mut purchase) = service.purchase(identity.account_id).filter(|purchase| {
        purchase.pending_web_external_id() == Some(request.external_transaction_id.as_str())
    }) else {
        warn!(
            account = identity.account_id,
            order = %request.external_transaction_id,
            "BattlePay: submitted order is not the pending checkout"
        );
        return;
    };
    let row = match service
        .account
        .load_purchase_like_cpp(request.external_transaction_id.clone(), identity.account_id)
        .await
    {
        Ok(row) => row,
        Err(error) => {
            warn!(account = identity.account_id, %error, "BattlePay: submitted order unreadable");
            return;
        }
    };
    let Some(row) = row else {
        warn!(order = %request.external_transaction_id, "BattlePay: submitted order does not exist");
        fail_web_purchase(session, service, identity.account_id, &mut purchase);
        return;
    };
    match row.status {
        BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP if row.product_id == purchase.product_id => {}
        BATTLE_PAY_PURCHASE_STATUS_FAILED_LIKE_CPP => {
            fail_web_purchase(session, service, identity.account_id, &mut purchase);
            return;
        }
        BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP => {
            finish_web_purchase(
                session,
                service,
                identity.account_id,
                &mut purchase,
                error::OK,
            );
            return;
        }
        status => {
            // Not paid (yet): the order stays pending; the web can still complete it
            // and the next store refresh delivers it.
            info!(
                order = %request.external_transaction_id,
                status,
                product = row.product_id,
                "BattlePay: submitted order is not paid yet"
            );
            return;
        }
    }
    let outcome = deliver_order_like_cpp(
        session,
        service,
        item_guid_generator,
        &row.external_id,
        row.product_id,
        &request.global_order_id,
        purchase.purchase_id,
    )
    .await;
    match outcome {
        DeliveryOutcomeLikeCpp::Delivered | DeliveryOutcomeLikeCpp::AlreadyDelivered => {
            finish_web_purchase(
                session,
                service,
                identity.account_id,
                &mut purchase,
                error::OK,
            );
        }
        DeliveryOutcomeLikeCpp::Deferred(why) => {
            warn!(order = %row.external_id, "BattlePay: paid order deferred: {why}");
        }
        DeliveryOutcomeLikeCpp::Quarantined => {}
    }
}

/// LegionCore callback path: `WebCheckoutPending = false; Lock = true;
/// SendAckFailed(PaymentFailed)`.
fn fail_web_purchase<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    account_id: u32,
    purchase: &mut ActivePurchaseLikeCpp,
) {
    if let Some(web) = purchase.web.as_mut() {
        web.pending = false;
    }
    purchase.lock = true;
    service.set_purchase(account_id, purchase.clone());
    send_ack_failed(session, purchase, error::PAYMENT_FAILED);
}

/// `CMSG_BATTLE_PAY_CANCEL_OPEN_CHECKOUT` 0x371b (LegionCore
/// `HandleBattlePayCancelOpenCheckout` + callback).
pub(crate) async fn handle_cancel_open_checkout<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    item_guid_generator: &ObjectGuidGenerator,
    request: BattlePayCancelOpenCheckout,
) {
    if !service.config.web_checkout {
        return;
    }
    let identity = session.battle_pay_identity();
    // The client resends this on every later close: only the pending order counts.
    let Some(mut purchase) = service.purchase(identity.account_id).filter(|purchase| {
        purchase.pending_web_external_id() == Some(request.external_transaction_id.as_str())
            && !purchase.lock
    }) else {
        debug!(order = %request.external_transaction_id, "BattlePay: cancel of a non-pending checkout");
        return;
    };
    let row = match service
        .account
        .load_purchase_like_cpp(request.external_transaction_id.clone(), identity.account_id)
        .await
    {
        Ok(row) => row,
        Err(error) => {
            warn!(account = identity.account_id, %error, "BattlePay: cancelled order unreadable");
            return;
        }
    };
    if let Some(row) = row
        .as_ref()
        .filter(|row| row.status == BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP)
    {
        // The web confirmed the payment right before the window closed.
        let outcome = deliver_order_like_cpp(
            session,
            service,
            item_guid_generator,
            &row.external_id,
            row.product_id,
            "",
            purchase.purchase_id,
        )
        .await;
        if matches!(
            outcome,
            DeliveryOutcomeLikeCpp::Delivered | DeliveryOutcomeLikeCpp::AlreadyDelivered
        ) {
            finish_web_purchase(
                session,
                service,
                identity.account_id,
                &mut purchase,
                error::OK,
            );
        }
        return;
    }
    // bnet-shop appends ";failed" to payment_ref when the provider rejected it.
    let result = if row
        .as_ref()
        .is_some_and(|row| row.payment_ref.ends_with(";failed"))
    {
        error::PAYMENT_FAILED
    } else {
        error::OK
    };
    let failed = service
        .account
        .mark_purchase_failed_like_cpp(request.external_transaction_id.clone(), identity.account_id)
        .await;
    if !failed.is_applied() {
        warn!(order = %request.external_transaction_id, ?failed, "BattlePay: cancelled order not marked failed");
    }
    info!(
        account = identity.account_id,
        order = %request.external_transaction_id,
        result,
        "BattlePay: web checkout closed without payment"
    );
    finish_web_purchase(session, service, identity.account_id, &mut purchase, result);
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
