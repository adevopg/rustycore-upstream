//! Web checkout (real money) and SSO token flow.

use wow_constants::ServerOpcodes;
use wow_packet::packets::battlepay::{
    BattlePayCancelOpenCheckout, BattlePayOpenCheckout, BattlePayPurchaseSubmitted,
};
use wow_persistence::{
    BATTLE_PAY_PURCHASE_STATUS_CREATED_LIKE_CPP, BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP,
    BATTLE_PAY_PURCHASE_STATUS_FAILED_LIKE_CPP, BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP,
};

use super::fakes::*;
use super::flow_token::{purchase_update, start_request, start_response};
use crate::battle_pay::constants::{BattlePayConfigLikeCpp, error, purchase_status};
use crate::battle_pay::flow::*;

const START_RESPONSE: u16 = ServerOpcodes::BattlePayStartPurchaseResponse as u16;
const PURCHASE_UPDATE: u16 = ServerOpcodes::BattlePayPurchaseUpdate as u16;
const START_CHECKOUT: u16 = ServerOpcodes::BattlePayStartCheckout as u16;
const SSO_RESPONSE: u16 = ServerOpcodes::GenerateSsoTokenResponse as u16;
const DELIVERY_ENDED: u16 = ServerOpcodes::BattlePayDeliveryEnded as u16;
const ACK_FAILED: u16 = ServerOpcodes::BattlePayAckFailed as u16;

/// `(ProductID, RegionID, GameAccountID, Signature, ExternalTransactionID)` of 0x2824.
fn start_checkout(bytes: &[u8]) -> (u32, u32, u64, String, String) {
    assert_eq!(opcode_of(bytes), START_CHECKOUT);
    let mut pkt = payload(bytes);
    let product = pkt.read_uint32().unwrap();
    let region = pkt.read_uint32().unwrap();
    let account = pkt.read_uint64().unwrap();
    let signature_len = pkt.read_bits(6).unwrap() as usize;
    let external_len = pkt.read_bits(7).unwrap() as usize;
    assert!(!pkt.read_bit().unwrap(), "not a subscription");
    pkt.reset_bits();
    let signature = String::from_utf8(pkt.read_bytes(signature_len).unwrap()).unwrap();
    let external = String::from_utf8(pkt.read_bytes(external_len).unwrap()).unwrap();
    (product, region, account, signature, external)
}

/// `(Kind, Result, Token)` of 0x281e.
fn sso_response(bytes: &[u8]) -> (u32, u32, String) {
    assert_eq!(opcode_of(bytes), SSO_RESPONSE);
    let mut pkt = payload(bytes);
    let kind = pkt.read_uint32().unwrap();
    let result = pkt.read_uint32().unwrap();
    pkt.read_uint64().unwrap();
    pkt.read_uint64().unwrap();
    let len = pkt.read_bits(7).unwrap() as usize;
    pkt.reset_bits();
    (
        kind,
        result,
        String::from_utf8(pkt.read_bytes(len).unwrap()).unwrap(),
    )
}

/// Start a web checkout and return the order's external id.
async fn checkout(h: &Harness, session: &mut FakeSession) -> String {
    handle_start_purchase(session, &h.service, start_request(MOUNT_PRODUCT)).await;
    let sent = session.take_sent();
    assert_eq!(
        opcodes(&sent),
        [START_RESPONSE, PURCHASE_UPDATE, START_CHECKOUT]
    );
    assert_eq!(start_response(&sent[0]).1, error::OK);
    let (product, region, account, signature, external) = start_checkout(&sent[2]);
    assert_eq!(
        (product, region, account),
        (MOUNT_PRODUCT, 2, u64::from(ACCOUNT))
    );
    assert_eq!(signature.len(), 32);
    assert_eq!(external.len(), 32);
    let order = h.account.order(&external).expect("order recorded");
    assert_eq!(order.status, BATTLE_PAY_PURCHASE_STATUS_CREATED_LIKE_CPP);
    assert_eq!(order.insert.signature, signature);
    assert_eq!(order.insert.price, "15.00");
    assert_eq!(order.insert.currency, "EUR");
    assert_eq!(order.insert.ip, "203.0.113.5");
    external
}

fn submitted(external: &str) -> BattlePayPurchaseSubmitted {
    BattlePayPurchaseSubmitted {
        global_order_id: "SUMUP-1".into(),
        external_transaction_id: external.to_owned(),
        unk_bit: false,
    }
}

#[tokio::test]
async fn web_checkout_never_touches_the_wallet_and_needs_no_balance() {
    let h = harness(web_config(), FakeAccount::with_balance(0));
    let mut session = FakeSession::in_world();
    checkout(&h, &mut session).await;
    assert_eq!(h.account.balance(), 0);
    assert!(h.account.state().token_log.is_empty());
}

#[tokio::test]
async fn sso_token_is_issued_only_for_a_pending_checkout_with_the_browser_enabled() {
    let h = harness(web_config(), FakeAccount::with_balance(0));
    let mut session = FakeSession::in_world();
    handle_open_checkout(
        &session,
        &h.service,
        BattlePayOpenCheckout { request_id: 9 },
    )
    .await;
    assert_eq!(sso_response(&session.take_sent()[0]), (9, 1, String::new()));

    checkout(&h, &mut session).await;
    handle_open_checkout(
        &session,
        &h.service,
        BattlePayOpenCheckout { request_id: 10 },
    )
    .await;
    let (kind, result, token) = sso_response(&session.take_sent()[0]);
    assert_eq!((kind, result), (10, 0));
    assert_eq!(token.len(), 64);
    assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let issued = h.account.state().sso_tokens.clone();
    assert_eq!(issued.len(), 1);
    assert_eq!(issued[0].battlenet_account_id, 70);
    assert_eq!(issued[0].account_id, ACCOUNT);
    assert_eq!(issued[0].character_guid, 42);
    assert_eq!(issued[0].lifetime_secs, 3600);
}

#[tokio::test]
async fn sso_token_is_refused_with_the_browser_disabled_or_in_token_mode() {
    let config = BattlePayConfigLikeCpp {
        browser_enabled: false,
        ..web_config()
    };
    let h = harness(config, FakeAccount::with_balance(0));
    let mut session = FakeSession::in_world();
    checkout(&h, &mut session).await;
    handle_open_checkout(
        &session,
        &h.service,
        BattlePayOpenCheckout { request_id: 3 },
    )
    .await;
    assert_eq!(sso_response(&session.take_sent()[0]).1, 1);
    assert!(h.account.state().sso_tokens.is_empty());

    let h = harness(token_config(), FakeAccount::with_balance(100));
    let session = FakeSession::in_world();
    handle_open_checkout(
        &session,
        &h.service,
        BattlePayOpenCheckout { request_id: 4 },
    )
    .await;
    assert_eq!(sso_response(&session.take_sent()[0]), (4, 1, String::new()));
}

#[tokio::test]
async fn submitted_before_paid_keeps_the_order_pending_then_paid_delivers_once() {
    let h = harness(web_config(), FakeAccount::with_balance(0));
    let mut session = FakeSession::in_world();
    let external = checkout(&h, &mut session).await;

    handle_purchase_submitted(&mut session, &h.service, &h.generator, submitted(&external)).await;
    assert!(session.grants.is_empty());
    assert!(session.take_sent().is_empty());
    assert_eq!(
        h.service
            .purchase(ACCOUNT)
            .unwrap()
            .pending_web_external_id(),
        Some(external.as_str())
    );

    // The web tier (bnet-shop) confirms the payment.
    h.account.set_status(
        &external,
        BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP,
        "sumup:1",
    );
    handle_purchase_submitted(&mut session, &h.service, &h.generator, submitted(&external)).await;
    assert_eq!(session.grants, vec![vec![(MOUNT_ITEM, 1)]]);
    let order = h.account.order(&external).unwrap();
    assert_eq!(order.status, BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP);
    assert_eq!(order.web_order_id, "SUMUP-1");
    let sent = session.take_sent();
    assert!(opcodes(&sent).contains(&DELIVERY_ENDED));
    let last = purchase_update(sent.last().unwrap());
    assert_eq!((last.1, last.2), (purchase_status::FINISH, error::OK));

    // Replays: a second notification and a store refresh deliver nothing more.
    handle_purchase_submitted(&mut session, &h.service, &h.generator, submitted(&external)).await;
    handle_get_product_list(&mut session, &h.service, &h.generator).await;
    assert_eq!(session.grants.len(), 1);
    assert_eq!(h.delivery.state().commits, 1);
}

#[tokio::test]
async fn paid_order_whose_notification_was_lost_is_delivered_on_store_refresh() {
    let h = harness(web_config(), FakeAccount::with_balance(0));
    let mut session = FakeSession::in_world();
    let external = checkout(&h, &mut session).await;
    h.account.set_status(
        &external,
        BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP,
        "sumup:1",
    );

    handle_get_product_list(&mut session, &h.service, &h.generator).await;
    assert_eq!(session.grants.len(), 1);
    assert_eq!(
        h.account.order(&external).unwrap().status,
        BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP
    );
    assert!(
        h.service
            .purchase(ACCOUNT)
            .unwrap()
            .pending_web_external_id()
            .is_none()
    );
    let last = purchase_update(session.take_sent().last().unwrap());
    assert_eq!(last.2, error::OK);
}

#[tokio::test]
async fn orders_of_another_realm_or_account_are_not_delivered_here() {
    let h = harness(web_config(), FakeAccount::with_balance(0));
    let mut session = FakeSession::in_world();
    let external = checkout(&h, &mut session).await;
    {
        let mut state = h.account.state();
        state.orders[0].insert.realm_id = REALM + 1;
        state.orders[0].status = BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP;
    }
    handle_get_product_list(&mut session, &h.service, &h.generator).await;
    assert!(session.grants.is_empty());

    let mut stranger = FakeSession::in_world();
    stranger.identity.account_id = ACCOUNT + 1;
    handle_purchase_submitted(
        &mut stranger,
        &h.service,
        &h.generator,
        submitted(&external),
    )
    .await;
    assert!(stranger.grants.is_empty());
}

#[tokio::test]
async fn failed_payment_raises_an_acknowledged_error() {
    let h = harness(web_config(), FakeAccount::with_balance(0));
    let mut session = FakeSession::in_world();
    let external = checkout(&h, &mut session).await;
    h.account.set_status(
        &external,
        BATTLE_PAY_PURCHASE_STATUS_FAILED_LIKE_CPP,
        "sumup:1;failed",
    );
    handle_purchase_submitted(&mut session, &h.service, &h.generator, submitted(&external)).await;
    let sent = session.take_sent();
    assert_eq!(opcodes(&sent), [ACK_FAILED]);
    let mut ack = payload(&sent[0]);
    ack.read_uint64().unwrap();
    ack.read_uint32().unwrap();
    ack.read_uint32().unwrap();
    assert_eq!(ack.read_uint32().unwrap(), error::PAYMENT_FAILED);
    assert!(session.grants.is_empty());
}

#[tokio::test]
async fn cancelled_checkout_fails_the_unpaid_order() {
    let h = harness(web_config(), FakeAccount::with_balance(0));
    let mut session = FakeSession::in_world();
    let external = checkout(&h, &mut session).await;
    let cancel = || BattlePayCancelOpenCheckout {
        external_transaction_id: external.clone(),
        unk_bit: false,
    };
    handle_cancel_open_checkout(&mut session, &h.service, &h.generator, cancel()).await;
    assert_eq!(
        h.account.order(&external).unwrap().status,
        BATTLE_PAY_PURCHASE_STATUS_FAILED_LIKE_CPP
    );
    assert_eq!(purchase_update(&session.take_sent()[0]).2, error::OK);
    // The client repeats the cancel on every later close: ignored.
    handle_cancel_open_checkout(&mut session, &h.service, &h.generator, cancel()).await;
    assert!(session.take_sent().is_empty());
}

#[tokio::test]
async fn cancel_after_a_rejected_payment_reports_payment_failed() {
    let h = harness(web_config(), FakeAccount::with_balance(0));
    let mut session = FakeSession::in_world();
    let external = checkout(&h, &mut session).await;
    h.account.set_status(
        &external,
        BATTLE_PAY_PURCHASE_STATUS_CREATED_LIKE_CPP,
        "sumup:9;failed",
    );
    handle_cancel_open_checkout(
        &mut session,
        &h.service,
        &h.generator,
        BattlePayCancelOpenCheckout {
            external_transaction_id: external.clone(),
            unk_bit: false,
        },
    )
    .await;
    assert_eq!(
        purchase_update(&session.take_sent()[0]).2,
        error::PAYMENT_FAILED
    );
}

#[tokio::test]
async fn cancel_racing_a_confirmed_payment_delivers_instead() {
    let h = harness(web_config(), FakeAccount::with_balance(0));
    let mut session = FakeSession::in_world();
    let external = checkout(&h, &mut session).await;
    h.account.set_status(
        &external,
        BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP,
        "sumup:1",
    );
    handle_cancel_open_checkout(
        &mut session,
        &h.service,
        &h.generator,
        BattlePayCancelOpenCheckout {
            external_transaction_id: external.clone(),
            unk_bit: false,
        },
    )
    .await;
    assert_eq!(session.grants.len(), 1);
    assert_eq!(
        h.account.order(&external).unwrap().status,
        BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP
    );
}

#[tokio::test]
async fn purchase_and_vas_lists_are_empty() {
    let session = FakeSession::in_world();
    handle_get_purchase_list(&session);
    handle_update_vas_purchase_states(&session);
    let sent = session.take_sent();
    assert_eq!(
        opcodes(&sent),
        [
            ServerOpcodes::BattlePayGetPurchaseListResponse as u16,
            ServerOpcodes::EnumVasPurchaseStatesResponse as u16
        ]
    );
    let mut list = payload(&sent[0]);
    assert_eq!(
        (list.read_uint32().unwrap(), list.read_uint32().unwrap()),
        (0, 0)
    );
}
