//! Token-wallet purchase state machine.

use wow_constants::ServerOpcodes;
use wow_core::ObjectGuid;
use wow_packet::packets::battlepay::{
    BattlePayAckFailedResponse, BattlePayConfirmPurchaseResponse, BattlePayStartPurchase,
};
use wow_persistence::{
    BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP, BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP,
    BattlePayTokenChargeOutcomeLikeCpp,
};

use super::fakes::*;
use crate::battle_pay::constants::{BattlePayConfigLikeCpp, error, purchase_status};
use crate::battle_pay::flow::*;

const START_RESPONSE: u16 = ServerOpcodes::BattlePayStartPurchaseResponse as u16;
const PURCHASE_UPDATE: u16 = ServerOpcodes::BattlePayPurchaseUpdate as u16;
const CONFIRM: u16 = ServerOpcodes::BattlePayConfirmPurchase as u16;
const DELIVERY_STARTED: u16 = ServerOpcodes::BattlePayDeliveryStarted as u16;
const DELIVERY_ENDED: u16 = ServerOpcodes::BattlePayDeliveryEnded as u16;
const MOUNT_DELIVERED: u16 = ServerOpcodes::BattlePayMountDelivered as u16;
const PRODUCT_LIST: u16 = ServerOpcodes::BattlePayGetProductListResponse as u16;

pub(super) fn start_request(product_id: u32) -> BattlePayStartPurchase {
    BattlePayStartPurchase {
        client_token: 55,
        product_id,
        target_character: ObjectGuid::EMPTY,
        wow_system: String::new(),
        public_key: String::new(),
        unk_string: String::new(),
    }
}

/// `(PurchaseID, PurchaseResult, ClientToken)` of a 0x2783.
pub(super) fn start_response(bytes: &[u8]) -> (u64, u32, u32) {
    assert_eq!(opcode_of(bytes), START_RESPONSE);
    let mut pkt = payload(bytes);
    (
        pkt.read_uint64().unwrap(),
        pkt.read_uint32().unwrap(),
        pkt.read_uint32().unwrap(),
    )
}

/// `(PurchaseID, Status, ResultCode, ProductID)` of the first purchase of a 0x2786.
pub(super) fn purchase_update(bytes: &[u8]) -> (u64, u32, u32, u32) {
    assert_eq!(opcode_of(bytes), PURCHASE_UPDATE);
    let mut pkt = payload(bytes);
    assert_eq!(pkt.read_uint32().unwrap(), 1);
    (
        pkt.read_uint64().unwrap(),
        pkt.read_uint32().unwrap(),
        pkt.read_uint32().unwrap(),
        pkt.read_uint32().unwrap(),
    )
}

/// Start a wallet purchase and return `(PurchaseID, ServerToken)` from 0x2787.
async fn started(h: &Harness, session: &mut FakeSession, product_id: u32) -> (u64, u32) {
    handle_start_purchase(session, &h.service, start_request(product_id)).await;
    let sent = session.take_sent();
    assert_eq!(opcodes(&sent), [START_RESPONSE, PURCHASE_UPDATE, CONFIRM]);
    let (purchase_id, result, client_token) = start_response(&sent[0]);
    assert_eq!((result, client_token), (error::OK, 55));
    assert_ne!(purchase_id, 0);
    let (_, status, _, product) = purchase_update(&sent[1]);
    assert_eq!((status, product), (purchase_status::LOADING, product_id));
    let mut confirm = payload(&sent[2]);
    assert_eq!(confirm.read_uint64().unwrap(), purchase_id);
    (purchase_id, confirm.read_uint32().unwrap())
}

fn confirm_request(server_token: u32, price: u64) -> BattlePayConfirmPurchaseResponse {
    BattlePayConfirmPurchaseResponse {
        confirm_purchase: true,
        server_token,
        client_current_price_fixed_point: price,
    }
}

#[tokio::test]
async fn wallet_purchase_charges_once_and_delivers_the_mount() {
    let h = harness(token_config(), FakeAccount::with_balance(100));
    let mut session = FakeSession::in_world();
    let (purchase_id, token) = started(&h, &mut session, MOUNT_PRODUCT).await;
    assert_eq!(
        h.account.balance(),
        100,
        "nothing is charged before the confirmation"
    );

    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm_request(token, 150_000),
    )
    .await;

    assert_eq!(h.account.balance(), 85);
    let order = h.account.only_order();
    assert_eq!(order.status, BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP);
    assert_eq!(order.insert.currency, "TOK");
    assert_eq!(order.insert.price, "15");
    assert_eq!(order.insert.external_id.len(), 32);
    assert_eq!(
        h.account.state().token_log,
        vec![(ACCOUNT, -15, MOUNT_PRODUCT)]
    );
    assert!(
        h.delivery
            .state()
            .receipts
            .contains(&order.insert.external_id)
    );
    assert_eq!(session.grants, vec![vec![(MOUNT_ITEM, 1)]]);
    let sent = session.take_sent();
    assert_eq!(
        opcodes(&sent),
        [
            DELIVERY_STARTED,
            MOUNT_DELIVERED,
            DELIVERY_ENDED,
            PURCHASE_UPDATE
        ]
    );
    let mut started = payload(&sent[0]);
    assert_eq!(started.read_uint64().unwrap(), purchase_id);
    assert_eq!(
        purchase_update(&sent[3]),
        (
            purchase_id,
            purchase_status::FINISH,
            error::OK,
            MOUNT_PRODUCT
        )
    );
}

#[tokio::test]
async fn non_mount_delivery_sends_no_mount_notification() {
    let h = harness(token_config(), FakeAccount::with_balance(8));
    let mut session = FakeSession::in_world();
    let (_, token) = started(&h, &mut session, BAG_PRODUCT).await;
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm_request(token, 80_000),
    )
    .await;
    assert_eq!(h.account.balance(), 0);
    assert_eq!(
        opcodes(&session.take_sent()),
        [DELIVERY_STARTED, DELIVERY_ENDED, PURCHASE_UPDATE]
    );
}

#[tokio::test]
async fn insufficient_balance_is_refused_at_start_without_a_confirmation() {
    let h = harness(token_config(), FakeAccount::with_balance(14));
    let mut session = FakeSession::in_world();
    handle_start_purchase(&session, &h.service, start_request(MOUNT_PRODUCT)).await;
    let sent = session.take_sent();
    assert_eq!(opcodes(&sent), [START_RESPONSE]);
    assert_eq!(
        start_response(&sent[0]),
        (0, error::INSUFFICIENT_BALANCE, 55)
    );
    // A forged confirmation for the refused order charges nothing.
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm_request(0, 150_000),
    )
    .await;
    assert_eq!(h.account.balance(), 14);
    assert!(h.account.state().orders.is_empty());
}

#[tokio::test]
async fn balance_spent_elsewhere_between_start_and_confirm_is_not_overdrawn() {
    let h = harness(token_config(), FakeAccount::with_balance(20));
    let mut session = FakeSession::in_world();
    let (_, token) = started(&h, &mut session, MOUNT_PRODUCT).await;
    h.account.state().balances.insert((ACCOUNT, 1), 10);
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm_request(token, 150_000),
    )
    .await;
    assert_eq!(h.account.balance(), 10);
    assert!(session.grants.is_empty());
    let sent = session.take_sent();
    assert_eq!(purchase_update(&sent[0]).2, error::INSUFFICIENT_BALANCE);
}

#[tokio::test]
async fn wrong_server_token_or_price_is_denied_before_charging() {
    let h = harness(token_config(), FakeAccount::with_balance(100));
    let mut session = FakeSession::in_world();
    let (_, token) = started(&h, &mut session, MOUNT_PRODUCT).await;
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm_request(token.wrapping_add(1), 150_000),
    )
    .await;
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm_request(token, 1),
    )
    .await;
    let mut declined = confirm_request(token, 150_000);
    declined.confirm_purchase = false;
    handle_confirm_purchase_response(&mut session, &h.service, &h.generator, declined).await;
    let sent = session.take_sent();
    assert_eq!(sent.len(), 3);
    assert!(
        sent.iter()
            .all(|bytes| purchase_update(bytes).2 == error::PURCHASE_DENIED)
    );
    assert_eq!(h.account.balance(), 100);
    assert!(session.grants.is_empty());
    // The correct confirmation still goes through afterwards.
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm_request(token, 150_000),
    )
    .await;
    assert_eq!(h.account.balance(), 85);
}

#[tokio::test]
async fn full_bags_refuse_before_any_charge() {
    let h = harness(token_config(), FakeAccount::with_balance(100));
    let mut session = FakeSession::in_world();
    session.bag_space = false;
    handle_start_purchase(&session, &h.service, start_request(MOUNT_PRODUCT)).await;
    assert_eq!(
        start_response(&session.take_sent()[0]).1,
        error::PURCHASE_DENIED
    );

    // Bags filled between StartPurchase and the confirmation.
    session.bag_space = true;
    let (_, token) = started(&h, &mut session, MOUNT_PRODUCT).await;
    session.bag_space = false;
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm_request(token, 150_000),
    )
    .await;
    assert_eq!(
        purchase_update(&session.take_sent()[0]).2,
        error::PURCHASE_DENIED
    );
    assert_eq!(h.account.balance(), 100);
    assert!(h.account.state().orders.is_empty());
}

#[tokio::test]
async fn owned_item_and_glue_session_are_refused() {
    let h = harness(token_config(), FakeAccount::with_balance(100));
    let mut session = FakeSession::in_world();
    session.owned.insert(MOUNT_ITEM);
    handle_start_purchase(&session, &h.service, start_request(MOUNT_PRODUCT)).await;
    assert_eq!(
        start_response(&session.take_sent()[0]).1,
        error::PURCHASE_DENIED
    );

    let mut glue = FakeSession::in_world();
    glue.identity.player = None;
    handle_start_purchase(&glue, &h.service, start_request(MOUNT_PRODUCT)).await;
    assert_eq!(
        start_response(&glue.take_sent()[0]).1,
        error::PURCHASE_DENIED
    );
}

#[tokio::test]
async fn double_confirmation_charges_and_delivers_once() {
    let h = harness(token_config(), FakeAccount::with_balance(100));
    let mut session = FakeSession::in_world();
    let (_, token) = started(&h, &mut session, MOUNT_PRODUCT).await;
    for _ in 0..2 {
        handle_confirm_purchase_response(
            &mut session,
            &h.service,
            &h.generator,
            confirm_request(token, 150_000),
        )
        .await;
    }
    assert_eq!(h.account.balance(), 85);
    assert_eq!(session.grants.len(), 1);
    assert_eq!(h.delivery.state().commits, 1);
    let last = session.take_sent().pop().unwrap();
    assert_eq!(purchase_update(&last).2, error::PURCHASE_DENIED);
}

#[tokio::test]
async fn ack_failed_releases_the_lock_only_for_the_matching_token() {
    let h = harness(token_config(), FakeAccount::with_balance(100));
    let session = FakeSession::in_world();
    let mut purchase_session = session;
    let (_, token) = started(&h, &mut purchase_session, MOUNT_PRODUCT).await;
    h.service
        .update_purchase(ACCOUNT, |purchase| purchase.lock = true);
    handle_ack_failed_response(
        &purchase_session,
        &h.service,
        BattlePayAckFailedResponse {
            server_token: token.wrapping_add(1),
        },
    );
    assert!(h.service.purchase(ACCOUNT).unwrap().lock);
    handle_ack_failed_response(
        &purchase_session,
        &h.service,
        BattlePayAckFailedResponse {
            server_token: token,
        },
    );
    let purchase = h.service.purchase(ACCOUNT).unwrap();
    assert!(!purchase.lock);
    assert_eq!(purchase.status, purchase_status::LOADING);
}

#[tokio::test]
async fn crash_between_charge_and_delivery_is_recovered_exactly_once() {
    let h = harness(token_config(), FakeAccount::with_balance(100));
    let mut session = FakeSession::in_world();
    let (_, token) = started(&h, &mut session, MOUNT_PRODUCT).await;
    h.delivery.state().fail_next_commit = true;
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm_request(token, 150_000),
    )
    .await;
    assert!(
        session.quarantined.is_some(),
        "divergent runtime inventory is quarantined"
    );
    let order = h.account.only_order();
    assert_eq!(order.status, BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP);
    assert!(h.delivery.state().receipts.is_empty());

    // Relog: the next store refresh delivers the paid order once.
    let mut relogged = FakeSession::in_world();
    for _ in 0..2 {
        handle_get_product_list(&mut relogged, &h.service, &h.generator).await;
    }
    assert_eq!(relogged.grants, vec![vec![(MOUNT_ITEM, 1)]]);
    assert_eq!(h.delivery.state().commits, 1);
    assert_eq!(
        h.account.only_order().status,
        BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP
    );
    assert_eq!(h.account.balance(), 85, "recovery never charges again");
    let sent = relogged.take_sent();
    assert_eq!(opcode_of(&sent[0]), PRODUCT_LIST);
    assert!(opcodes(&sent).contains(&DELIVERY_ENDED));
}

#[tokio::test]
async fn delivered_but_unmarked_order_is_only_remarked() {
    let h = harness(token_config(), FakeAccount::with_balance(100));
    let mut session = FakeSession::in_world();
    let (_, token) = started(&h, &mut session, MOUNT_PRODUCT).await;
    h.account.state().mark_delivered_fails = true;
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm_request(token, 150_000),
    )
    .await;
    assert_eq!(
        h.account.only_order().status,
        BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP
    );
    h.account.state().mark_delivered_fails = false;

    let mut relogged = FakeSession::in_world();
    handle_get_product_list(&mut relogged, &h.service, &h.generator).await;
    assert!(
        relogged.grants.is_empty(),
        "the receipt prevents a second grant"
    );
    assert_eq!(
        h.account.only_order().status,
        BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP
    );
    assert_eq!(opcodes(&relogged.take_sent()), [PRODUCT_LIST]);
}

#[tokio::test]
async fn unknown_charge_outcome_is_reported_and_left_to_recovery() {
    let h = harness(token_config(), FakeAccount::with_balance(100));
    let mut session = FakeSession::in_world();
    let (_, token) = started(&h, &mut session, MOUNT_PRODUCT).await;
    h.account.state().charge_override = Some(BattlePayTokenChargeOutcomeLikeCpp::Unknown {
        reason: "connection lost".into(),
    });
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm_request(token, 150_000),
    )
    .await;
    assert!(session.grants.is_empty());
    assert_eq!(purchase_update(&session.take_sent()[0]).2, error::OTHER);
}

#[tokio::test]
async fn disabled_shop_answers_locked_and_ignores_purchases() {
    let h = harness(
        BattlePayConfigLikeCpp::default(),
        FakeAccount::with_balance(100),
    );
    let mut session = FakeSession::in_world();
    handle_get_product_list(&mut session, &h.service, &h.generator).await;
    let sent = session.take_sent();
    assert_eq!(opcodes(&sent), [PRODUCT_LIST]);
    assert_eq!(payload(&sent[0]).read_uint32().unwrap(), 1);
    handle_start_purchase(&session, &h.service, start_request(MOUNT_PRODUCT)).await;
    assert!(session.take_sent().is_empty());
}

#[tokio::test]
async fn moderators_see_the_shop_without_the_player_feature_switch() {
    let config = BattlePayConfigLikeCpp {
        enabled: true,
        ..BattlePayConfigLikeCpp::default()
    };
    let h = harness(config, FakeAccount::with_balance(0));
    let mut session = FakeSession::in_world();
    session.identity.security = 1;
    handle_get_product_list(&mut session, &h.service, &h.generator).await;
    let sent = session.take_sent();
    assert_eq!(payload(&sent[0]).read_uint32().unwrap(), 0);
}
