//! Price arithmetic and product-list projection.

use std::collections::HashMap;

use wow_persistence::{BattlePayProductItemRowLikeCpp, BattlePayProductRowLikeCpp};

use super::fakes::*;
use crate::battle_pay::catalog::{BattlePayCatalogLikeCpp, ProductListViewerLikeCpp};
use crate::battle_pay::constants::*;

fn viewer<'a>(
    in_world: bool,
    locale: u8,
    balances: &'a HashMap<u8, i64>,
    owned: &'a dyn Fn(u32) -> bool,
) -> ProductListViewerLikeCpp<'a> {
    ProductListViewerLikeCpp {
        in_world,
        locale,
        class_mask: 1,
        web_checkout: false,
        currency_id: 4,
        token_balances: balances,
        owned,
        item_allowed: &|_| true,
    }
}

#[test]
fn prices_use_legioncore_fixed_point_and_round_tokens_up() {
    assert_eq!(cents_to_fixed_point_like_cpp(1500), 150_000);
    assert_eq!(cents_to_fixed_point_like_cpp(1099), 109_900);
    assert_eq!(fixed_point_to_tokens_like_cpp(150_000), 15);
    assert_eq!(fixed_point_to_tokens_like_cpp(150_001), 16);
    assert_eq!(fixed_point_to_tokens_like_cpp(0), 0);
    assert_eq!(fixed_point_to_decimal_like_cpp(150_000), "15.00");
    assert_eq!(fixed_point_to_decimal_like_cpp(109_900), "10.99");
    assert_eq!(fixed_point_to_decimal_like_cpp(500), "0.05");
}

#[test]
fn currency_and_locale_ids_follow_legioncore_and_numeric_locale_constant() {
    assert_eq!(currency_id_from_code_like_cpp("EUR"), 4);
    assert_eq!(currency_id_from_code_like_cpp("usd"), 1);
    assert_eq!(currency_id_from_code_like_cpp("XYZ"), 0);
    assert_eq!(locale_index_from_name_like_cpp("esES"), Some(6));
    assert_eq!(locale_index_from_name_like_cpp("enUS"), Some(0));
    assert_eq!(locale_index_from_name_like_cpp("xxYY"), None);
    assert!(is_valid_locale_index_like_cpp(6));
    assert!(!is_valid_locale_index_like_cpp(9));
    assert!(!is_valid_locale_index_like_cpp(12));
}

#[test]
fn availability_needs_the_master_switch_and_player_or_moderator_access() {
    let mut config = BattlePayConfigLikeCpp::default();
    assert!(!config.is_available_for_like_cpp(3));
    config.enabled = true;
    assert!(!config.is_available_for_like_cpp(0));
    assert!(config.is_available_for_like_cpp(SEC_MODERATOR_LIKE_CPP));
    config.store_enabled_for_players = true;
    assert!(config.is_available_for_like_cpp(0));
}

#[test]
fn loader_skips_bad_website_types_unknown_items_and_bad_locales() {
    let mut rows = seed_rows();
    rows.products.push(BattlePayProductRowLikeCpp {
        product_id: 99,
        website_type: 40,
        ..Default::default()
    });
    rows.product_items.push(BattlePayProductItemRowLikeCpp {
        id: 50,
        product_id: BAG_PRODUCT,
        item_id: 1,
        quantity: 1,
        ..Default::default()
    });
    rows.product_items.push(BattlePayProductItemRowLikeCpp {
        id: 51,
        product_id: BAG_PRODUCT,
        item_id: BAG_ITEM,
        quantity: 1,
        display_info_id: 777,
        ..Default::default()
    });
    rows.group_locales[0].locale = 9;
    let (catalog, report) = BattlePayCatalogLikeCpp::from_rows_like_cpp(rows, |item| item != 1);
    assert_eq!(catalog.product_count(), 2);
    assert_eq!(report.skipped_products, 1);
    assert_eq!(report.skipped_items, 2);
    assert_eq!(report.skipped_locales, 1);
    assert_eq!(catalog.product(BAG_PRODUCT).unwrap().items.len(), 1);
    assert_eq!(
        catalog.product(MOUNT_PRODUCT).unwrap().current_price,
        150_000
    );
}

#[test]
fn in_world_list_carries_groups_entries_products_and_prices() {
    let catalog = seed_catalog();
    let balances = HashMap::from([(1u8, 100i64)]);
    let list = catalog.product_list_like_cpp(&viewer(true, 0, &balances, &|_| false));
    assert_eq!(list.result, PRODUCT_LIST_AVAILABLE_LIKE_CPP);
    assert_eq!(list.currency_id, 4);
    assert_eq!(list.product_groups.len(), 2);
    assert_eq!(list.shop_entries.len(), 2);
    assert_eq!(list.product_infos.len(), 2);
    let mount = &list.product_infos[0];
    assert_eq!(mount.product_id, MOUNT_PRODUCT);
    assert_eq!(mount.current_price_fixed_point, 150_000);
    assert_eq!(mount.product_ids, vec![MOUNT_PRODUCT]);
    assert_eq!(mount.unk1, PRODUCT_INFO_UNK1_LIKE_CPP);
    let card = mount.display_info.as_ref().unwrap();
    assert_eq!(card.name1, "Big Blizzard Bear");
    assert_eq!(card.file_data_id, Some(298586));
    assert_eq!(card.visuals.len(), 1);
    assert_eq!(card.visuals[0].visual_id, 4);
    let product = &list.products[0];
    assert_eq!(product.items[0].item_id, MOUNT_ITEM);
    assert!(!product.items[0].has_pet);
    // The whole response serializes within the codec limits.
    let _ = wow_packet::ServerPacket::to_bytes(&list);
}

#[test]
fn glue_list_hides_ingame_only_groups_and_plain_item_products() {
    let catalog = seed_catalog();
    let balances = HashMap::new();
    let list = catalog.product_list_like_cpp(&viewer(false, 0, &balances, &|_| false));
    assert!(list.product_groups.is_empty());
    assert!(list.shop_entries.is_empty());
    assert!(list.products.is_empty());
}

#[test]
fn locale_number_selects_translations_and_empty_ones_fall_back() {
    let catalog = seed_catalog();
    let balances = HashMap::new();
    let list = catalog.product_list_like_cpp(&viewer(true, 6, &balances, &|_| false));
    assert_eq!(list.product_groups[0].name, "Monturas");
    assert_eq!(list.product_groups[1].name, "Bags");
    let card = list.product_infos[0].display_info.as_ref().unwrap();
    assert_eq!(card.name1, "Gran oso de Blizzard");
    assert_eq!(card.name2, "Subtitle"); // empty translation -> default
}

#[test]
fn owned_items_and_class_restricted_products_are_filtered() {
    let mut rows = seed_rows();
    rows.products[1].class_mask = 0x2;
    let catalog = BattlePayCatalogLikeCpp::from_rows_like_cpp(rows, |_| true).0;
    let balances = HashMap::new();
    let owned = |item| item == MOUNT_ITEM;
    let list = catalog.product_list_like_cpp(&viewer(true, 0, &balances, &owned));
    assert!(
        list.products.is_empty(),
        "mount owned, bag for another class"
    );
}

#[test]
fn owns_tokens_only_groups_need_a_positive_balance_in_token_mode() {
    let mut rows = seed_rows();
    rows.groups[0].owns_tokens_only = true;
    let catalog = BattlePayCatalogLikeCpp::from_rows_like_cpp(rows, |_| true).0;
    let empty = HashMap::new();
    let list = catalog.product_list_like_cpp(&viewer(true, 0, &empty, &|_| false));
    assert_eq!(list.product_groups.len(), 1);
    assert!(list.products.iter().all(|p| p.product_id != MOUNT_PRODUCT));
    let funded = HashMap::from([(1u8, 1i64)]);
    let list = catalog.product_list_like_cpp(&viewer(true, 0, &funded, &|_| false));
    assert_eq!(list.product_groups.len(), 2);
}

#[test]
fn hidden_price_disables_buy_when_tokens_do_not_cover_it() {
    let mut rows = seed_rows();
    rows.display_infos[0].flags = DISPLAY_FLAG_HIDE_PRICE_LIKE_CPP;
    let catalog = BattlePayCatalogLikeCpp::from_rows_like_cpp(rows, |_| true).0;
    let poor = HashMap::from([(1u8, 14i64)]);
    let list = catalog.product_list_like_cpp(&viewer(true, 0, &poor, &|_| false));
    assert!(list.products[0].items[0].has_pet);
    let rich = HashMap::from([(1u8, 15i64)]);
    let list = catalog.product_list_like_cpp(&viewer(true, 0, &rich, &|_| false));
    assert!(!list.products[0].items[0].has_pet);
}

#[test]
fn undeliverable_products_are_never_listed() {
    let mut rows = seed_rows();
    rows.products[1].website_type = 29; // CharacterBoost: not ported
    let catalog = BattlePayCatalogLikeCpp::from_rows_like_cpp(rows, |_| true).0;
    let balances = HashMap::new();
    let list = catalog.product_list_like_cpp(&viewer(true, 0, &balances, &|_| false));
    assert_eq!(list.products.len(), 1);
    assert_eq!(list.products[0].product_id, MOUNT_PRODUCT);
}

#[test]
fn feature_system_available_bit_and_delivery_delay_follow_bpay_enabled() {
    let policy = crate::session::SupportFeaturePolicyLikeCpp {
        bpay_store_enabled: true,
        bpay_store_available: true,
        ..Default::default()
    };
    let (_pkt_tx, pkt_rx) = flume::bounded(1);
    let (send_tx, _send_rx) = flume::bounded(1);
    let session = crate::session::WorldSession::new(
        1,
        "BattlePay".into(),
        0,
        2,
        9,
        54261,
        vec![0; 40],
        "enUS".into(),
        pkt_rx,
        send_tx,
    );
    let status = session.feature_system_status_with_policy_like_cpp(&policy);
    assert!(status.config.bpay_store_available);
    assert_eq!(
        status.config.bpay_store_product_delivery_delay,
        PRODUCT_DELIVERY_DELAY_SECS_LIKE_CPP
    );
    let closed = session.feature_system_status_with_policy_like_cpp(&Default::default());
    assert!(!closed.config.bpay_store_available);
    assert_eq!(closed.config.bpay_store_product_delivery_delay, 0);
}
