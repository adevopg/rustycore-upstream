//! Character boost: distribution from a paid order, assignment from character
//! select and the boost applied at the next login.

use wow_constants::ServerOpcodes;
use wow_core::ObjectGuid;
use wow_packet::packets::battlepay::{
    BattlePayConfirmPurchaseResponse, BattlePayDistributionAssignToTarget, BattlePayStartPurchase,
    CharacterUpgradeManualUnrevokeRequest,
};
use wow_persistence::{
    BATTLE_PAY_DISTRIBUTION_STATUS_ASSIGNED_LIKE_CPP,
    BATTLE_PAY_DISTRIBUTION_STATUS_AVAILABLE_LIKE_CPP,
    BATTLE_PAY_DISTRIBUTION_STATUS_FINISHED_LIKE_CPP,
    BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP,
};

use super::fakes::*;
use super::fakes_services::*;
use crate::battle_pay::boost::{
    apply_pending_boost, handle_character_upgrade_manual_unrevoke_request,
    handle_distribution_assign_to_target, send_distribution_list,
};
use crate::battle_pay::constants::at_login;
use crate::battle_pay::flow::*;

const DISTRIBUTION_UPDATE: u16 = ServerOpcodes::BattlePayDistributionUpdate as u16;
const DISTRIBUTION_LIST: u16 = ServerOpcodes::BattlePayGetDistributionListResponse as u16;
const PURCHASE_UPDATE: u16 = ServerOpcodes::BattlePayPurchaseUpdate as u16;
const ASSIGN_RESPONSE: u16 = ServerOpcodes::BattlePayStartDistributionAssignToTargetResponse as u16;
const UPGRADE_STARTED: u16 = ServerOpcodes::CharacterUpgradeStarted as u16;
const UPGRADE_COMPLETE: u16 = ServerOpcodes::CharacterUpgradeComplete as u16;
const UNREVOKE_RESULT: u16 = ServerOpcodes::CharacterUpgradeManualUnrevokeResult as u16;

fn boost_harness() -> Harness {
    let h = harness_with_catalog(
        token_config(),
        FakeAccount::with_balance(100),
        services_catalog(),
    );
    seed_characters(&h.characters, &h.distributions);
    h
}

/// Buy the level 70 boost at character select; returns the distribution id.
async fn bought_boost(h: &Harness) -> u64 {
    let mut session = FakeSession::at_glue();
    handle_start_purchase(
        &session,
        &h.service,
        BattlePayStartPurchase {
            client_token: 3,
            product_id: BOOST_70_PRODUCT,
            ..BattlePayStartPurchase::default()
        },
    )
    .await;
    let sent = session.take_sent();
    let mut confirm = payload(sent.last().unwrap());
    confirm.read_uint64().unwrap();
    let server_token = confirm.read_uint32().unwrap();
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        BattlePayConfirmPurchaseResponse {
            confirm_purchase: true,
            server_token,
            client_current_price_fixed_point: 400_000,
        },
    )
    .await;
    let sent = session.take_sent();
    // Purchase update first (LegionCore order): a distribution update received before it
    // would leave the client's `JustOrderedProduct` armed and the store stuck on
    // "Purchase sent" on every reopen.
    assert_eq!(opcodes(&sent), [PURCHASE_UPDATE, DISTRIBUTION_UPDATE]);
    h.distributions.only().id
}

fn assign(distribution_id: u64, character: u64) -> BattlePayDistributionAssignToTarget {
    BattlePayDistributionAssignToTarget {
        client_token: 1,
        distribution_id,
        target_character: ObjectGuid::create_player(REALM as u16, character as i64),
        // Alliance (1 << 24), spec 0: what the Wrath flow sends.
        product_choice: 1 << 24,
    }
}

/// `(DistributionID, Result)` of a 0x2784.
fn assign_result(bytes: &[u8]) -> (u64, u32) {
    assert_eq!(opcode_of(bytes), ASSIGN_RESPONSE);
    let mut pkt = payload(bytes);
    (pkt.read_uint64().unwrap(), pkt.read_uint32().unwrap())
}

#[tokio::test]
async fn a_paid_boost_becomes_an_available_distribution() {
    let h = boost_harness();
    let id = bought_boost(&h).await;
    let row = h.distributions.only();
    assert_eq!(
        row.status,
        BATTLE_PAY_DISTRIBUTION_STATUS_AVAILABLE_LIKE_CPP
    );
    assert_eq!(row.product_id, BOOST_70_PRODUCT);
    assert_ne!(id, 0);
    assert_eq!(
        h.account.only_order().status,
        BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP
    );
    assert_eq!(h.account.balance(), 60);

    let session = FakeSession::at_glue();
    send_distribution_list(&session, &h.service).await;
    let sent = session.take_sent();
    assert_eq!(opcodes(&sent), [DISTRIBUTION_LIST]);
    let mut list = payload(&sent[0]);
    assert_eq!(list.read_uint32().unwrap(), 0);
    assert_eq!(list.read_bits(11).unwrap(), 1);
}

#[tokio::test]
async fn assigning_queues_the_level_and_the_login_boost() {
    let h = boost_harness();
    let id = bought_boost(&h).await;
    let session = FakeSession::at_glue();
    handle_distribution_assign_to_target(&session, &h.service, assign(id, 43)).await;
    let sent = session.take_sent();
    assert_eq!(
        opcodes(&sent),
        [ASSIGN_RESPONSE, UPGRADE_STARTED, DISTRIBUTION_UPDATE]
    );
    assert_eq!(assign_result(&sent[0]), (id, 0));
    let row = h.distributions.only();
    assert_eq!(row.status, BATTLE_PAY_DISTRIBUTION_STATUS_ASSIGNED_LIKE_CPP);
    assert_eq!((row.character_guid, row.realm_id), (43, REALM));
    let character = h.characters.row(43);
    assert_eq!(character.level, 70);
    assert_eq!(character.at_login_flags, at_login::CHARACTER_BOOST);

    // Assigned already: a second assignment is refused.
    handle_distribution_assign_to_target(&session, &h.service, assign(id, 43)).await;
    assert_eq!(assign_result(&session.take_sent()[0]).1, 1);
}

#[tokio::test]
async fn assignment_refusals_leave_the_distribution_available() {
    let h = boost_harness();
    let id = bought_boost(&h).await;
    h.characters.add(character(50, ACCOUNT, 70));
    let mut paladin = character(51, ACCOUNT, 20);
    paladin.class = 2; // no loadout in the test table
    h.characters.add(paladin);
    h.characters.add(character(52, 99, 20));
    let session = FakeSession::in_world(); // guid 42 is in the world
    for (distribution, character) in [(id, 50), (id, 51), (id, 52), (id, 42), (id + 1, 43)] {
        handle_distribution_assign_to_target(&session, &h.service, assign(distribution, character))
            .await;
        let sent = session.take_sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(assign_result(&sent[0]).1, 1);
    }
    assert_eq!(
        h.distributions.only().status,
        BATTLE_PAY_DISTRIBUTION_STATUS_AVAILABLE_LIKE_CPP
    );
}

fn boosted_session() -> FakeSession {
    let mut session = FakeSession::in_world();
    let player = session.identity.player.as_mut().unwrap();
    player.guid = ObjectGuid::create_player(REALM as u16, 43);
    player.level = 70;
    session.at_login_flags = at_login::CHARACTER_BOOST;
    session
}

#[tokio::test]
async fn the_boost_is_applied_at_the_next_login() {
    let h = boost_harness();
    let id = bought_boost(&h).await;
    handle_distribution_assign_to_target(&FakeSession::at_glue(), &h.service, assign(id, 43)).await;

    let mut session = boosted_session();
    session.item_counts.insert(6948, 1); // already has a hearthstone
    apply_pending_boost(&mut session, &h.service, &h.generator).await;
    let sent = session.take_sent();
    assert_eq!(opcodes(&sent), [UPGRADE_COMPLETE, DISTRIBUTION_UPDATE]);
    assert_eq!(session.grants, vec![vec![(199503, 1), (199504, 1)]]);
    assert_eq!(session.at_login_flags, 0);
    assert_eq!(session.money, h.service.config().boost_money);
    assert_eq!(
        h.characters.state().money_added,
        h.service.config().boost_money
    );
    assert_eq!(h.characters.row(43).at_login_flags, 0);
    assert_eq!(
        h.distributions.only().status,
        BATTLE_PAY_DISTRIBUTION_STATUS_FINISHED_LIKE_CPP
    );

    // Nothing is pending any more: a later login is a no-op.
    let mut again = boosted_session();
    again.at_login_flags = 0;
    apply_pending_boost(&mut again, &h.service, &h.generator).await;
    assert!(again.take_sent().is_empty());
}

#[tokio::test]
async fn a_boost_without_bag_space_waits_for_the_next_login() {
    let h = boost_harness();
    let id = bought_boost(&h).await;
    handle_distribution_assign_to_target(&FakeSession::at_glue(), &h.service, assign(id, 43)).await;
    let mut session = boosted_session();
    session.bag_space = false;
    apply_pending_boost(&mut session, &h.service, &h.generator).await;
    assert!(session.take_sent().is_empty());
    assert_eq!(session.at_login_flags, at_login::CHARACTER_BOOST);
    assert_eq!(
        h.distributions.only().status,
        BATTLE_PAY_DISTRIBUTION_STATUS_ASSIGNED_LIKE_CPP
    );
}

#[tokio::test]
async fn a_boost_flag_without_a_distribution_is_cleared() {
    let h = boost_harness();
    let mut session = boosted_session();
    apply_pending_boost(&mut session, &h.service, &h.generator).await;
    assert_eq!(session.at_login_flags, 0);
    assert!(session.grants.is_empty());
}

#[test]
fn unrevoke_reports_failure_without_revocation() {
    let session = FakeSession::at_glue();
    handle_character_upgrade_manual_unrevoke_request(
        &session,
        CharacterUpgradeManualUnrevokeRequest {
            character_guid: ObjectGuid::create_player(1, 43),
        },
    );
    let sent = session.take_sent();
    assert_eq!(opcodes(&sent), [UNREVOKE_RESULT]);
    assert_eq!(payload(&sent[0]).read_uint32().unwrap(), 1);
}
