//! BattlePay web-checkout handlers (LegionCore `HandleGenerateSSOToken`,
//! `HandleBattlePayPurchaseSubmitted`, `HandleBattlePayCancelOpenCheckout` and
//! their callbacks, `BattlePayHandler.cpp:901-1117`), moved out of `flow.rs`.

use rand::Rng;
use tracing::{debug, info, warn};
use wow_core::ObjectGuidGenerator;
use wow_packet::packets::battlepay::{
    BattlePayCancelOpenCheckout, BattlePayOpenCheckout, BattlePayPurchaseSubmitted,
    GenerateSsoTokenResponse,
};
use wow_persistence::{
    BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP, BATTLE_PAY_PURCHASE_STATUS_FAILED_LIKE_CPP,
    BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP, BattlePaySsoTokenIssueLikeCpp,
};

use super::constants::*;
use super::flow::{
    BattlePaySessionLikeCpp, DeliveryOutcomeLikeCpp, PaidOrderLikeCpp, deliver_order_like_cpp,
    send_ack_failed, send_purchase_update,
};
use super::service::{ActivePurchaseLikeCpp, BattlePayServiceLikeCpp};

pub(crate) fn finish_web_purchase<S: BattlePaySessionLikeCpp>(
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
    let order = PaidOrderLikeCpp::from_row(&row, &request.global_order_id, purchase.purchase_id);
    // LegionCore `HandleBattlePayPurchaseSubmittedCallback`: `Status = Finish;
    // SendPurchaseUpdate(Ok); ProcessDelivery(...)`. The purchase update must precede the
    // delivery packets (see the note in `flow.rs` about the client's `JustOrderedProduct`).
    finish_web_purchase(
        session,
        service,
        identity.account_id,
        &mut purchase,
        error::OK,
    );
    let outcome = deliver_order_like_cpp(session, service, item_guid_generator, &order).await;
    match outcome {
        DeliveryOutcomeLikeCpp::Delivered | DeliveryOutcomeLikeCpp::AlreadyDelivered => {}
        DeliveryOutcomeLikeCpp::Deferred(why) => {
            warn!(order = %row.external_id, "BattlePay: paid order deferred: {why}");
        }
        DeliveryOutcomeLikeCpp::Quarantined => {}
    }
}

/// LegionCore callback path: `WebCheckoutPending = false; Lock = true;
/// SendAckFailed(PaymentFailed)`.
pub(crate) fn fail_web_purchase<S: BattlePaySessionLikeCpp>(
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
        // The web confirmed the payment right before the window closed. Purchase update
        // first, then delivery (LegionCore order; see `flow.rs`).
        let order = PaidOrderLikeCpp::from_row(row, "", purchase.purchase_id);
        finish_web_purchase(
            session,
            service,
            identity.account_id,
            &mut purchase,
            error::OK,
        );
        if let DeliveryOutcomeLikeCpp::Deferred(why) =
            deliver_order_like_cpp(session, service, item_guid_generator, &order).await
        {
            warn!(order = %row.external_id, "BattlePay: paid order deferred: {why}");
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
