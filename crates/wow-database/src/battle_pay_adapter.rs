//! MariaDB adapters for the in-game shop (BattlePay) account and delivery ports.
//!
//! Login side: LegionCore `Player::ChangeTokenCount` (`LOGIN_INS_OR_UPD_TOKEN` +
//! `LOGIN_INS_LOG_USE_DONATE_TOKEN` in one transaction) and the `LOGIN_*_BPAY_PURCHASE*`
//! statements of `BattlePayHandler.cpp`. RustyCore makes the wallet debit
//! conditional (`amount >= price`) and records the paid order in the same
//! transaction, so a debit without an owed delivery cannot exist.
//!
//! Character side: the delivery receipt (`character_battlepay_delivery`, keyed by
//! the order's `external_id`) commits with the delivered item rows, so a replayed
//! delivery finds the receipt instead of granting twice.
//!
//! Both sides resolve an unknown COMMIT outcome by reading their own witness row
//! (the order row, the receipt row), like the quest-reward adapter does.

use std::sync::Arc;

use wow_persistence::{
    BattlePayAccountPersistencePortLikeCpp, BattlePayDeliveryPersistencePortLikeCpp,
    BattlePayDeliveryReceiptLikeCpp, BattlePayPurchaseInsertLikeCpp, BattlePayPurchaseRowLikeCpp,
    BattlePaySsoTokenIssueLikeCpp, BattlePayTokenChargeLikeCpp, BattlePayTokenChargeOutcomeLikeCpp,
    PersistenceFutureLikeCpp, PersistenceOutcomeLikeCpp, PlayerInventoryPersistenceRequestLikeCpp,
};

use crate::player::inventory_adapter::{
    InventoryTransactionBuilderLikeCpp, append_inventory_request_like_cpp,
};
use crate::result::SqlResult;
use crate::web_token::{WEB_TOKEN_PROGRAM_WOW, WebTokenIssue, WebTokenKind};
use crate::{
    CharStatements, CharacterDatabase, LoginDatabase, LoginStatements, PreparedStatement,
    SqlTransaction, SqlTransactionCommitError,
};

/// Read an integer column whatever its declared width/signedness.
pub(crate) fn column_u64_like_cpp(result: &SqlResult, column: usize) -> u64 {
    result
        .try_read::<u64>(column)
        .or_else(|| result.try_read::<u32>(column).map(u64::from))
        .or_else(|| result.try_read::<u16>(column).map(u64::from))
        .or_else(|| result.try_read::<u8>(column).map(u64::from))
        .or_else(|| result.try_read::<i64>(column).map(|v| v.max(0) as u64))
        .or_else(|| result.try_read::<i32>(column).map(|v| v.max(0) as u64))
        .or_else(|| result.try_read::<i16>(column).map(|v| v.max(0) as u64))
        .or_else(|| result.try_read::<i8>(column).map(|v| v.max(0) as u64))
        .unwrap_or(0)
}

/// Signed variant of [`column_u64_like_cpp`].
pub(crate) fn column_i64_like_cpp(result: &SqlResult, column: usize) -> i64 {
    result
        .try_read::<i64>(column)
        .or_else(|| result.try_read::<i32>(column).map(i64::from))
        .or_else(|| result.try_read::<i16>(column).map(i64::from))
        .or_else(|| result.try_read::<i8>(column).map(i64::from))
        .or_else(|| {
            result
                .try_read::<u64>(column)
                .map(|v| v.min(i64::MAX as u64) as i64)
        })
        .or_else(|| result.try_read::<u32>(column).map(i64::from))
        .or_else(|| result.try_read::<u16>(column).map(i64::from))
        .or_else(|| result.try_read::<u8>(column).map(i64::from))
        .unwrap_or(0)
}

fn purchase_insert_statement_like_cpp(
    statement: LoginStatements,
    purchase: &BattlePayPurchaseInsertLikeCpp,
) -> PreparedStatement {
    let mut stmt = PreparedStatement::for_statement(statement);
    stmt.set_string(0, &purchase.external_id);
    stmt.set_string(1, &purchase.signature);
    stmt.set_u32(2, purchase.battlenet_account_id);
    stmt.set_u32(3, purchase.account_id);
    stmt.set_u32(4, purchase.realm_id);
    stmt.set_u64(5, purchase.character_guid);
    stmt.set_u32(6, purchase.product_id);
    stmt.set_string(7, &purchase.price);
    stmt.set_string(8, &purchase.currency);
    stmt.set_string(9, &purchase.ip);
    stmt.set_string(10, &purchase.payment_ref);
    stmt
}

/// The one Login DB transaction of a wallet purchase: guarded debit (skipped for a
/// free product), token log row, paid order row.
pub(crate) fn token_charge_transaction_like_cpp(
    charge: &BattlePayTokenChargeLikeCpp,
) -> SqlTransaction {
    let purchase = &charge.purchase;
    let mut transaction = SqlTransaction::new();
    if charge.amount > 0 {
        let mut spend = PreparedStatement::for_statement(LoginStatements::UPD_ACCOUNT_TOKEN_SPEND);
        spend.set_i64(0, charge.amount);
        spend.set_u32(1, purchase.account_id);
        spend.set_u8(2, charge.token_type);
        spend.set_i64(3, charge.amount);
        transaction.append_expect_rows_affected(spend, 1);
    }
    let mut log = PreparedStatement::for_statement(LoginStatements::INS_ACCOUNT_DONATE_TOKEN_LOG);
    log.set_u32(0, purchase.account_id);
    log.set_u32(1, purchase.realm_id);
    log.set_u64(2, purchase.character_guid);
    log.set_i64(3, -charge.amount);
    log.set_u8(4, charge.token_type);
    log.set_u8(5, charge.buy_type);
    log.set_u32(6, purchase.product_id);
    transaction.append(log);
    transaction.append_expect_rows_affected(
        purchase_insert_statement_like_cpp(LoginStatements::INS_BPAY_PURCHASE_PAID, purchase),
        1,
    );
    transaction
}

fn purchase_row_like_cpp(result: &SqlResult) -> BattlePayPurchaseRowLikeCpp {
    BattlePayPurchaseRowLikeCpp {
        id: column_u64_like_cpp(result, 0),
        external_id: result.read_string(1),
        product_id: column_u64_like_cpp(result, 2) as u32,
        status: column_u64_like_cpp(result, 3) as u8,
        character_guid: column_u64_like_cpp(result, 4),
        payment_ref: result.read_string(5),
        web_order_id: result.read_string(6),
    }
}

fn purchase_rows_like_cpp(mut result: SqlResult) -> Vec<BattlePayPurchaseRowLikeCpp> {
    let mut rows = Vec::with_capacity(result.count());
    if result.is_empty() {
        return rows;
    }
    loop {
        rows.push(purchase_row_like_cpp(&result));
        if !result.next_row() {
            break;
        }
    }
    rows
}

fn outcome_from_commit_like_cpp(
    result: Result<(), SqlTransactionCommitError>,
) -> PersistenceOutcomeLikeCpp {
    match result {
        Ok(()) => PersistenceOutcomeLikeCpp::Applied { rows: 0 },
        Err(SqlTransactionCommitError::DefinitelyRolledBack(error)) => {
            PersistenceOutcomeLikeCpp::Failed {
                reason: error.to_string(),
            }
        }
        Err(SqlTransactionCommitError::CommitOutcomeUnknown(error)) => {
            PersistenceOutcomeLikeCpp::Unknown {
                reason: error.to_string(),
            }
        }
    }
}

pub struct MariaDbBattlePayAccountPersistenceAdapterLikeCpp {
    login_db: Arc<LoginDatabase>,
}

impl MariaDbBattlePayAccountPersistenceAdapterLikeCpp {
    pub fn new(login_db: Arc<LoginDatabase>) -> Self {
        Self { login_db }
    }

    async fn token_balance_like_cpp(&self, account_id: u32, token_type: u8) -> Result<i64, String> {
        let balances = self.token_balances_like_cpp(account_id).await?;
        Ok(balances
            .into_iter()
            .find(|(kind, _)| *kind == token_type)
            .map(|(_, amount)| amount)
            .unwrap_or(0))
    }

    async fn token_balances_like_cpp(&self, account_id: u32) -> Result<Vec<(u8, i64)>, String> {
        let mut stmt = self.login_db.prepare(LoginStatements::SEL_ACCOUNT_TOKENS);
        stmt.set_u32(0, account_id);
        let mut result = self
            .login_db
            .query(&stmt)
            .await
            .map_err(|error| error.to_string())?;
        let mut balances = Vec::with_capacity(result.count());
        if result.is_empty() {
            return Ok(balances);
        }
        loop {
            balances.push((
                column_u64_like_cpp(&result, 0) as u8,
                column_i64_like_cpp(&result, 1),
            ));
            if !result.next_row() {
                break;
            }
        }
        Ok(balances)
    }

    async fn purchase_like_cpp(
        &self,
        external_id: &str,
        account_id: u32,
    ) -> Result<Option<BattlePayPurchaseRowLikeCpp>, String> {
        let mut stmt = self
            .login_db
            .prepare(LoginStatements::SEL_BPAY_PURCHASE_BY_EXTERNAL_ID);
        stmt.set_string(0, external_id);
        stmt.set_u32(1, account_id);
        let result = self
            .login_db
            .query(&stmt)
            .await
            .map_err(|error| error.to_string())?;
        Ok(purchase_rows_like_cpp(result).into_iter().next())
    }
}

impl BattlePayAccountPersistencePortLikeCpp for MariaDbBattlePayAccountPersistenceAdapterLikeCpp {
    fn load_token_balances_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<(u8, i64)>, String>> {
        Box::pin(async move { self.token_balances_like_cpp(account_id).await })
    }

    fn charge_tokens_like_cpp(
        &self,
        charge: BattlePayTokenChargeLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, BattlePayTokenChargeOutcomeLikeCpp> {
        Box::pin(async move {
            let account_id = charge.purchase.account_id;
            match self
                .token_balance_like_cpp(account_id, charge.token_type)
                .await
            {
                Ok(balance) if balance < charge.amount => {
                    return BattlePayTokenChargeOutcomeLikeCpp::InsufficientBalance { balance };
                }
                Ok(_) => {}
                Err(reason) => return BattlePayTokenChargeOutcomeLikeCpp::Failed { reason },
            }
            let transaction = token_charge_transaction_like_cpp(&charge);
            match self
                .login_db
                .commit_transaction_with_outcome_like_cpp(transaction)
                .await
            {
                Ok(()) => BattlePayTokenChargeOutcomeLikeCpp::Charged,
                Err(SqlTransactionCommitError::DefinitelyRolledBack(error)) => {
                    // The guarded debit matched no row: a concurrent spend won.
                    match self
                        .token_balance_like_cpp(account_id, charge.token_type)
                        .await
                    {
                        Ok(balance) if balance < charge.amount => {
                            BattlePayTokenChargeOutcomeLikeCpp::InsufficientBalance { balance }
                        }
                        _ => BattlePayTokenChargeOutcomeLikeCpp::Failed {
                            reason: error.to_string(),
                        },
                    }
                }
                Err(SqlTransactionCommitError::CommitOutcomeUnknown(error)) => {
                    // The paid order row is the witness of the whole transaction.
                    match self
                        .purchase_like_cpp(&charge.purchase.external_id, account_id)
                        .await
                    {
                        Ok(Some(_)) => BattlePayTokenChargeOutcomeLikeCpp::Charged,
                        Ok(None) => BattlePayTokenChargeOutcomeLikeCpp::Failed {
                            reason: error.to_string(),
                        },
                        Err(_) => BattlePayTokenChargeOutcomeLikeCpp::Unknown {
                            reason: error.to_string(),
                        },
                    }
                }
            }
        })
    }

    fn insert_web_purchase_like_cpp(
        &self,
        purchase: BattlePayPurchaseInsertLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let stmt =
                purchase_insert_statement_like_cpp(LoginStatements::INS_BPAY_PURCHASE, &purchase);
            match self.login_db.execute(&stmt).await {
                Ok(rows) => PersistenceOutcomeLikeCpp::Applied { rows },
                Err(error) => PersistenceOutcomeLikeCpp::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn load_purchase_like_cpp(
        &self,
        external_id: String,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BattlePayPurchaseRowLikeCpp>, String>> {
        Box::pin(async move { self.purchase_like_cpp(&external_id, account_id).await })
    }

    fn load_paid_purchases_like_cpp(
        &self,
        account_id: u32,
        realm_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, Result<Vec<BattlePayPurchaseRowLikeCpp>, String>> {
        Box::pin(async move {
            let mut stmt = self
                .login_db
                .prepare(LoginStatements::SEL_BPAY_PURCHASES_PAID);
            stmt.set_u32(0, account_id);
            stmt.set_u32(1, realm_id);
            let result = self
                .login_db
                .query(&stmt)
                .await
                .map_err(|error| error.to_string())?;
            Ok(purchase_rows_like_cpp(result))
        })
    }

    fn mark_purchase_delivered_like_cpp(
        &self,
        external_id: String,
        web_order_id: String,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let mut stmt = self
                .login_db
                .prepare(LoginStatements::UPD_BPAY_PURCHASE_DELIVERED);
            stmt.set_string(0, &web_order_id);
            stmt.set_string(1, &external_id);
            match self.login_db.execute(&stmt).await {
                Ok(rows) => PersistenceOutcomeLikeCpp::Applied { rows },
                Err(error) => PersistenceOutcomeLikeCpp::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn mark_purchase_failed_like_cpp(
        &self,
        external_id: String,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let mut stmt = self
                .login_db
                .prepare(LoginStatements::UPD_BPAY_PURCHASE_FAILED);
            stmt.set_string(0, &external_id);
            stmt.set_u32(1, account_id);
            match self.login_db.execute(&stmt).await {
                Ok(rows) => PersistenceOutcomeLikeCpp::Applied { rows },
                Err(error) => PersistenceOutcomeLikeCpp::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn issue_sso_token_like_cpp(
        &self,
        issue: BattlePaySsoTokenIssueLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, Result<String, String>> {
        Box::pin(async move {
            let row = WebTokenIssue {
                battlenet_account: issue.battlenet_account_id,
                account: issue.account_id,
                realm: issue.realm_id,
                character_guid: issue.character_guid,
                program: WEB_TOKEN_PROGRAM_WOW,
                kind: WebTokenKind::Sso,
                ip: issue.ip,
                lifetime_secs: issue.lifetime_secs,
            };
            crate::web_token::issue_web_token_from_bytes(&self.login_db, &row, &issue.random_bytes)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

/// Receipt plus item rows of one delivery, receipt first.
pub(crate) fn delivery_transaction_like_cpp(
    receipt: &BattlePayDeliveryReceiptLikeCpp,
    inventory: &[PlayerInventoryPersistenceRequestLikeCpp],
) -> SqlTransaction {
    let mut transaction = InventoryTransactionBuilderLikeCpp::new();
    let mut insert = PreparedStatement::for_statement(CharStatements::INS_BATTLEPAY_DELIVERY);
    insert.set_string(0, &receipt.external_id);
    insert.set_u32(1, receipt.account_id);
    insert.set_u64(2, receipt.character_guid);
    insert.set_u32(3, receipt.product_id);
    transaction.append_expect_rows_affected(insert, 1);
    for request in inventory {
        append_inventory_request_like_cpp(&mut transaction, request);
    }
    transaction.finish()
}

pub struct MariaDbBattlePayDeliveryPersistenceAdapterLikeCpp {
    character_db: Arc<CharacterDatabase>,
}

impl MariaDbBattlePayDeliveryPersistenceAdapterLikeCpp {
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
}

impl BattlePayDeliveryPersistencePortLikeCpp for MariaDbBattlePayDeliveryPersistenceAdapterLikeCpp {
    fn delivery_receipt_exists_like_cpp(
        &self,
        external_id: String,
    ) -> PersistenceFutureLikeCpp<'_, Result<bool, String>> {
        Box::pin(async move { self.receipt_exists_like_cpp(&external_id).await })
    }

    fn persist_delivery_like_cpp(
        &self,
        receipt: BattlePayDeliveryReceiptLikeCpp,
        inventory: Vec<PlayerInventoryPersistenceRequestLikeCpp>,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let transaction = delivery_transaction_like_cpp(&receipt, &inventory);
            let outcome = outcome_from_commit_like_cpp(
                transaction
                    .commit_with_outcome_like_cpp(self.character_db.pool())
                    .await,
            );
            let PersistenceOutcomeLikeCpp::Unknown { reason } = outcome else {
                return outcome;
            };
            // The receipt commits with the items: its presence decides the outcome.
            match self.receipt_exists_like_cpp(&receipt.external_id).await {
                Ok(true) => PersistenceOutcomeLikeCpp::Applied { rows: 0 },
                Ok(false) => PersistenceOutcomeLikeCpp::Failed { reason },
                Err(_) => PersistenceOutcomeLikeCpp::Unknown { reason },
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StatementDef;

    fn charge(amount: i64) -> BattlePayTokenChargeLikeCpp {
        BattlePayTokenChargeLikeCpp {
            token_type: 1,
            amount,
            buy_type: 0,
            purchase: BattlePayPurchaseInsertLikeCpp {
                external_id: "E".repeat(32),
                signature: "S".repeat(32),
                battlenet_account_id: 2,
                account_id: 3,
                realm_id: 1,
                character_guid: 9,
                product_id: 7,
                price: "15".into(),
                currency: "TOK".into(),
                ip: "127.0.0.1".into(),
                payment_ref: "tokens:1".into(),
            },
        }
    }

    #[test]
    fn wallet_debit_is_guarded_and_commits_with_log_and_paid_order() {
        let transaction = token_charge_transaction_like_cpp(&charge(15));
        assert_eq!(transaction.len(), 3);
        assert_eq!(
            LoginStatements::UPD_ACCOUNT_TOKEN_SPEND.sql(),
            "UPDATE account_tokens SET amount = amount - ? WHERE account_id = ? AND tokenType = ? AND amount >= ?"
        );
        assert!(
            LoginStatements::INS_BPAY_PURCHASE_PAID
                .sql()
                .contains("status, paid")
        );
        assert!(
            LoginStatements::INS_BPAY_PURCHASE_PAID
                .sql()
                .ends_with("1, NOW())")
        );
    }

    #[test]
    fn free_product_skips_the_wallet_debit() {
        assert_eq!(token_charge_transaction_like_cpp(&charge(0)).len(), 2);
    }

    #[test]
    fn delivery_receipt_is_the_first_statement_of_the_item_transaction() {
        let receipt = BattlePayDeliveryReceiptLikeCpp {
            external_id: "E".repeat(32),
            account_id: 3,
            character_guid: 9,
            product_id: 7,
        };
        let transaction = delivery_transaction_like_cpp(&receipt, &[]);
        assert_eq!(transaction.len(), 1);
        assert!(
            CharStatements::INS_BATTLEPAY_DELIVERY
                .sql()
                .starts_with("INSERT INTO character_battlepay_delivery")
        );
    }

    #[test]
    fn delivered_and_failed_transitions_only_move_the_expected_status() {
        assert!(
            LoginStatements::UPD_BPAY_PURCHASE_DELIVERED
                .sql()
                .ends_with("WHERE external_id = ? AND status = 1")
        );
        assert!(
            LoginStatements::UPD_BPAY_PURCHASE_FAILED
                .sql()
                .ends_with("AND status = 0")
        );
        assert!(
            LoginStatements::SEL_BPAY_PURCHASES_PAID
                .sql()
                .contains("WHERE account = ? AND realm = ? AND status = 1")
        );
    }
}
