//! Value-added services (VAS) of the 54261 store: character list, VAS purchase
//! start and the delivery of the at-login character services.
//!
//! LegionCore anchors: `SendVasCharacterList` / `MakePurchase` VAS branch
//! (`BattlePayHandler.cpp:338-448`), `HandleUpdateVasPurchaseStates` (186),
//! `HandleBattlePayStartVasPurchase` + callback (1397, 1606),
//! `CharacterService::{SetRename,ChangeFaction,ChangeRace,Customize,
//! RestoreDeletedCharacter}` (`CharacterService.cpp`). The 54261 flow (client
//! evidence in `docs/migration/battlepay-343-protocol.md`, section 9):
//! `PurchaseProduct` -> 0x36d3 -> server 0x27f1 (or 0x27f2 for a transfer, after
//! which the client asks 0x36f8 itself) -> STORE_CHARACTER_LIST_RECEIVED ->
//! `PurchaseVASProduct` -> 0x36fa -> 0x2783 + 0x2786 + 0x2787 (wallet) or 0x2824
//! (web) -> confirmation -> delivery -> 0x2786 + 0x27f4 STORE_VAS_PURCHASE_COMPLETE.
//! Validation failures answer 0x27f3 with `Enum.VasError` codes
//! (STORE_VAS_PURCHASE_ERROR).

use tracing::{info, warn};
use wow_core::ObjectGuid;
use wow_core::guid::HighGuid;
use wow_packet::packets::battlepay::{
    BattlePayStartVasPurchase, EnumVasPurchaseStatesResponse, GetVasAccountCharacterList,
    GetVasAccountCharacterListResult, GetVasTransferTargetRealmList,
    GetVasTransferTargetRealmListResult, VasAccountCharacter, VasGetServiceStatusResponse,
    VasPurchase, VasPurchaseComplete, VasPurchaseStateUpdate, VasTargetRealm,
};
use wow_persistence::{
    BattlePayCharacterRowLikeCpp, BattlePayDeliveryReceiptLikeCpp, PersistenceOutcomeLikeCpp,
};

use super::catalog::BattlePayProductLikeCpp;
use super::constants::*;
use super::flow::{
    BattlePayIdentityLikeCpp, BattlePaySessionLikeCpp, DeliveryOutcomeLikeCpp, PaidOrderLikeCpp,
    begin_confirmation, mark_delivered, receipt_exists,
};
use super::product_kind::{CharacterServiceLikeCpp, ProductKindLikeCpp};
use super::service::{ActivePurchaseLikeCpp, BattlePayServiceLikeCpp};

pub(crate) fn player_guid_like_cpp(realm_id: u32, counter: u64) -> ObjectGuid {
    ObjectGuid::create_player(realm_id as u16, counter as i64)
}

fn clipped(value: &str, max: usize) -> String {
    let mut end = value.len().min(max);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

/// LegionCore `SendVasCharacterList` characters of this realm.
async fn account_characters(
    service: &BattlePayServiceLikeCpp,
    identity: &BattlePayIdentityLikeCpp,
) -> Vec<VasAccountCharacter> {
    let rows = match service
        .characters
        .load_account_characters_like_cpp(identity.account_id)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            warn!(account = identity.account_id, %error, "BattlePay: VAS character list unavailable");
            Vec::new()
        }
    };
    let wow_account_guid =
        ObjectGuid::create_global(HighGuid::WowAccount, 0, i64::from(identity.account_id));
    rows.into_iter()
        .map(|row| VasAccountCharacter {
            wow_account_guid,
            character_guid: player_guid_like_cpp(identity.realm_id, row.guid),
            virtual_realm_address: identity.virtual_realm_address,
            race: row.race,
            class: row.class,
            sex: row.gender,
            level: row.level,
            last_login: row.logout_time,
            unk: 0,
            name: clipped(&row.name, (1 << 6) - 1),
            realm_name: clipped(&identity.realm_name, (1 << 9) - 1),
        })
        .collect()
}

async fn send_character_list<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    identity: &BattlePayIdentityLikeCpp,
    token: u32,
    choice_type: u32,
) {
    let characters = account_characters(service, identity).await;
    info!(
        account = identity.account_id,
        token,
        choice_type,
        characters = characters.len(),
        "BattlePay: VAS character list sent"
    );
    session.send_battle_pay_packet(&GetVasAccountCharacterListResult {
        token,
        result: error::OK,
        choice_type,
        characters,
    });
}

fn send_realm_list<S: BattlePaySessionLikeCpp>(
    session: &S,
    identity: &BattlePayIdentityLikeCpp,
    token: u32,
    choice_type: u32,
) {
    // Only same-realm transfers are delivered (see `transfer.rs`): this realm is
    // the one destination offered.
    session.send_battle_pay_packet(&GetVasTransferTargetRealmListResult {
        token,
        result: error::OK,
        choice_type,
        realms: vec![VasTargetRealm {
            virtual_realm_address: identity.virtual_realm_address,
            cfg_realms_id: identity.realm_id,
            name: clipped(&identity.realm_name, (1 << 9) - 1),
            ..VasTargetRealm::default()
        }],
    });
}

/// `MakePurchase` VAS branch: the store's `PurchaseProduct` of a VAS product is
/// answered with the lists the validation frame waits for.
pub(crate) async fn send_vas_lists_for_purchase<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    identity: &BattlePayIdentityLikeCpp,
    product: &BattlePayProductLikeCpp,
    token: u32,
) {
    let choice_type = u32::from(product.choice_type);
    if matches!(product.kind_like_cpp(), ProductKindLikeCpp::Transfer { .. }) {
        // The client answers a matching realm list with 0x36f8 itself (0x141a4afed).
        send_realm_list(session, identity, token, choice_type);
        return;
    }
    send_character_list(session, service, identity, token, choice_type).await;
}

/// `CMSG_GET_VAS_ACCOUNT_CHARACTER_LIST` 0x36f8 (LegionCore
/// `HandleBattlePayRequestVasCharacterList`).
pub(crate) async fn handle_get_vas_account_character_list<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    request: GetVasAccountCharacterList,
) {
    let identity = session.battle_pay_identity();
    if !service.config.is_available_for_like_cpp(identity.security) {
        return;
    }
    send_character_list(
        session,
        service,
        &identity,
        request.client_token,
        request.choice_type,
    )
    .await;
}

/// `CMSG_GET_VAS_TRANSFER_TARGET_REALM_LIST` 0x36f9.
pub(crate) fn handle_get_vas_transfer_target_realm_list<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    request: GetVasTransferTargetRealmList,
) {
    let identity = session.battle_pay_identity();
    if !service.config.is_available_for_like_cpp(identity.security) {
        return;
    }
    send_realm_list(
        session,
        &identity,
        request.client_token,
        request.choice_type,
    );
}

/// `CMSG_VAS_GET_SERVICE_STATUS` 0x3711 (LegionCore
/// `HandleBattlePayRequestCurrentVasTransferQueues`): transfers are applied at
/// confirmation, so both queues are under an hour.
pub(crate) fn handle_vas_get_service_status<S: BattlePaySessionLikeCpp>(session: &S) {
    session.send_battle_pay_packet(&VasGetServiceStatusResponse {
        transfer_queue: VAS_QUEUE_UNDER_AN_HOUR_LIKE_CPP,
        faction_transfer_queue: VAS_QUEUE_UNDER_AN_HOUR_LIKE_CPP,
    });
}

/// `CMSG_UPDATE_VAS_PURCHASE_STATES` (LegionCore `HandleUpdateVasPurchaseStates`):
/// the open VAS purchase of the account while its web payment is pending, else an
/// empty list. Wallet purchases complete within the confirmation.
pub(crate) fn handle_update_vas_purchase_states<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
) {
    let identity = session.battle_pay_identity();
    let mut response = EnumVasPurchaseStatesResponse::default();
    if let Some(purchase) = service.purchase(identity.account_id).filter(|purchase| {
        purchase.pending_web_external_id().is_some()
            && service
                .catalog
                .product(purchase.product_id)
                .is_some_and(BattlePayProductLikeCpp::is_vas_like_cpp)
    }) {
        response.purchases.push(VasPurchase {
            player_guid: purchase.target_character,
            product_id: purchase.product_id,
            state: VAS_PROGRESS_PAYMENT_PENDING_LIKE_CPP,
            purchase_id: purchase.purchase_id,
            errors: Vec::new(),
        });
    }
    session.send_battle_pay_packet(&response);
}

/// `Enum.VasPurchaseProgress.PaymentPending`.
const VAS_PROGRESS_PAYMENT_PENDING_LIKE_CPP: u32 = 2;

/// STORE_VAS_PURCHASE_ERROR: 0x27f3 with the VAS error codes (client handler
/// `0x141a4a790` keeps the errors for `C_StoreSecure.GetVASErrors`).
pub(crate) fn send_vas_error<S: BattlePaySessionLikeCpp>(
    session: &S,
    identity: &BattlePayIdentityLikeCpp,
    character: ObjectGuid,
    product_id: u32,
    vas_error: u32,
    why: &str,
) {
    info!(
        account = identity.account_id,
        product = product_id,
        vas_error,
        "BattlePay: VAS purchase refused: {why}"
    );
    session.send_battle_pay_packet(&VasPurchaseStateUpdate {
        unk: 0,
        purchase: VasPurchase {
            player_guid: character,
            product_id,
            state: vas_progress::INVALID,
            purchase_id: 0,
            errors: vec![vas_error],
        },
    });
}

/// STORE_VAS_PURCHASE_COMPLETE (LegionCore `BattlePayVasPurchaseComplete`, same
/// field values as `CompleteVasCharacterTransfer`).
pub(crate) fn send_vas_complete<S: BattlePaySessionLikeCpp>(
    session: &S,
    product_id: u32,
    character: ObjectGuid,
    name: &str,
) {
    session.send_battle_pay_packet(&VasPurchaseComplete {
        product_id,
        result: error::OK,
        character_guid: character,
        glue_guid: character,
        unk: 0,
        handled_guid: ObjectGuid::EMPTY,
        character_name: clipped(name, (1 << 6) - 1),
    });
}

/// Checks shared by every VAS service on the chosen character.
pub(crate) fn check_vas_character(
    identity: &BattlePayIdentityLikeCpp,
    row: Option<&BattlePayCharacterRowLikeCpp>,
) -> Result<(), (u32, &'static str)> {
    let Some(row) = row.filter(|row| row.account_id == identity.account_id) else {
        return Err((
            vas_error::INVALID_SOURCE_ACCOUNT,
            "the character is not of this account",
        ));
    };
    if row.level < VAS_MIN_CHARACTER_LEVEL_LIKE_CPP {
        return Err((vas_error::UNDER_MIN_LEVEL_REQ, "character below level 10"));
    }
    if row.at_login_flags & at_login::CHARACTER_BOOST != 0 {
        return Err((vas_error::CHARACTER_HAS_VAS_PENDING, "a boost is pending"));
    }
    Ok(())
}

/// `CMSG_BATTLE_PAY_START_VAS_PURCHASE` 0x36fa (LegionCore
/// `HandleBattlePayStartVasPurchase`, extended from transfers to every VAS
/// service the 54261 store sells).
pub(crate) async fn handle_start_vas_purchase<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    request: BattlePayStartVasPurchase,
) {
    let identity = session.battle_pay_identity();
    if !service.config.is_available_for_like_cpp(identity.security) {
        return;
    }
    let character = request.character_guid;
    let refuse = |vas_error: u32, why: &str| {
        send_vas_error(
            session,
            &identity,
            character,
            request.product_id,
            vas_error,
            why,
        );
    };
    let Some(product) = service
        .catalog
        .product(request.product_id)
        .filter(|product| product.is_vas_like_cpp() && product.is_deliverable_like_cpp())
    else {
        return refuse(vas_error::CHAR_LOCKED, "not a VAS product");
    };
    if service
        .catalog
        .group_for_product(request.product_id)
        .is_none()
    {
        return refuse(vas_error::CHAR_LOCKED, "product is in no group");
    }
    if service
        .purchase(identity.account_id)
        .is_some_and(|purchase| purchase.pending_web_external_id().is_some() && !purchase.lock)
    {
        return refuse(
            vas_error::BATTLEPAY_DELIVERY_PENDING,
            "a web payment is pending",
        );
    }
    let row = match service
        .characters
        .load_character_like_cpp(character.counter() as u64)
        .await
    {
        Ok(row) => row,
        Err(error) => {
            warn!(%error, "BattlePay: VAS character lookup failed");
            return refuse(vas_error::CHAR_LOCKED, "character lookup failed");
        }
    };
    if let Err((vas_error, why)) = check_vas_character(&identity, row.as_ref()) {
        return refuse(vas_error, why);
    }
    let row = row.expect("checked above");
    let mut purchase = ActivePurchaseLikeCpp {
        purchase_id: 0,
        client_token: request.client_token,
        server_token: 0,
        product_id: request.product_id,
        current_price: product.current_price,
        status: purchase_status::LOADING,
        target_character: character,
        lock: false,
        web: None,
        transfer: None,
    };
    match product.kind_like_cpp() {
        ProductKindLikeCpp::Service(kind) => {
            if let Some(vas_error) = kind.already_flagged_error_like_cpp(row.at_login_flags) {
                return refuse(vas_error, "the service is already pending");
            }
        }
        ProductKindLikeCpp::Transfer { .. } => {
            match super::transfer::check_transfer(service, &identity, &request, &row).await {
                Ok(target) => purchase.transfer = Some(target),
                Err((vas_error, why)) => return refuse(vas_error, why),
            }
        }
        _ => return refuse(vas_error::CHAR_LOCKED, "not a character service"),
    }
    info!(
        account = identity.account_id,
        product = request.product_id,
        character = row.guid,
        "BattlePay: VAS purchase accepted"
    );
    // LegionCore callback: StartPurchaseResponse, PurchaseUpdate and the wallet
    // confirmation (STORE_CONFIRM_PURCHASE closes the VAS frame) or the checkout.
    begin_confirmation(session, service, &identity, purchase).await;
}

/// LegionCore `CharacterService::{SetRename,ChangeFaction,ChangeRace,Customize}`:
/// the at-login flag of the order's character, committed with the receipt.
pub(crate) async fn deliver_service<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    order: &PaidOrderLikeCpp,
    kind: CharacterServiceLikeCpp,
) -> DeliveryOutcomeLikeCpp {
    let identity = session.battle_pay_identity();
    if order.character_guid == 0 {
        return DeliveryOutcomeLikeCpp::Deferred("the order names no character");
    }
    let character = player_guid_like_cpp(identity.realm_id, order.character_guid);
    let already_delivered = match receipt_exists(service, order).await {
        Ok(exists) => exists,
        Err(outcome) => return outcome,
    };
    let flag = kind.at_login_flag_like_cpp();
    if !already_delivered {
        let receipt = BattlePayDeliveryReceiptLikeCpp {
            external_id: order.external_id.clone(),
            account_id: identity.account_id,
            character_guid: order.character_guid,
            product_id: order.product_id,
        };
        match service
            .characters
            .persist_service_delivery_like_cpp(receipt, flag)
            .await
        {
            PersistenceOutcomeLikeCpp::Applied { .. } => {}
            outcome => {
                warn!(order = %order.external_id, ?outcome, "BattlePay: service delivery did not commit");
                return DeliveryOutcomeLikeCpp::Deferred("service delivery did not commit");
            }
        }
        if identity
            .player
            .is_some_and(|player| player.guid == character)
        {
            let flags = session.battle_pay_player_at_login_flags();
            session.battle_pay_set_player_at_login_flags(flags | flag);
        }
    }
    mark_delivered(service, order).await;
    let name = service
        .characters
        .load_character_like_cpp(order.character_guid)
        .await
        .ok()
        .flatten()
        .map(|row| row.name)
        .unwrap_or_default();
    send_vas_complete(session, order.product_id, character, &name);
    info!(
        account = identity.account_id,
        product = order.product_id,
        character = order.character_guid,
        ?kind,
        order = %order.external_id,
        already_delivered,
        "BattlePay: character service delivered"
    );
    if already_delivered {
        DeliveryOutcomeLikeCpp::AlreadyDelivered
    } else {
        DeliveryOutcomeLikeCpp::Delivered
    }
}

/// LegionCore `CharacterService::RestoreDeletedCharacter` adapted to TC 3.4.3:
/// the account's character-undelete cooldown is cleared, so the client's
/// "restore character" works again immediately.
pub(crate) async fn deliver_undelete<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    order: &PaidOrderLikeCpp,
) -> DeliveryOutcomeLikeCpp {
    let identity = session.battle_pay_identity();
    match service
        .distributions
        .grant_undelete_like_cpp(
            identity.battlenet_account_id,
            order.external_id.clone(),
            order.web_order_id.clone(),
        )
        .await
    {
        PersistenceOutcomeLikeCpp::Applied { .. } => {
            info!(
                account = identity.account_id,
                order = %order.external_id,
                "BattlePay: character undelete cooldown cleared"
            );
            DeliveryOutcomeLikeCpp::Delivered
        }
        outcome => {
            warn!(order = %order.external_id, ?outcome, "BattlePay: undelete service did not commit");
            DeliveryOutcomeLikeCpp::Deferred("undelete service did not commit")
        }
    }
}
