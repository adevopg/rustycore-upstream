//! MariaDB adapters for the BattlePay character services, boosts and transfers.
//!
//! Login side: LegionCore `LOGIN_*_BPAY_DISTRIBUTION*` and the VAS transfer target
//! lookups (`LoginDatabase.cpp:148-149,222-229`). Character side: LegionCore
//! `CHAR_UPD_ADD_AT_LOGIN_FLAG` / `CHAR_UPD_CHARACTER_BOOST_QUEUED`
//! (`CharacterDatabase.cpp:524,549`) and the same-realm account move of
//! `CompleteVasCharacterTransfer`. Each write that consumes a paid order commits
//! with its witness row (the order transition on the Login side, the
//! `character_battlepay_delivery` receipt on the Character side) and an unknown
//! COMMIT is resolved by reading that witness, like `battle_pay_adapter`.

use std::sync::Arc;

use wow_persistence::{
    BattlePayBnetGameAccountsLikeCpp, BattlePayBoostCompletionLikeCpp,
    BattlePayCharacterRowLikeCpp, BattlePayCharacterServicePersistencePortLikeCpp,
    BattlePayCharacterTransferLikeCpp, BattlePayDeliveryReceiptLikeCpp,
    BattlePayDistributionAssignLikeCpp, BattlePayDistributionGrantLikeCpp,
    BattlePayDistributionPersistencePortLikeCpp, BattlePayDistributionRowLikeCpp,
    PersistenceFutureLikeCpp, PersistenceOutcomeLikeCpp, PlayerInventoryPersistenceRequestLikeCpp,
};

use crate::battle_pay_adapter::{column_u64_like_cpp, outcome_from_commit_like_cpp};
use crate::player::inventory_adapter::{
    InventoryTransactionBuilderLikeCpp, append_inventory_request_like_cpp,
};
use crate::result::SqlResult;
use crate::{
    CharStatements, CharacterDatabase, LoginDatabase, LoginStatements, PreparedStatement,
    SqlTransaction,
};

fn rows_like_cpp<T>(mut result: SqlResult, decode: impl Fn(&SqlResult) -> T) -> Vec<T> {
    let mut rows = Vec::with_capacity(result.count());
    if result.is_empty() {
        return rows;
    }
    loop {
        rows.push(decode(&result));
        if !result.next_row() {
            break;
        }
    }
    rows
}

fn distribution_row_like_cpp(result: &SqlResult) -> BattlePayDistributionRowLikeCpp {
    BattlePayDistributionRowLikeCpp {
        id: column_u64_like_cpp(result, 0),
        product_id: column_u64_like_cpp(result, 1) as u32,
        status: column_u64_like_cpp(result, 2) as u8,
        revoked: column_u64_like_cpp(result, 3) != 0,
        character_guid: column_u64_like_cpp(result, 4),
        specialization_id: column_u64_like_cpp(result, 5) as u16,
        realm_id: column_u64_like_cpp(result, 6) as u32,
    }
}

fn delivered_statement_like_cpp(external_id: &str, web_order_id: &str) -> PreparedStatement {
    let mut delivered =
        PreparedStatement::for_statement(LoginStatements::UPD_BPAY_PURCHASE_DELIVERED);
    delivered.set_string(0, web_order_id);
    delivered.set_string(1, external_id);
    delivered
}

/// Distribution created from the paid order + order delivered, one transaction.
pub(crate) fn distribution_grant_transaction_like_cpp(
    grant: &BattlePayDistributionGrantLikeCpp,
) -> SqlTransaction {
    let mut transaction = SqlTransaction::new();
    let mut insert =
        PreparedStatement::for_statement(LoginStatements::INS_BPAY_DISTRIBUTION_FROM_PURCHASE);
    insert.set_u64(0, grant.distribution_id);
    insert.set_string(1, &grant.external_id);
    transaction.append_expect_rows_affected(insert, 1);
    transaction.append_expect_rows_affected(
        delivered_statement_like_cpp(&grant.external_id, &grant.web_order_id),
        1,
    );
    transaction
}

/// Undelete cooldown cleared + order delivered, one transaction.
pub(crate) fn undelete_grant_transaction_like_cpp(
    battlenet_account_id: u32,
    external_id: &str,
    web_order_id: &str,
) -> SqlTransaction {
    let mut transaction = SqlTransaction::new();
    let mut reset =
        PreparedStatement::for_statement(LoginStatements::UPD_BPAY_RESET_UNDELETE_COOLDOWN);
    reset.set_u32(0, battlenet_account_id);
    transaction.append(reset);
    transaction
        .append_expect_rows_affected(delivered_statement_like_cpp(external_id, web_order_id), 1);
    transaction
}

pub struct MariaDbBattlePayDistributionPersistenceAdapterLikeCpp {
    login_db: Arc<LoginDatabase>,
}

impl MariaDbBattlePayDistributionPersistenceAdapterLikeCpp {
    pub fn new(login_db: Arc<LoginDatabase>) -> Self {
        Self { login_db }
    }

    async fn execute_like_cpp(&self, stmt: PreparedStatement) -> PersistenceOutcomeLikeCpp {
        match self.login_db.execute(&stmt).await {
            Ok(rows) => PersistenceOutcomeLikeCpp::Applied { rows },
            Err(error) => PersistenceOutcomeLikeCpp::Failed {
                reason: error.to_string(),
            },
        }
    }

    /// Whether the order left `Paid` (the witness of both order transactions).
    async fn order_delivered_like_cpp(&self, external_id: &str) -> Result<bool, String> {
        let mut stmt = self
            .login_db
            .prepare(LoginStatements::SEL_BPAY_PURCHASE_STATUS);
        stmt.set_string(0, external_id);
        let result = self
            .login_db
            .query(&stmt)
            .await
            .map_err(|error| error.to_string())?;
        Ok(!result.is_empty() && column_u64_like_cpp(&result, 0) == 2)
    }

    async fn commit_order_transaction_like_cpp(
        &self,
        transaction: SqlTransaction,
        external_id: &str,
    ) -> PersistenceOutcomeLikeCpp {
        let outcome = outcome_from_commit_like_cpp(
            self.login_db
                .commit_transaction_with_outcome_like_cpp(transaction)
                .await,
        );
        let PersistenceOutcomeLikeCpp::Unknown { reason } = outcome else {
            return outcome;
        };
        match self.order_delivered_like_cpp(external_id).await {
            Ok(true) => PersistenceOutcomeLikeCpp::Applied { rows: 0 },
            Ok(false) => PersistenceOutcomeLikeCpp::Failed { reason },
            Err(_) => PersistenceOutcomeLikeCpp::Unknown { reason },
        }
    }
}

impl BattlePayDistributionPersistencePortLikeCpp
    for MariaDbBattlePayDistributionPersistenceAdapterLikeCpp
{
    fn load_distributions_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<BattlePayDistributionRowLikeCpp>, String>> {
        Box::pin(async move {
            let mut stmt = self
                .login_db
                .prepare(LoginStatements::SEL_BPAY_DISTRIBUTIONS);
            stmt.set_u32(0, account_id);
            let result = self
                .login_db
                .query(&stmt)
                .await
                .map_err(|error| error.to_string())?;
            Ok(rows_like_cpp(result, distribution_row_like_cpp))
        })
    }

    fn grant_distribution_like_cpp(
        &self,
        grant: BattlePayDistributionGrantLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let transaction = distribution_grant_transaction_like_cpp(&grant);
            self.commit_order_transaction_like_cpp(transaction, &grant.external_id)
                .await
        })
    }

    fn assign_distribution_like_cpp(
        &self,
        assign: BattlePayDistributionAssignLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let mut stmt = self
                .login_db
                .prepare(LoginStatements::UPD_BPAY_DISTRIBUTION_ASSIGNED);
            stmt.set_u32(0, assign.realm_id);
            stmt.set_u64(1, assign.character_guid);
            stmt.set_u16(2, assign.specialization_id);
            stmt.set_u16(3, assign.choice_id);
            stmt.set_u64(4, assign.distribution_id);
            stmt.set_u32(5, assign.account_id);
            self.execute_like_cpp(stmt).await
        })
    }

    fn unassign_distribution_like_cpp(
        &self,
        distribution_id: u64,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let mut stmt = self
                .login_db
                .prepare(LoginStatements::UPD_BPAY_DISTRIBUTION_UNASSIGN);
            stmt.set_u64(0, distribution_id);
            stmt.set_u32(1, account_id);
            self.execute_like_cpp(stmt).await
        })
    }

    fn load_pending_distribution_like_cpp(
        &self,
        character_guid: u64,
        realm_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayDistributionRowLikeCpp>, String>> {
        Box::pin(async move {
            let mut stmt = self
                .login_db
                .prepare(LoginStatements::SEL_BPAY_DISTRIBUTION_PENDING_BY_CHAR);
            stmt.set_u64(0, character_guid);
            stmt.set_u32(1, realm_id);
            let result = self
                .login_db
                .query(&stmt)
                .await
                .map_err(|error| error.to_string())?;
            Ok(rows_like_cpp(result, distribution_row_like_cpp)
                .into_iter()
                .next())
        })
    }

    fn finish_distribution_like_cpp(
        &self,
        distribution_id: u64,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let mut stmt = self
                .login_db
                .prepare(LoginStatements::UPD_BPAY_DISTRIBUTION_FINISHED);
            stmt.set_u64(0, distribution_id);
            self.execute_like_cpp(stmt).await
        })
    }

    fn grant_undelete_like_cpp(
        &self,
        battlenet_account_id: u32,
        external_id: String,
        web_order_id: String,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let transaction = undelete_grant_transaction_like_cpp(
                battlenet_account_id,
                &external_id,
                &web_order_id,
            );
            self.commit_order_transaction_like_cpp(transaction, &external_id)
                .await
        })
    }

    fn load_account_battlenet_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<u32>, String>> {
        Box::pin(async move {
            let mut stmt = self
                .login_db
                .prepare(LoginStatements::SEL_BPAY_VAS_TRANSFER_TARGET_ACCOUNT);
            stmt.set_u32(0, account_id);
            let result = self
                .login_db
                .query(&stmt)
                .await
                .map_err(|error| error.to_string())?;
            Ok((!result.is_empty()).then(|| column_u64_like_cpp(&result, 0) as u32))
        })
    }

    fn load_bnet_game_accounts_like_cpp(
        &self,
        email: String,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayBnetGameAccountsLikeCpp>, String>>
    {
        Box::pin(async move {
            let mut stmt = self
                .login_db
                .prepare(LoginStatements::SEL_BPAY_VAS_TRANSFER_TARGET_BY_EMAIL);
            stmt.set_string(0, &email);
            let result = self
                .login_db
                .query(&stmt)
                .await
                .map_err(|error| error.to_string())?;
            let rows = rows_like_cpp(result, |row| {
                (
                    column_u64_like_cpp(row, 0) as u32,
                    column_u64_like_cpp(row, 1) as u32,
                    row.try_read::<String>(2).unwrap_or_default(),
                )
            });
            let Some(&(battlenet_account_id, _, _)) = rows.first() else {
                return Ok(None);
            };
            Ok(Some(BattlePayBnetGameAccountsLikeCpp {
                battlenet_account_id,
                // A Battle.net account without game accounts yields one NULL row.
                game_accounts: rows
                    .into_iter()
                    .filter(|(_, account, _)| *account != 0)
                    .map(|(_, account, name)| (account, name))
                    .collect(),
            }))
        })
    }
}

fn character_row_like_cpp(result: &SqlResult) -> BattlePayCharacterRowLikeCpp {
    BattlePayCharacterRowLikeCpp {
        guid: column_u64_like_cpp(result, 0),
        account_id: column_u64_like_cpp(result, 1) as u32,
        name: result.try_read::<String>(2).unwrap_or_default(),
        race: column_u64_like_cpp(result, 3) as u8,
        class: column_u64_like_cpp(result, 4) as u8,
        gender: column_u64_like_cpp(result, 5) as u8,
        level: column_u64_like_cpp(result, 6) as u8,
        at_login_flags: column_u64_like_cpp(result, 7) as u16,
        online: column_u64_like_cpp(result, 8) != 0,
        logout_time: column_u64_like_cpp(result, 9),
        guild_id: column_u64_like_cpp(result, 10),
        guild_leader_guid: column_u64_like_cpp(result, 11),
    }
}

fn receipt_statement_like_cpp(receipt: &BattlePayDeliveryReceiptLikeCpp) -> PreparedStatement {
    let mut insert = PreparedStatement::for_statement(CharStatements::INS_BATTLEPAY_DELIVERY);
    insert.set_string(0, &receipt.external_id);
    insert.set_u32(1, receipt.account_id);
    insert.set_u64(2, receipt.character_guid);
    insert.set_u32(3, receipt.product_id);
    insert
}

/// Receipt + `at_login |= flags` of the owned character.
pub(crate) fn service_delivery_transaction_like_cpp(
    receipt: &BattlePayDeliveryReceiptLikeCpp,
    at_login_flags: u16,
) -> SqlTransaction {
    let mut transaction = SqlTransaction::new();
    transaction.append_expect_rows_affected(receipt_statement_like_cpp(receipt), 1);
    let mut flag =
        PreparedStatement::for_statement(CharStatements::UPD_BATTLEPAY_ADD_AT_LOGIN_FLAG);
    flag.set_u16(0, at_login_flags);
    flag.set_u64(1, receipt.character_guid);
    flag.set_u32(2, receipt.account_id);
    transaction.append_expect_rows_affected(flag, 1);
    transaction
}

/// Receipt + guild membership removal + account move.
pub(crate) fn transfer_delivery_transaction_like_cpp(
    receipt: &BattlePayDeliveryReceiptLikeCpp,
    transfer: &BattlePayCharacterTransferLikeCpp,
) -> SqlTransaction {
    let mut transaction = SqlTransaction::new();
    transaction.append_expect_rows_affected(receipt_statement_like_cpp(receipt), 1);
    let mut guild =
        PreparedStatement::for_statement(CharStatements::DEL_BATTLEPAY_TRANSFER_GUILD_MEMBER);
    guild.set_u64(0, transfer.character_guid);
    transaction.append(guild);
    let mut account =
        PreparedStatement::for_statement(CharStatements::UPD_BATTLEPAY_TRANSFER_ACCOUNT);
    account.set_u32(0, transfer.to_account_id);
    account.set_u16(1, transfer.add_at_login_flags);
    account.set_u64(2, transfer.character_guid);
    account.set_u32(3, transfer.from_account_id);
    transaction.append_expect_rows_affected(account, 1);
    transaction
}

/// Receipt + loadout item rows + money and at-login flag removal of a boost.
pub(crate) fn boost_completion_transaction_like_cpp(
    receipt: &BattlePayDeliveryReceiptLikeCpp,
    completion: &BattlePayBoostCompletionLikeCpp,
    inventory: &[PlayerInventoryPersistenceRequestLikeCpp],
) -> SqlTransaction {
    let mut transaction = InventoryTransactionBuilderLikeCpp::new();
    transaction.append_expect_rows_affected(receipt_statement_like_cpp(receipt), 1);
    for request in inventory {
        append_inventory_request_like_cpp(&mut transaction, request);
    }
    let mut finish =
        PreparedStatement::for_statement(CharStatements::UPD_BATTLEPAY_CHARACTER_BOOST_FINISHED);
    finish.set_u64(0, completion.money);
    finish.set_u16(1, completion.remove_at_login_flags);
    finish.set_u64(2, completion.character_guid);
    transaction.append(finish);
    transaction.finish()
}

pub struct MariaDbBattlePayCharacterServicePersistenceAdapterLikeCpp {
    character_db: Arc<CharacterDatabase>,
}

impl MariaDbBattlePayCharacterServicePersistenceAdapterLikeCpp {
    pub fn new(character_db: Arc<CharacterDatabase>) -> Self {
        Self { character_db }
    }

    async fn receipt_exists_like_cpp(&self, external_id: &str) -> Result<bool, String> {
        let mut stmt = self
            .character_db
            .prepare(CharStatements::SEL_BATTLEPAY_DELIVERY);
        stmt.set_string(0, external_id);
        let result = self
            .character_db
            .query(&stmt)
            .await
            .map_err(|error| error.to_string())?;
        Ok(!result.is_empty())
    }

    async fn commit_receipt_transaction_like_cpp(
        &self,
        transaction: SqlTransaction,
        external_id: &str,
    ) -> PersistenceOutcomeLikeCpp {
        let outcome = outcome_from_commit_like_cpp(
            transaction
                .commit_with_outcome_like_cpp(self.character_db.pool())
                .await,
        );
        let PersistenceOutcomeLikeCpp::Unknown { reason } = outcome else {
            return outcome;
        };
        match self.receipt_exists_like_cpp(external_id).await {
            Ok(true) => PersistenceOutcomeLikeCpp::Applied { rows: 0 },
            Ok(false) => PersistenceOutcomeLikeCpp::Failed { reason },
            Err(_) => PersistenceOutcomeLikeCpp::Unknown { reason },
        }
    }
}

impl BattlePayCharacterServicePersistencePortLikeCpp
    for MariaDbBattlePayCharacterServicePersistenceAdapterLikeCpp
{
    fn load_account_characters_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<BattlePayCharacterRowLikeCpp>, String>> {
        Box::pin(async move {
            let mut stmt = self
                .character_db
                .prepare(CharStatements::SEL_BATTLEPAY_ACCOUNT_CHARACTERS);
            stmt.set_u32(0, account_id);
            let result = self
                .character_db
                .query(&stmt)
                .await
                .map_err(|error| error.to_string())?;
            Ok(rows_like_cpp(result, character_row_like_cpp))
        })
    }

    fn load_character_like_cpp(
        &self,
        character_guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayCharacterRowLikeCpp>, String>> {
        Box::pin(async move {
            let mut stmt = self
                .character_db
                .prepare(CharStatements::SEL_BATTLEPAY_CHARACTER);
            stmt.set_u64(0, character_guid);
            let result = self
                .character_db
                .query(&stmt)
                .await
                .map_err(|error| error.to_string())?;
            Ok(rows_like_cpp(result, character_row_like_cpp)
                .into_iter()
                .next())
        })
    }

    fn persist_service_delivery_like_cpp(
        &self,
        receipt: BattlePayDeliveryReceiptLikeCpp,
        at_login_flags: u16,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let transaction = service_delivery_transaction_like_cpp(&receipt, at_login_flags);
            self.commit_receipt_transaction_like_cpp(transaction, &receipt.external_id)
                .await
        })
    }

    fn persist_transfer_delivery_like_cpp(
        &self,
        receipt: BattlePayDeliveryReceiptLikeCpp,
        transfer: BattlePayCharacterTransferLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let transaction = transfer_delivery_transaction_like_cpp(&receipt, &transfer);
            self.commit_receipt_transaction_like_cpp(transaction, &receipt.external_id)
                .await
        })
    }

    fn queue_character_boost_like_cpp(
        &self,
        character_guid: u64,
        account_id: u32,
        level: u8,
        at_login_flags: u16,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let mut stmt = self
                .character_db
                .prepare(CharStatements::UPD_BATTLEPAY_CHARACTER_BOOST_QUEUED);
            stmt.set_u8(0, level);
            stmt.set_u16(1, at_login_flags);
            stmt.set_u64(2, character_guid);
            stmt.set_u32(3, account_id);
            stmt.set_u8(4, level);
            match self.character_db.execute(&stmt).await {
                Ok(rows) => PersistenceOutcomeLikeCpp::Applied { rows },
                Err(error) => PersistenceOutcomeLikeCpp::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn persist_boost_completion_like_cpp(
        &self,
        receipt: BattlePayDeliveryReceiptLikeCpp,
        completion: BattlePayBoostCompletionLikeCpp,
        inventory: Vec<PlayerInventoryPersistenceRequestLikeCpp>,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let transaction =
                boost_completion_transaction_like_cpp(&receipt, &completion, &inventory);
            self.commit_receipt_transaction_like_cpp(transaction, &receipt.external_id)
                .await
        })
    }
}

#[cfg(test)]
#[path = "battle_pay_services_adapter_tests.rs"]
mod tests;
