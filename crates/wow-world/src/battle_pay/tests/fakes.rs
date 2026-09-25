//! In-memory session and persistence doubles for the BattlePay flows.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};

use wow_core::guid::HighGuid;
use wow_core::{ObjectGuid, ObjectGuidGenerator};
use wow_packet::{ServerPacket, WorldPacket};
use wow_persistence::{
    BATTLE_PAY_PURCHASE_STATUS_CREATED_LIKE_CPP, BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP,
    BATTLE_PAY_PURCHASE_STATUS_FAILED_LIKE_CPP, BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP,
    BattlePayAccountPersistencePortLikeCpp, BattlePayCatalogRowsLikeCpp,
    BattlePayDeliveryPersistencePortLikeCpp, BattlePayDeliveryReceiptLikeCpp,
    BattlePayDisplayInfoLocaleRowLikeCpp, BattlePayDisplayInfoRowLikeCpp,
    BattlePayDisplayInfoVisualRowLikeCpp, BattlePayProductGroupLocaleRowLikeCpp,
    BattlePayProductGroupRowLikeCpp, BattlePayProductItemRowLikeCpp, BattlePayProductRowLikeCpp,
    BattlePayPurchaseInsertLikeCpp, BattlePayPurchaseRowLikeCpp, BattlePayShopEntryRowLikeCpp,
    BattlePaySsoTokenIssueLikeCpp, BattlePayTokenChargeLikeCpp, BattlePayTokenChargeOutcomeLikeCpp,
    PersistenceFutureLikeCpp, PersistenceOutcomeLikeCpp, PlayerInventoryPersistenceRequestLikeCpp,
};

use crate::battle_pay::catalog::BattlePayCatalogLikeCpp;
use crate::battle_pay::constants::BattlePayConfigLikeCpp;
use crate::battle_pay::flow::{
    BattlePayIdentityLikeCpp, BattlePayPlayerLikeCpp, BattlePaySessionLikeCpp,
};
use crate::battle_pay::service::BattlePayServiceLikeCpp;

pub(super) const ACCOUNT: u32 = 7;
pub(super) const REALM: u32 = 1;
pub(super) const MOUNT_PRODUCT: u32 = 1;
pub(super) const BAG_PRODUCT: u32 = 3;
pub(super) const MOUNT_ITEM: u32 = 43516;
pub(super) const BAG_ITEM: u32 = 38082;

/// The seed catalog of `2026_09_25_00_world.sql` (mount 15.00, bag 8.00).
pub(super) fn seed_rows() -> BattlePayCatalogRowsLikeCpp {
    let group = |group_id, name: &str, ordering| BattlePayProductGroupRowLikeCpp {
        group_id,
        name: name.into(),
        icon_file_data_id: 132261,
        ordering,
        token_type: 1,
        ingame_only: true,
        ..Default::default()
    };
    let display = |id, name: &str, file_data_id| BattlePayDisplayInfoRowLikeCpp {
        display_info_id: id,
        file_data_id,
        names: [
            name.into(),
            "Subtitle".into(),
            "Description".into(),
            String::new(),
        ],
        ..Default::default()
    };
    let product = |product_id, cents, display_info_id, website_type| BattlePayProductRowLikeCpp {
        product_id,
        normal_price_cents: cents,
        current_price_cents: cents,
        display_info_id,
        website_type,
        ..Default::default()
    };
    let item = |id, product_id, item_id| BattlePayProductItemRowLikeCpp {
        id,
        product_id,
        item_id,
        quantity: 1,
        ..Default::default()
    };
    let entry = |entry_id, group_id, product_id| BattlePayShopEntryRowLikeCpp {
        entry_id,
        group_id,
        product_id,
        ordering: 1,
        ..Default::default()
    };
    BattlePayCatalogRowsLikeCpp {
        display_infos: vec![
            display(1, "Big Blizzard Bear", 298586),
            display(3, "\"Gigantique\" Bag", 133639),
        ],
        visuals: vec![BattlePayDisplayInfoVisualRowLikeCpp {
            display_info_id: 1,
            display_id: 27567,
            visual_id: 4,
            product_name: "Big Blizzard Bear".into(),
        }],
        products: vec![
            product(MOUNT_PRODUCT, 1500, 1, 21),
            product(BAG_PRODUCT, 800, 3, 3),
        ],
        product_items: vec![
            item(1, MOUNT_PRODUCT, MOUNT_ITEM),
            item(3, BAG_PRODUCT, BAG_ITEM),
        ],
        groups: vec![group(1, "Mounts", 1), group(11, "Bags", 3)],
        shop_entries: vec![entry(1, 1, MOUNT_PRODUCT), entry(3, 11, BAG_PRODUCT)],
        group_locales: vec![BattlePayProductGroupLocaleRowLikeCpp {
            group_id: 1,
            locale: 6,
            name: "Monturas".into(),
        }],
        display_info_locales: vec![BattlePayDisplayInfoLocaleRowLikeCpp {
            display_info_id: 1,
            locale: 6,
            names: [
                "Gran oso de Blizzard".into(),
                String::new(),
                String::new(),
                String::new(),
            ],
        }],
        token_types: Vec::new(),
    }
}

pub(super) fn seed_catalog() -> BattlePayCatalogLikeCpp {
    BattlePayCatalogLikeCpp::from_rows_like_cpp(seed_rows(), |_| true).0
}

/// One stored `battlepay_purchase` row.
#[derive(Debug, Clone)]
pub(super) struct StoredOrder {
    pub insert: BattlePayPurchaseInsertLikeCpp,
    pub status: u8,
    pub web_order_id: String,
}

#[derive(Default)]
pub(super) struct AccountState {
    pub balances: HashMap<(u32, u8), i64>,
    pub orders: Vec<StoredOrder>,
    pub token_log: Vec<(u32, i64, u32)>,
    pub sso_tokens: Vec<BattlePaySsoTokenIssueLikeCpp>,
    /// Next charge reports this instead of running.
    pub charge_override: Option<BattlePayTokenChargeOutcomeLikeCpp>,
    pub mark_delivered_fails: bool,
}

#[derive(Default)]
pub(super) struct FakeAccount(pub Mutex<AccountState>);

impl FakeAccount {
    pub(super) fn with_balance(amount: i64) -> Arc<Self> {
        let account = Self::default();
        account.state().balances.insert((ACCOUNT, 1), amount);
        Arc::new(account)
    }

    pub(super) fn state(&self) -> std::sync::MutexGuard<'_, AccountState> {
        self.0.lock().unwrap()
    }

    pub(super) fn balance(&self) -> i64 {
        self.state()
            .balances
            .get(&(ACCOUNT, 1))
            .copied()
            .unwrap_or(0)
    }

    pub(super) fn order(&self, external_id: &str) -> Option<StoredOrder> {
        self.state()
            .orders
            .iter()
            .find(|order| order.insert.external_id == external_id)
            .cloned()
    }

    pub(super) fn only_order(&self) -> StoredOrder {
        let state = self.state();
        assert_eq!(state.orders.len(), 1, "exactly one order expected");
        state.orders[0].clone()
    }

    pub(super) fn set_status(&self, external_id: &str, status: u8, payment_ref: &str) {
        let mut state = self.state();
        let order = state
            .orders
            .iter_mut()
            .find(|order| order.insert.external_id == external_id)
            .expect("order exists");
        order.status = status;
        order.insert.payment_ref = payment_ref.to_owned();
    }
}

fn row(order: &StoredOrder, id: usize) -> BattlePayPurchaseRowLikeCpp {
    BattlePayPurchaseRowLikeCpp {
        id: id as u64 + 1,
        external_id: order.insert.external_id.clone(),
        product_id: order.insert.product_id,
        status: order.status,
        character_guid: order.insert.character_guid,
        payment_ref: order.insert.payment_ref.clone(),
        web_order_id: order.web_order_id.clone(),
    }
}

impl BattlePayAccountPersistencePortLikeCpp for FakeAccount {
    fn load_token_balances_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<(u8, i64)>, String>> {
        let rows = self
            .state()
            .balances
            .iter()
            .filter(|((account, _), _)| *account == account_id)
            .map(|((_, kind), amount)| (*kind, *amount))
            .collect();
        Box::pin(async move { Ok(rows) })
    }

    fn charge_tokens_like_cpp(
        &self,
        charge: BattlePayTokenChargeLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, BattlePayTokenChargeOutcomeLikeCpp> {
        let mut state = self.state();
        if let Some(outcome) = state.charge_override.take() {
            return Box::pin(async move { outcome });
        }
        let key = (charge.purchase.account_id, charge.token_type);
        let balance = state.balances.get(&key).copied().unwrap_or(0);
        if balance < charge.amount {
            return Box::pin(async move {
                BattlePayTokenChargeOutcomeLikeCpp::InsufficientBalance { balance }
            });
        }
        state.balances.insert(key, balance - charge.amount);
        state.token_log.push((
            charge.purchase.account_id,
            -charge.amount,
            charge.purchase.product_id,
        ));
        state.orders.push(StoredOrder {
            insert: charge.purchase,
            status: BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP,
            web_order_id: String::new(),
        });
        Box::pin(async { BattlePayTokenChargeOutcomeLikeCpp::Charged })
    }

    fn insert_web_purchase_like_cpp(
        &self,
        purchase: BattlePayPurchaseInsertLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        self.state().orders.push(StoredOrder {
            insert: purchase,
            status: BATTLE_PAY_PURCHASE_STATUS_CREATED_LIKE_CPP,
            web_order_id: String::new(),
        });
        Box::pin(async { PersistenceOutcomeLikeCpp::Applied { rows: 1 } })
    }

    fn load_purchase_like_cpp(
        &self,
        external_id: String,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayPurchaseRowLikeCpp>, String>> {
        let found = self
            .state()
            .orders
            .iter()
            .enumerate()
            .find(|(_, order)| {
                order.insert.external_id == external_id && order.insert.account_id == account_id
            })
            .map(|(id, order)| row(order, id));
        Box::pin(async move { Ok(found) })
    }

    fn load_paid_purchases_like_cpp(
        &self,
        account_id: u32,
        realm_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<BattlePayPurchaseRowLikeCpp>, String>> {
        let rows = self
            .state()
            .orders
            .iter()
            .enumerate()
            .filter(|(_, order)| {
                order.insert.account_id == account_id
                    && order.insert.realm_id == realm_id
                    && order.status == BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP
            })
            .map(|(id, order)| row(order, id))
            .collect();
        Box::pin(async move { Ok(rows) })
    }

    fn mark_purchase_delivered_like_cpp(
        &self,
        external_id: String,
        web_order_id: String,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        let mut state = self.state();
        if state.mark_delivered_fails {
            return Box::pin(async {
                PersistenceOutcomeLikeCpp::Failed {
                    reason: "lost".into(),
                }
            });
        }
        let mut rows = 0;
        for order in state.orders.iter_mut().filter(|order| {
            order.insert.external_id == external_id
                && order.status == BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP
        }) {
            order.status = BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP;
            if order.web_order_id.is_empty() {
                order.web_order_id = web_order_id.clone();
            }
            rows += 1;
        }
        Box::pin(async move { PersistenceOutcomeLikeCpp::Applied { rows } })
    }

    fn mark_purchase_failed_like_cpp(
        &self,
        external_id: String,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        let mut rows = 0;
        for order in self.state().orders.iter_mut().filter(|order| {
            order.insert.external_id == external_id
                && order.insert.account_id == account_id
                && order.status == BATTLE_PAY_PURCHASE_STATUS_CREATED_LIKE_CPP
        }) {
            order.status = BATTLE_PAY_PURCHASE_STATUS_FAILED_LIKE_CPP;
            rows += 1;
        }
        Box::pin(async move { PersistenceOutcomeLikeCpp::Applied { rows } })
    }

    fn issue_sso_token_like_cpp(
        &self,
        issue: BattlePaySsoTokenIssueLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, Result<String, String>> {
        let token = wow_packet_hex(&issue.random_bytes);
        self.state().sso_tokens.push(issue);
        Box::pin(async move { Ok(token) })
    }
}

fn wow_packet_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

#[derive(Default)]
pub(super) struct DeliveryState {
    pub receipts: HashSet<String>,
    pub commits: usize,
    pub fail_next_commit: bool,
}

#[derive(Default)]
pub(super) struct FakeDelivery(pub Mutex<DeliveryState>);

impl FakeDelivery {
    pub(super) fn state(&self) -> std::sync::MutexGuard<'_, DeliveryState> {
        self.0.lock().unwrap()
    }
}

impl BattlePayDeliveryPersistencePortLikeCpp for FakeDelivery {
    fn delivery_receipt_exists_like_cpp(
        &self,
        external_id: String,
    ) -> PersistenceFutureLikeCpp<'_, Result<bool, String>> {
        let exists = self.state().receipts.contains(&external_id);
        Box::pin(async move { Ok(exists) })
    }

    fn persist_delivery_like_cpp(
        &self,
        receipt: BattlePayDeliveryReceiptLikeCpp,
        _inventory: Vec<PlayerInventoryPersistenceRequestLikeCpp>,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        let mut state = self.state();
        if std::mem::take(&mut state.fail_next_commit)
            || !state.receipts.insert(receipt.external_id)
        {
            return Box::pin(async {
                PersistenceOutcomeLikeCpp::Failed {
                    reason: "rolled back".into(),
                }
            });
        }
        state.commits += 1;
        Box::pin(async { PersistenceOutcomeLikeCpp::Applied { rows: 0 } })
    }
}

/// A world session reduced to what the shop reads and writes.
pub(super) struct FakeSession {
    pub identity: BattlePayIdentityLikeCpp,
    pub sent: Mutex<Vec<Vec<u8>>>,
    pub bag_space: bool,
    pub owned: HashSet<u32>,
    pub grants: Vec<Vec<(u32, u32)>>,
    pub quarantined: Option<&'static str>,
}

impl FakeSession {
    pub(super) fn in_world() -> Self {
        Self {
            identity: BattlePayIdentityLikeCpp {
                account_id: ACCOUNT,
                battlenet_account_id: 70,
                realm_id: REALM,
                region_id: 2,
                security: 0,
                locale: 0,
                ip: "203.0.113.5".into(),
                player: Some(BattlePayPlayerLikeCpp {
                    guid: ObjectGuid::create_player(1, 42),
                    class_mask: 1,
                }),
            },
            sent: Mutex::new(Vec::new()),
            bag_space: true,
            owned: HashSet::new(),
            grants: Vec::new(),
            quarantined: None,
        }
    }

    /// Opcodes sent so far, in order, then cleared.
    pub(super) fn take_sent(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }
}

/// Opcode of a serialized server packet.
pub(super) fn opcode_of(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

pub(super) fn payload(bytes: &[u8]) -> WorldPacket {
    WorldPacket::from_bytes(&bytes[2..])
}

pub(super) fn opcodes(packets: &[Vec<u8>]) -> Vec<u16> {
    packets.iter().map(|bytes| opcode_of(bytes)).collect()
}

impl BattlePaySessionLikeCpp for FakeSession {
    fn battle_pay_identity(&self) -> BattlePayIdentityLikeCpp {
        self.identity.clone()
    }

    fn send_battle_pay_packet<P: ServerPacket>(&self, packet: &P) {
        self.sent.lock().unwrap().push(packet.to_bytes());
    }

    fn battle_pay_item_owned(&self, item_id: u32) -> bool {
        self.owned.contains(&item_id)
    }

    fn battle_pay_item_allowed(&self, _item_id: u32) -> bool {
        true
    }

    fn battle_pay_item_is_mount(&self, item_id: u32) -> bool {
        item_id == MOUNT_ITEM
    }

    fn battle_pay_can_store(&self, _items: &[(u32, u32)]) -> bool {
        self.bag_space
    }

    fn battle_pay_grant_items(
        &mut self,
        _item_guid_generator: &ObjectGuidGenerator,
        items: &[(u32, u32)],
    ) -> impl Future<Output = Option<Vec<PlayerInventoryPersistenceRequestLikeCpp>>> + Send {
        self.grants.push(items.to_vec());
        async { Some(Vec::new()) }
    }

    fn battle_pay_quarantine(&mut self, reason: &'static str) {
        self.quarantined = Some(reason);
    }
}

pub(super) struct Harness {
    pub service: BattlePayServiceLikeCpp,
    pub account: Arc<FakeAccount>,
    pub delivery: Arc<FakeDelivery>,
    pub generator: ObjectGuidGenerator,
}

pub(super) fn harness(config: BattlePayConfigLikeCpp, account: Arc<FakeAccount>) -> Harness {
    let delivery = Arc::new(FakeDelivery::default());
    let service = BattlePayServiceLikeCpp::new(
        config,
        Arc::new(seed_catalog()),
        Arc::clone(&account) as Arc<dyn BattlePayAccountPersistencePortLikeCpp>,
        Arc::clone(&delivery) as Arc<dyn BattlePayDeliveryPersistencePortLikeCpp>,
    );
    Harness {
        service,
        account,
        delivery,
        generator: ObjectGuidGenerator::new(HighGuid::Item, 1),
    }
}

pub(super) fn token_config() -> BattlePayConfigLikeCpp {
    BattlePayConfigLikeCpp {
        enabled: true,
        store_enabled_for_players: true,
        ..BattlePayConfigLikeCpp::default()
    }
}

pub(super) fn web_config() -> BattlePayConfigLikeCpp {
    BattlePayConfigLikeCpp {
        web_checkout: true,
        browser_enabled: true,
        ..token_config()
    }
}
