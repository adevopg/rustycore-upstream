//! The production `WorldSession` side of the shop: bag planning, the item grant
//! through the shared `StoreNewItem` projection, the durable delivery batch and the
//! realm-connection packets, driven through the registered opcode handlers.

use std::sync::Arc;

use wow_constants::{ClientOpcodes, InventoryType, ItemClass, ServerOpcodes};
use wow_core::guid::HighGuid;
use wow_core::{ObjectGuid, ObjectGuidGenerator, Position};
use wow_data::{ItemRecord, ItemSparseTemplateEntry, ItemStatsStore, ItemStore};
use wow_packet::WorldPacket;
use wow_persistence::{
    BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP, BattlePayAccountPersistencePortLikeCpp,
    BattlePayDeliveryPersistencePortLikeCpp,
};

use super::fakes::*;
use crate::battle_pay::BattlePayServiceLikeCpp;
use crate::battle_pay::flow::BattlePaySessionLikeCpp;
use crate::session::{SessionHandlerCatalogsLikeCpp, SessionIdGeneratorsLikeCpp, WorldSession};

fn install_mount_item(session: &mut WorldSession) {
    session.set_item_store(Arc::new(ItemStore::from_records([ItemRecord {
        id: MOUNT_ITEM,
        class_id: ItemClass::Miscellaneous as u8,
        subclass_id: 5,
        material: 0,
        inventory_type: InventoryType::NonEquip as i8,
        sheathe_type: 0,
        random_select: 0,
        random_suffix_group_id: 0,
        scaling_stat_distribution_id: 0,
        scaling_stat_value: 0,
    }])));
    session.set_item_stats_store(Arc::new(ItemStatsStore::from_sparse_templates([(
        MOUNT_ITEM,
        ItemSparseTemplateEntry {
            flags: [0; 4],
            bag_family: 0,
            start_quest_id: 0,
            stackable: 1,
            max_count: 0,
            lock_id: 0,
            required_reputation_rank: 0,
            sell_price: 0,
            buy_price: 0,
            vendor_stack_count: 1,
            price_variance: 1.0,
            price_random_value: 1.0,
            max_durability: 0,
            other_faction_item_id: 0,
            content_tuning_id: 0,
            player_level_to_item_level_curve_id: 0,
            limit_category: 0,
            instance_bound: 0,
            zone_bound: [0, 0],
            required_reputation_faction: 0,
            allowable_class: -1,
            required_expansion: 0,
            bonding: 1,
            container_slots: 0,
            inventory_type: InventoryType::NonEquip as i8,
        },
    )])));
}

fn world_session() -> (WorldSession, flume::Receiver<Vec<u8>>) {
    let (_pkt_tx, pkt_rx) = flume::bounded(8);
    let (send_tx, _send_rx) = flume::bounded(64);
    let mut session = WorldSession::new(
        ACCOUNT,
        "BattlePayTest".into(),
        0,
        2,
        9,
        54261,
        vec![0; 40],
        "esES".into(),
        pkt_rx,
        send_tx,
    );
    session.set_state(crate::session::SessionState::LoggedIn);
    session.set_realm_id(REALM as u16);
    session.set_player_guid(Some(ObjectGuid::create_player(1, 42)));
    session.set_loaded_player_identity_like_cpp(571, 1, 1, 80, 0);
    session.set_player_position_like_cpp(Position::new(10.0, 0.0, 0.0, 0.0));
    session.set_remote_address_like_cpp(Some("198.51.100.4".into()));
    install_mount_item(&mut session);
    let (realm_tx, realm_rx) = flume::unbounded();
    session.install_realm_send_channel_for_test(realm_tx);
    (session, realm_rx)
}

fn catalogs(service: BattlePayServiceLikeCpp) -> SessionHandlerCatalogsLikeCpp {
    SessionHandlerCatalogsLikeCpp {
        battle_pay: Arc::new(service),
        id_generators: Arc::new(SessionIdGeneratorsLikeCpp {
            item: Arc::new(ObjectGuidGenerator::new(HighGuid::Item, 500)),
            ..Default::default()
        }),
        ..Default::default()
    }
}

async fn dispatch(
    session: &mut WorldSession,
    catalogs: &SessionHandlerCatalogsLikeCpp,
    opcode: ClientOpcodes,
    body: impl FnOnce(&mut WorldPacket),
) {
    let mut packet = WorldPacket::new_empty();
    packet.write_uint16(opcode as u16);
    body(&mut packet);
    session
        .dispatch_packet(catalogs, WorldPacket::from_bytes(packet.data()))
        .await;
}

fn drain(rx: &flume::Receiver<Vec<u8>>) -> Vec<Vec<u8>> {
    rx.try_iter().collect()
}

#[test]
fn identity_reads_the_session_account_locale_and_address() {
    let (session, _) = world_session();
    let identity = session.battle_pay_identity();
    assert_eq!(identity.account_id, ACCOUNT);
    assert_eq!(identity.realm_id, REALM);
    assert_eq!(identity.locale, 6);
    assert_eq!(identity.ip, "198.51.100.4");
    assert_eq!(
        identity.player.unwrap().guid,
        ObjectGuid::create_player(1, 42)
    );
    assert!(session.battle_pay_item_is_mount(MOUNT_ITEM));
    assert!(session.battle_pay_can_store(&[(MOUNT_ITEM, 1)]));
}

#[tokio::test]
async fn registered_handlers_charge_and_store_the_item_in_the_real_inventory() {
    let (mut session, realm_rx) = world_session();
    let account = FakeAccount::with_balance(100);
    let delivery = Arc::new(FakeDelivery::default());
    let catalogs = catalogs(BattlePayServiceLikeCpp::new(
        token_config(),
        Arc::new(seed_catalog()),
        Arc::clone(&account) as Arc<dyn BattlePayAccountPersistencePortLikeCpp>,
        Arc::clone(&delivery) as Arc<dyn BattlePayDeliveryPersistencePortLikeCpp>,
    ));

    dispatch(
        &mut session,
        &catalogs,
        ClientOpcodes::BattlePayGetProductList,
        |_| {},
    )
    .await;
    let list = drain(&realm_rx);
    assert_eq!(
        opcode_of(&list[0]),
        ServerOpcodes::BattlePayGetProductListResponse as u16
    );

    dispatch(
        &mut session,
        &catalogs,
        ClientOpcodes::BattlePayStartPurchase,
        |pkt| {
            pkt.write_uint32(55);
            pkt.write_uint32(MOUNT_PRODUCT);
            pkt.write_packed_guid(&ObjectGuid::EMPTY);
            pkt.write_bits(0, 6);
            pkt.write_bits(0, 12);
            pkt.write_bits(0, 7);
            pkt.flush_bits();
        },
    )
    .await;
    let started = drain(&realm_rx);
    let confirm = started
        .iter()
        .find(|bytes| opcode_of(bytes) == ServerOpcodes::BattlePayConfirmPurchase as u16)
        .expect("wallet mode asks for confirmation");
    let mut confirm = payload(confirm);
    confirm.read_uint64().unwrap();
    let server_token = confirm.read_uint32().unwrap();

    dispatch(
        &mut session,
        &catalogs,
        ClientOpcodes::BattlePayConfirmPurchaseResponse,
        |pkt| {
            pkt.write_bit(true);
            pkt.flush_bits();
            pkt.write_uint32(server_token);
            pkt.write_uint64(150_000);
        },
    )
    .await;

    assert_eq!(account.balance(), 85);
    assert_eq!(
        account.only_order().status,
        BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP
    );
    assert_eq!(delivery.state().commits, 1);
    assert!(
        session
            .inventory_items_like_cpp()
            .values()
            .any(|item| item.entry_id == MOUNT_ITEM),
        "the mount item is in the bags"
    );
    let delivered: Vec<u16> = drain(&realm_rx)
        .iter()
        .map(|bytes| opcode_of(bytes))
        .collect();
    for expected in [
        ServerOpcodes::BattlePayDeliveryStarted,
        ServerOpcodes::BattlePayMountDelivered,
        ServerOpcodes::BattlePayDeliveryEnded,
        ServerOpcodes::BattlePayPurchaseUpdate,
    ] {
        assert!(
            delivered.contains(&(expected as u16)),
            "{expected:?} sent on the realm connection"
        );
    }
}
