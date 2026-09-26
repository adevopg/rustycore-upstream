//! Character services, VAS flow and character transfer.

use wow_constants::ServerOpcodes;
use wow_core::ObjectGuid;
use wow_core::guid::HighGuid;
use wow_packet::packets::battlepay::{
    BattlePayConfirmPurchaseResponse, BattlePayStartPurchase, BattlePayStartVasPurchase,
    GetVasAccountCharacterList, VasCheckTransferOk,
};
use wow_persistence::{
    BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP, BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP,
    BattlePayBnetGameAccountsLikeCpp,
};

use super::fakes::*;
use super::fakes_services::*;
use crate::battle_pay::constants::{at_login, error, purchase_status, vas_error};
use crate::battle_pay::flow::*;
use crate::battle_pay::transfer::handle_vas_check_transfer_ok;
use crate::battle_pay::vas::{handle_get_vas_account_character_list, handle_start_vas_purchase};

const START_RESPONSE: u16 = ServerOpcodes::BattlePayStartPurchaseResponse as u16;
const PURCHASE_UPDATE: u16 = ServerOpcodes::BattlePayPurchaseUpdate as u16;
const CONFIRM: u16 = ServerOpcodes::BattlePayConfirmPurchase as u16;
const CHARACTER_LIST: u16 = ServerOpcodes::GetVasAccountCharacterListResult as u16;
const REALM_LIST: u16 = ServerOpcodes::GetVasTransferTargetRealmListResult as u16;
const VAS_STATE: u16 = ServerOpcodes::VasPurchaseStateUpdate as u16;
const VAS_COMPLETE: u16 = ServerOpcodes::VasPurchaseComplete as u16;
const START_CHECKOUT: u16 = ServerOpcodes::BattlePayStartCheckout as u16;
const TRANSFER_OK: u16 = ServerOpcodes::VasCheckTransferOkResponse as u16;

fn services_harness(config: crate::battle_pay::BattlePayConfigLikeCpp) -> Harness {
    let h = harness_with_catalog(config, FakeAccount::with_balance(100), services_catalog());
    seed_characters(&h.characters, &h.distributions);
    h
}

fn start(product_id: u32) -> BattlePayStartPurchase {
    BattlePayStartPurchase {
        client_token: 77,
        product_id,
        target_character: ObjectGuid::EMPTY,
        wow_system: String::new(),
        public_key: String::new(),
        unk_string: String::new(),
    }
}

fn vas_start(product_id: u32, character: u64) -> BattlePayStartVasPurchase {
    BattlePayStartVasPurchase {
        client_token: 78,
        product_id,
        character_guid: ObjectGuid::create_player(REALM as u16, character as i64),
        ..BattlePayStartVasPurchase::default()
    }
}

/// `(PurchaseID, ServerToken)` of the 0x2787 closing an accepted start.
fn accepted(sent: &[Vec<u8>]) -> (u64, u32) {
    assert_eq!(opcodes(sent), [START_RESPONSE, PURCHASE_UPDATE, CONFIRM]);
    let mut response = payload(&sent[0]);
    let purchase_id = response.read_uint64().unwrap();
    assert_eq!(response.read_uint32().unwrap(), error::OK);
    let mut confirm = payload(&sent[2]);
    assert_eq!(confirm.read_uint64().unwrap(), purchase_id);
    (purchase_id, confirm.read_uint32().unwrap())
}

fn confirm(server_token: u32, price_cents: u64) -> BattlePayConfirmPurchaseResponse {
    BattlePayConfirmPurchaseResponse {
        confirm_purchase: true,
        server_token,
        client_current_price_fixed_point: price_cents * 100,
    }
}

/// `(State, Errors)` of a 0x27f3.
fn vas_state(bytes: &[u8]) -> (u32, Vec<u32>) {
    assert_eq!(opcode_of(bytes), VAS_STATE);
    let mut pkt = payload(bytes);
    pkt.read_uint32().unwrap();
    pkt.read_packed_guid().unwrap();
    pkt.read_uint32().unwrap();
    let state = pkt.read_uint32().unwrap();
    pkt.read_uint64().unwrap();
    let count = pkt.read_bits(2).unwrap();
    let errors = (0..count).map(|_| pkt.read_uint32().unwrap()).collect();
    (state, errors)
}

#[test]
fn services_are_listed_at_character_select_and_items_are_not() {
    let catalog = services_catalog();
    let listed = |product_id| {
        catalog
            .product(product_id)
            .unwrap()
            .listed_at_glue_like_cpp()
    };
    assert!(listed(RENAME_PRODUCT));
    assert!(listed(BOOST_70_PRODUCT));
    assert!(listed(TRANSFER_PRODUCT));
    assert!(listed(UNDELETE_PRODUCT));
    assert!(
        listed(MOUNT_PRODUCT),
        "ItemMount is listed without a player"
    );
    assert!(
        !listed(BAG_PRODUCT),
        "plain items need a character in the world"
    );
}

#[test]
fn boost_products_carry_the_client_boost_type() {
    let catalog = services_catalog();
    let product =
        catalog.distribution_product_like_cpp(catalog.product(BOOST_70_PRODUCT).unwrap(), 0);
    assert_eq!(
        product.product_type, 1,
        "JamBattlePayProduct.Type 1 = upgrade"
    );
    assert_eq!(
        product.unk4, 7,
        "CharacterServiceInfo BoostType 7 = level 70"
    );
}

#[tokio::test]
async fn vas_purchase_at_glue_answers_the_character_list_with_the_client_token() {
    let h = services_harness(token_config());
    let session = FakeSession::at_glue();
    handle_start_purchase(&session, &h.service, start(RENAME_PRODUCT)).await;
    let sent = session.take_sent();
    assert_eq!(opcodes(&sent), [CHARACTER_LIST]);
    let mut list = payload(&sent[0]);
    assert_eq!(list.read_uint32().unwrap(), 77, "token = ClientToken");
    assert_eq!(list.read_uint32().unwrap(), 0, "result");
    assert_eq!(list.read_uint32().unwrap(), 7, "choice type");
    assert_eq!(list.read_uint32().unwrap(), 2, "both characters");
    assert!(
        h.service.purchase(ACCOUNT).is_none(),
        "nothing registered yet"
    );

    handle_start_purchase(&session, &h.service, start(TRANSFER_PRODUCT)).await;
    assert_eq!(opcodes(&session.take_sent()), [REALM_LIST]);

    handle_get_vas_account_character_list(
        &session,
        &h.service,
        GetVasAccountCharacterList {
            client_token: 90,
            choice_type: 15,
        },
    )
    .await;
    let sent = session.take_sent();
    assert_eq!(opcodes(&sent), [CHARACTER_LIST]);
    assert_eq!(payload(&sent[0]).read_uint32().unwrap(), 90);
}

#[tokio::test]
async fn rename_bought_at_glue_flags_the_chosen_character_once() {
    let h = services_harness(token_config());
    let mut session = FakeSession::at_glue();
    handle_start_vas_purchase(&session, &h.service, vas_start(RENAME_PRODUCT, 43)).await;
    let (_, server_token) = accepted(&session.take_sent());

    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm(server_token, 1000),
    )
    .await;
    let sent = session.take_sent();
    // Purchase update before the delivery packet (LegionCore order).
    assert_eq!(opcodes(&sent), [PURCHASE_UPDATE, VAS_COMPLETE]);
    assert_eq!(h.account.balance(), 90);
    assert_eq!(h.characters.row(43).at_login_flags, at_login::RENAME);
    let order = h.account.only_order();
    assert_eq!(order.insert.character_guid, 43);
    assert_eq!(order.status, BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP);
    assert_eq!(h.characters.state().commits, 1);

    // A replayed recovery of the same (re-opened) order grants nothing again.
    h.account.set_status(
        &order.insert.external_id,
        BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP,
        "",
    );
    deliver_paid_purchases(&mut session, &h.service, &h.generator).await;
    assert_eq!(h.characters.state().commits, 1);
    assert_eq!(
        h.account.only_order().status,
        BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP
    );
}

#[tokio::test]
async fn vas_start_refusals_use_the_client_vas_errors() {
    let h = services_harness(token_config());
    let session = FakeSession::at_glue();
    let mut flagged = character(44, ACCOUNT, 30);
    flagged.at_login_flags = at_login::RENAME;
    h.characters.add(flagged);
    h.characters.add(character(45, ACCOUNT, 5));
    h.characters.add(character(46, 99, 30));

    for (product, guid, expected) in [
        (RENAME_PRODUCT, 44, vas_error::ALREADY_RENAME_FLAGGED),
        (RENAME_PRODUCT, 45, vas_error::UNDER_MIN_LEVEL_REQ),
        (RENAME_PRODUCT, 46, vas_error::INVALID_SOURCE_ACCOUNT),
        (FACTION_PRODUCT, 999, vas_error::INVALID_SOURCE_ACCOUNT),
    ] {
        handle_start_vas_purchase(&session, &h.service, vas_start(product, guid)).await;
        let sent = session.take_sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(vas_state(&sent[0]), (0, vec![expected]));
    }
    assert!(h.service.purchase(ACCOUNT).is_none());
}

#[tokio::test]
async fn faction_and_race_services_set_their_at_login_flags() {
    for (product, price, flag) in [
        (FACTION_PRODUCT, 2500, at_login::CHANGE_FACTION),
        (RACE_PRODUCT, 2000, at_login::CHANGE_RACE),
        (CUSTOMIZE_PRODUCT, 1000, at_login::CUSTOMIZE),
    ] {
        let h = services_harness(token_config());
        let mut session = FakeSession::at_glue();
        handle_start_vas_purchase(&session, &h.service, vas_start(product, 43)).await;
        let (_, server_token) = accepted(&session.take_sent());
        handle_confirm_purchase_response(
            &mut session,
            &h.service,
            &h.generator,
            confirm(server_token, price),
        )
        .await;
        assert_eq!(h.characters.row(43).at_login_flags, flag);
        // A second appearance service on the same character is refused.
        handle_start_vas_purchase(&session, &h.service, vas_start(CUSTOMIZE_PRODUCT, 43)).await;
        let sent = session.take_sent();
        assert_eq!(opcode_of(sent.last().unwrap()), VAS_STATE);
    }
}

#[tokio::test]
async fn in_world_rename_mirrors_the_flag_on_the_player() {
    let h = services_harness(token_config());
    let mut session = FakeSession::in_world();
    handle_start_purchase(&session, &h.service, start(INGAME_RENAME_PRODUCT)).await;
    let (_, server_token) = accepted(&session.take_sent());
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm(server_token, 1000),
    )
    .await;
    assert_eq!(h.characters.row(42).at_login_flags, at_login::RENAME);
    assert_eq!(session.at_login_flags, at_login::RENAME);

    // Without a character in the world a non-VAS service cannot be bought.
    let glue = FakeSession::at_glue();
    handle_start_purchase(&glue, &h.service, start(INGAME_RENAME_PRODUCT)).await;
    let sent = glue.take_sent();
    assert_eq!(opcodes(&sent), [START_RESPONSE]);
    assert_eq!(payload(&sent[0]).read_uint64().unwrap(), 0);
}

#[tokio::test]
async fn restore_deleted_character_clears_the_undelete_cooldown() {
    let h = services_harness(token_config());
    let mut session = FakeSession::at_glue();
    handle_start_purchase(&session, &h.service, start(UNDELETE_PRODUCT)).await;
    let (_, server_token) = accepted(&session.take_sent());
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm(server_token, 500),
    )
    .await;
    assert_eq!(h.distributions.state().undelete_resets, [70]);
    assert_eq!(
        h.account.only_order().status,
        BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP
    );
    let purchase = h.service.purchase(ACCOUNT).unwrap();
    assert_eq!(purchase.status, purchase_status::FINISH);
}

fn transfer_start(character: u64, account: u32, bnet: u32) -> BattlePayStartVasPurchase {
    BattlePayStartVasPurchase {
        wow_account_guid: ObjectGuid::create_global(HighGuid::WowAccount, 0, i64::from(account)),
        bnet_account_guid: ObjectGuid::create_global(HighGuid::BNetAccount, 0, i64::from(bnet)),
        ..vas_start(TRANSFER_PRODUCT, character)
    }
}

#[tokio::test]
async fn transfer_moves_the_character_to_the_checked_account() {
    let h = services_harness(token_config());
    let mut session = FakeSession::at_glue();
    handle_start_vas_purchase(&session, &h.service, transfer_start(43, 8, 80)).await;
    let (_, server_token) = accepted(&session.take_sent());
    let order_target = h.service.purchase(ACCOUNT).unwrap().transfer.unwrap();
    assert_eq!(
        (order_target.account_id, order_target.battlenet_account_id),
        (8, 80)
    );
    handle_confirm_purchase_response(
        &mut session,
        &h.service,
        &h.generator,
        confirm(server_token, 2000),
    )
    .await;
    assert_eq!(
        opcodes(&session.take_sent()),
        [PURCHASE_UPDATE, VAS_COMPLETE]
    );
    assert_eq!(h.characters.row(43).account_id, 8);
    assert_eq!(h.account.only_order().insert.vas_target_account, 8);
}

#[tokio::test]
async fn transfer_refusals() {
    let h = services_harness(token_config());
    let session = FakeSession::at_glue();
    let mut leader = character(47, ACCOUNT, 30);
    leader.guild_id = 5;
    leader.guild_leader_guid = 47;
    h.characters.add(leader);

    let cases = [
        (
            transfer_start(43, 8, 81),
            vas_error::INVALID_DESTINATION_ACCOUNT,
        ),
        (
            transfer_start(43, ACCOUNT, 70),
            vas_error::INVALID_DESTINATION_ACCOUNT,
        ),
        (
            transfer_start(47, 8, 80),
            vas_error::CANNOT_MOVE_GUILD_MASTER,
        ),
        (
            BattlePayStartVasPurchase {
                destination_realm_address: 0x0201_0002,
                ..transfer_start(43, 8, 80)
            },
            vas_error::INELIGIBLE_TARGET_REALM,
        ),
    ];
    for (request, expected) in cases {
        handle_start_vas_purchase(&session, &h.service, request).await;
        let sent = session.take_sent();
        assert_eq!(vas_state(&sent[0]).1, vec![expected]);
    }
    // The in-world character cannot be moved.
    let in_world = FakeSession::in_world();
    handle_start_vas_purchase(&in_world, &h.service, transfer_start(42, 8, 80)).await;
    assert_eq!(
        vas_state(&in_world.take_sent()[0]).1,
        vec![vas_error::CHAR_LOCKED]
    );
}

#[tokio::test]
async fn battlenet_transfer_validation_lists_the_other_accounts() {
    let h = services_harness(token_config());
    h.distributions.state().bnet_by_email.insert(
        "other@example.invalid".into(),
        BattlePayBnetGameAccountsLikeCpp {
            battlenet_account_id: 80,
            game_accounts: vec![(8, "80#1".into())],
        },
    );
    let session = FakeSession::at_glue();
    let check = |email: &str| VasCheckTransferOk {
        client_token: 5,
        bnet_account_name: email.into(),
    };
    handle_vas_check_transfer_ok(&session, &h.service, check(" other@example.invalid ")).await;
    let sent = session.take_sent();
    assert_eq!(opcodes(&sent), [TRANSFER_OK]);
    let mut pkt = payload(&sent[0]);
    assert_eq!(pkt.read_uint32().unwrap(), 5);
    assert_eq!(pkt.read_uint32().unwrap(), 0);
    pkt.read_packed_guid().unwrap();
    assert_eq!(pkt.read_uint32().unwrap(), 1);

    handle_vas_check_transfer_ok(&session, &h.service, check("nobody")).await;
    let mut pkt = payload(&session.take_sent()[0]);
    pkt.read_uint32().unwrap();
    assert_eq!(pkt.read_uint32().unwrap(), 1, "invalid e-mail");
}

#[tokio::test]
async fn web_vas_order_records_the_target_and_is_delivered_when_paid() {
    let h = services_harness(web_config());
    let mut session = FakeSession::at_glue();
    handle_start_vas_purchase(&session, &h.service, vas_start(RACE_PRODUCT, 43)).await;
    let sent = session.take_sent();
    assert_eq!(
        opcodes(&sent),
        [START_RESPONSE, PURCHASE_UPDATE, START_CHECKOUT]
    );
    let order = h.account.only_order();
    assert_eq!(order.insert.character_guid, 43);
    assert_eq!(h.characters.row(43).at_login_flags, 0);

    // The web marks the order paid; the next product list delivers it at glue.
    h.account.set_status(
        &order.insert.external_id,
        BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP,
        "",
    );
    deliver_paid_purchases(&mut session, &h.service, &h.generator).await;
    assert_eq!(h.characters.row(43).at_login_flags, at_login::CHANGE_RACE);
    assert_eq!(
        h.account.only_order().status,
        BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP
    );
}
