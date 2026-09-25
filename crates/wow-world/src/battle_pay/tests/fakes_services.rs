//! In-memory doubles of the character-service ports (distributions in the Login
//! DB, character rows in the Character DB) and the services demo catalog.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use wow_persistence::{
    BATTLE_PAY_DISTRIBUTION_STATUS_ASSIGNED_LIKE_CPP,
    BATTLE_PAY_DISTRIBUTION_STATUS_AVAILABLE_LIKE_CPP,
    BATTLE_PAY_DISTRIBUTION_STATUS_FINISHED_LIKE_CPP,
    BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP, BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP,
    BattlePayBnetGameAccountsLikeCpp, BattlePayBoostCompletionLikeCpp, BattlePayCatalogRowsLikeCpp,
    BattlePayCharacterRowLikeCpp, BattlePayCharacterServicePersistencePortLikeCpp,
    BattlePayCharacterTransferLikeCpp, BattlePayDeliveryReceiptLikeCpp,
    BattlePayDisplayInfoRowLikeCpp, BattlePayDistributionAssignLikeCpp,
    BattlePayDistributionGrantLikeCpp, BattlePayDistributionPersistencePortLikeCpp,
    BattlePayDistributionRowLikeCpp, BattlePayProductGroupRowLikeCpp, BattlePayProductRowLikeCpp,
    BattlePayShopEntryRowLikeCpp, PersistenceFutureLikeCpp, PersistenceOutcomeLikeCpp,
    PlayerInventoryPersistenceRequestLikeCpp,
};

use super::fakes::{ACCOUNT, FakeAccount, FakeDelivery, seed_rows};
use crate::battle_pay::catalog::BattlePayCatalogLikeCpp;

pub(super) const RENAME_PRODUCT: u32 = 20;
pub(super) const CUSTOMIZE_PRODUCT: u32 = 21;
pub(super) const FACTION_PRODUCT: u32 = 22;
pub(super) const RACE_PRODUCT: u32 = 23;
pub(super) const UNDELETE_PRODUCT: u32 = 24;
pub(super) const BOOST_70_PRODUCT: u32 = 25;
pub(super) const TRANSFER_PRODUCT: u32 = 189;
/// A rename sold outside the VAS flow (LegionCore in-world `CharacterService`).
pub(super) const INGAME_RENAME_PRODUCT: u32 = 30;

pub(super) const WARRIOR_LOADOUT_70: [u32; 3] = [6948, 199503, 199504];

/// `(class 1, purpose 12)` like `CharacterLoadout.db2` loadout 1657 (trimmed).
pub(super) fn boost_loadouts() -> HashMap<(u8, i32), Vec<u32>> {
    HashMap::from([((1, 12), WARRIOR_LOADOUT_70.to_vec())])
}

/// The seed catalog plus the services group of `2026_09_25_01_world.sql`.
pub(super) fn services_catalog() -> BattlePayCatalogLikeCpp {
    let mut rows: BattlePayCatalogRowsLikeCpp = seed_rows();
    rows.groups.push(BattlePayProductGroupRowLikeCpp {
        group_id: 22,
        name: "Services".into(),
        icon_file_data_id: 1126584,
        ordering: 4,
        token_type: 1,
        ingame_only: false,
        ..Default::default()
    });
    let mut product = |product_id, cents, product_type, choice_type, website_type, script: &str| {
        rows.display_infos.push(BattlePayDisplayInfoRowLikeCpp {
            display_info_id: product_id,
            file_data_id: 1126584,
            names: [
                format!("Service {product_id}"),
                String::new(),
                String::new(),
                String::new(),
            ],
            ..Default::default()
        });
        rows.products.push(BattlePayProductRowLikeCpp {
            product_id,
            normal_price_cents: cents,
            current_price_cents: cents,
            product_type,
            choice_type,
            website_type,
            display_info_id: product_id,
            script_name: script.into(),
            ..Default::default()
        });
        rows.shop_entries.push(BattlePayShopEntryRowLikeCpp {
            entry_id: product_id,
            group_id: 22,
            product_id,
            ordering: 1,
            ..Default::default()
        });
    };
    product(RENAME_PRODUCT, 1000, 0, 7, 5, "");
    product(CUSTOMIZE_PRODUCT, 1000, 0, 9, 22, "");
    product(FACTION_PRODUCT, 2500, 0, 8, 9, "");
    product(RACE_PRODUCT, 2000, 0, 10, 10, "");
    product(UNDELETE_PRODUCT, 500, 0, 0, 15, "");
    product(BOOST_70_PRODUCT, 4000, 1, 0, 29, "battlepay_boost_70");
    product(TRANSFER_PRODUCT, 2000, 0, 15, 12, "");
    product(INGAME_RENAME_PRODUCT, 1000, 0, 0, 5, "");
    BattlePayCatalogLikeCpp::from_rows_like_cpp(rows, |_| true).0
}

pub(super) fn character(guid: u64, account_id: u32, level: u8) -> BattlePayCharacterRowLikeCpp {
    BattlePayCharacterRowLikeCpp {
        guid,
        account_id,
        name: format!("Char{guid}"),
        race: 1,
        class: 1,
        gender: 0,
        level,
        ..Default::default()
    }
}

#[derive(Default)]
pub(super) struct CharactersState {
    pub rows: HashMap<u64, BattlePayCharacterRowLikeCpp>,
    pub commits: usize,
    pub money_added: u64,
}

pub(super) struct FakeCharacters {
    pub state: Mutex<CharactersState>,
    delivery: Arc<FakeDelivery>,
}

impl FakeCharacters {
    pub(super) fn new(delivery: Arc<FakeDelivery>) -> Self {
        Self {
            state: Mutex::new(CharactersState::default()),
            delivery,
        }
    }

    pub(super) fn state(&self) -> std::sync::MutexGuard<'_, CharactersState> {
        self.state.lock().unwrap()
    }

    pub(super) fn add(&self, row: BattlePayCharacterRowLikeCpp) {
        self.state().rows.insert(row.guid, row);
    }

    pub(super) fn row(&self, guid: u64) -> BattlePayCharacterRowLikeCpp {
        self.state().rows.get(&guid).cloned().expect("character")
    }

    /// Insert the receipt or refuse a replay (primary key of the real table).
    fn take_receipt(&self, receipt: &BattlePayDeliveryReceiptLikeCpp) -> bool {
        self.delivery
            .state()
            .receipts
            .insert(receipt.external_id.clone())
    }
}

fn rolled_back() -> PersistenceOutcomeLikeCpp {
    PersistenceOutcomeLikeCpp::Failed {
        reason: "rolled back".into(),
    }
}

impl BattlePayCharacterServicePersistencePortLikeCpp for FakeCharacters {
    fn load_account_characters_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<BattlePayCharacterRowLikeCpp>, String>> {
        let mut rows: Vec<_> = self
            .state()
            .rows
            .values()
            .filter(|row| row.account_id == account_id)
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.guid);
        Box::pin(async move { Ok(rows) })
    }

    fn load_character_like_cpp(
        &self,
        character_guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayCharacterRowLikeCpp>, String>> {
        let row = self.state().rows.get(&character_guid).cloned();
        Box::pin(async move { Ok(row) })
    }

    fn persist_service_delivery_like_cpp(
        &self,
        receipt: BattlePayDeliveryReceiptLikeCpp,
        at_login_flags: u16,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        let mut state = self.state();
        let owned = state
            .rows
            .get(&receipt.character_guid)
            .is_some_and(|row| row.account_id == receipt.account_id);
        if !owned || !self.take_receipt(&receipt) {
            return Box::pin(async { rolled_back() });
        }
        state
            .rows
            .get_mut(&receipt.character_guid)
            .expect("owned")
            .at_login_flags |= at_login_flags;
        state.commits += 1;
        Box::pin(async { PersistenceOutcomeLikeCpp::Applied { rows: 0 } })
    }

    fn persist_transfer_delivery_like_cpp(
        &self,
        receipt: BattlePayDeliveryReceiptLikeCpp,
        transfer: BattlePayCharacterTransferLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        let mut state = self.state();
        let movable = state
            .rows
            .get(&transfer.character_guid)
            .is_some_and(|row| row.account_id == transfer.from_account_id && !row.online);
        if !movable || !self.take_receipt(&receipt) {
            return Box::pin(async { rolled_back() });
        }
        let row = state
            .rows
            .get_mut(&transfer.character_guid)
            .expect("movable");
        row.account_id = transfer.to_account_id;
        row.at_login_flags |= transfer.add_at_login_flags;
        row.guild_id = 0;
        state.commits += 1;
        Box::pin(async { PersistenceOutcomeLikeCpp::Applied { rows: 0 } })
    }

    fn queue_character_boost_like_cpp(
        &self,
        character_guid: u64,
        account_id: u32,
        level: u8,
        at_login_flags: u16,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        let mut state = self.state();
        let rows = match state.rows.get_mut(&character_guid) {
            Some(row) if row.account_id == account_id && !row.online && row.level < level => {
                row.level = level;
                row.at_login_flags |= at_login_flags;
                1
            }
            _ => 0,
        };
        Box::pin(async move { PersistenceOutcomeLikeCpp::Applied { rows } })
    }

    fn persist_boost_completion_like_cpp(
        &self,
        receipt: BattlePayDeliveryReceiptLikeCpp,
        completion: BattlePayBoostCompletionLikeCpp,
        _inventory: Vec<PlayerInventoryPersistenceRequestLikeCpp>,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        if !self.take_receipt(&receipt) {
            return Box::pin(async { rolled_back() });
        }
        let mut state = self.state();
        if let Some(row) = state.rows.get_mut(&completion.character_guid) {
            row.at_login_flags &= !completion.remove_at_login_flags;
        }
        state.money_added += completion.money;
        state.commits += 1;
        Box::pin(async { PersistenceOutcomeLikeCpp::Applied { rows: 0 } })
    }
}

#[derive(Default)]
pub(super) struct DistributionsState {
    pub rows: Vec<(u32, BattlePayDistributionRowLikeCpp)>,
    pub undelete_resets: Vec<u32>,
    /// `account.id -> battlenet_account`.
    pub accounts: HashMap<u32, u32>,
    pub bnet_by_email: HashMap<String, BattlePayBnetGameAccountsLikeCpp>,
}

pub(super) struct FakeDistributions {
    pub state: Mutex<DistributionsState>,
    account: Arc<FakeAccount>,
}

impl FakeDistributions {
    pub(super) fn new(account: Arc<FakeAccount>) -> Self {
        Self {
            state: Mutex::new(DistributionsState::default()),
            account,
        }
    }

    pub(super) fn state(&self) -> std::sync::MutexGuard<'_, DistributionsState> {
        self.state.lock().unwrap()
    }

    pub(super) fn only(&self) -> BattlePayDistributionRowLikeCpp {
        let state = self.state();
        assert_eq!(state.rows.len(), 1, "exactly one distribution expected");
        state.rows[0].1.clone()
    }

    /// The order transition shared by the grant transactions.
    fn consume_paid_order(&self, external_id: &str, web_order_id: &str) -> Option<(u32, u32)> {
        let mut account = self.account.state();
        let order = account.orders.iter_mut().find(|order| {
            order.insert.external_id == external_id
                && order.status == BATTLE_PAY_PURCHASE_STATUS_PAID_LIKE_CPP
        })?;
        order.status = BATTLE_PAY_PURCHASE_STATUS_DELIVERED_LIKE_CPP;
        if order.web_order_id.is_empty() {
            order.web_order_id = web_order_id.to_owned();
        }
        Some((order.insert.account_id, order.insert.product_id))
    }
}

impl BattlePayDistributionPersistencePortLikeCpp for FakeDistributions {
    fn load_distributions_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<BattlePayDistributionRowLikeCpp>, String>> {
        let rows = self
            .state()
            .rows
            .iter()
            .filter(|(account, row)| {
                *account == account_id
                    && (row.status < BATTLE_PAY_DISTRIBUTION_STATUS_FINISHED_LIKE_CPP
                        || row.revoked)
            })
            .map(|(_, row)| row.clone())
            .collect();
        Box::pin(async move { Ok(rows) })
    }

    fn grant_distribution_like_cpp(
        &self,
        grant: BattlePayDistributionGrantLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        let Some((account_id, product_id)) =
            self.consume_paid_order(&grant.external_id, &grant.web_order_id)
        else {
            return Box::pin(async { rolled_back() });
        };
        self.state().rows.push((
            account_id,
            BattlePayDistributionRowLikeCpp {
                id: grant.distribution_id,
                product_id,
                status: BATTLE_PAY_DISTRIBUTION_STATUS_AVAILABLE_LIKE_CPP,
                ..Default::default()
            },
        ));
        Box::pin(async { PersistenceOutcomeLikeCpp::Applied { rows: 0 } })
    }

    fn assign_distribution_like_cpp(
        &self,
        assign: BattlePayDistributionAssignLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        let mut rows = 0;
        for (account, row) in self.state().rows.iter_mut() {
            if row.id == assign.distribution_id
                && *account == assign.account_id
                && row.status == BATTLE_PAY_DISTRIBUTION_STATUS_AVAILABLE_LIKE_CPP
                && !row.revoked
            {
                row.status = BATTLE_PAY_DISTRIBUTION_STATUS_ASSIGNED_LIKE_CPP;
                row.realm_id = assign.realm_id;
                row.character_guid = assign.character_guid;
                row.specialization_id = assign.specialization_id;
                rows += 1;
            }
        }
        Box::pin(async move { PersistenceOutcomeLikeCpp::Applied { rows } })
    }

    fn unassign_distribution_like_cpp(
        &self,
        distribution_id: u64,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        let mut rows = 0;
        for (account, row) in self.state().rows.iter_mut() {
            if row.id == distribution_id
                && *account == account_id
                && row.status == BATTLE_PAY_DISTRIBUTION_STATUS_ASSIGNED_LIKE_CPP
            {
                row.status = BATTLE_PAY_DISTRIBUTION_STATUS_AVAILABLE_LIKE_CPP;
                row.character_guid = 0;
                row.realm_id = 0;
                rows += 1;
            }
        }
        Box::pin(async move { PersistenceOutcomeLikeCpp::Applied { rows } })
    }

    fn load_pending_distribution_like_cpp(
        &self,
        character_guid: u64,
        realm_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayDistributionRowLikeCpp>, String>> {
        let row = self
            .state()
            .rows
            .iter()
            .map(|(_, row)| row)
            .find(|row| {
                row.character_guid == character_guid
                    && row.realm_id == realm_id
                    && row.status == BATTLE_PAY_DISTRIBUTION_STATUS_ASSIGNED_LIKE_CPP
            })
            .cloned();
        Box::pin(async move { Ok(row) })
    }

    fn finish_distribution_like_cpp(
        &self,
        distribution_id: u64,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        let mut rows = 0;
        for (_, row) in self.state().rows.iter_mut() {
            if row.id == distribution_id
                && row.status == BATTLE_PAY_DISTRIBUTION_STATUS_ASSIGNED_LIKE_CPP
            {
                row.status = BATTLE_PAY_DISTRIBUTION_STATUS_FINISHED_LIKE_CPP;
                rows += 1;
            }
        }
        Box::pin(async move { PersistenceOutcomeLikeCpp::Applied { rows } })
    }

    fn grant_undelete_like_cpp(
        &self,
        battlenet_account_id: u32,
        external_id: String,
        web_order_id: String,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        if self
            .consume_paid_order(&external_id, &web_order_id)
            .is_none()
        {
            return Box::pin(async { rolled_back() });
        }
        self.state().undelete_resets.push(battlenet_account_id);
        Box::pin(async { PersistenceOutcomeLikeCpp::Applied { rows: 0 } })
    }

    fn load_account_battlenet_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<u32>, String>> {
        let bnet = self.state().accounts.get(&account_id).copied();
        Box::pin(async move { Ok(bnet) })
    }

    fn load_bnet_game_accounts_like_cpp(
        &self,
        email: String,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayBnetGameAccountsLikeCpp>, String>>
    {
        let found = self.state().bnet_by_email.get(&email).cloned();
        Box::pin(async move { Ok(found) })
    }
}

/// Characters and accounts every services test starts from: this account owns
/// guid 42 (level 20, the in-world character of `FakeSession::in_world`) and 43
/// (level 30); account 8 of Battle.net account 80 has room for transfers.
pub(super) fn seed_characters(characters: &FakeCharacters, distributions: &FakeDistributions) {
    characters.add(character(42, ACCOUNT, 20));
    characters.add(character(43, ACCOUNT, 30));
    let mut state = distributions.state();
    state.accounts.insert(ACCOUNT, 70);
    state.accounts.insert(8, 80);
}
