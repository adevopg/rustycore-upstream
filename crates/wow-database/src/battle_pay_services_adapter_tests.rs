use super::*;
use crate::StatementDef;

fn receipt() -> BattlePayDeliveryReceiptLikeCpp {
    BattlePayDeliveryReceiptLikeCpp {
        external_id: "E".repeat(32),
        account_id: 3,
        character_guid: 9,
        product_id: 7,
    }
}

#[test]
fn distribution_grant_consumes_the_paid_order_in_one_transaction() {
    let transaction = distribution_grant_transaction_like_cpp(&BattlePayDistributionGrantLikeCpp {
        distribution_id: 42,
        external_id: "E".repeat(32),
        web_order_id: String::new(),
    });
    assert_eq!(transaction.len(), 2);
    let insert = LoginStatements::INS_BPAY_DISTRIBUTION_FROM_PURCHASE.sql();
    assert!(insert.starts_with("INSERT INTO battlepay_distribution"));
    assert!(insert.ends_with("WHERE external_id = ? AND status = 1"));
}

#[test]
fn undelete_grant_resets_the_cooldown_with_the_order_transition() {
    assert_eq!(undelete_grant_transaction_like_cpp(1, "E", "").len(), 2);
    assert_eq!(
        LoginStatements::UPD_BPAY_RESET_UNDELETE_COOLDOWN.sql(),
        "UPDATE battlenet_accounts SET LastCharacterUndelete = 0 WHERE id = ?"
    );
}

#[test]
fn distribution_transitions_match_legioncore() {
    assert!(
        LoginStatements::UPD_BPAY_DISTRIBUTION_ASSIGNED
            .sql()
            .ends_with("WHERE id = ? AND account = ? AND status = 1 AND revoked = 0")
    );
    assert!(
        LoginStatements::UPD_BPAY_DISTRIBUTION_FINISHED
            .sql()
            .ends_with("WHERE id = ? AND status = 2")
    );
    assert!(
        LoginStatements::SEL_BPAY_DISTRIBUTIONS
            .sql()
            .contains("WHERE account = ? AND (status < 4 OR revoked = 1)")
    );
}

#[test]
fn service_delivery_commits_the_receipt_with_the_owned_at_login_flag() {
    let transaction = service_delivery_transaction_like_cpp(&receipt(), 0x40);
    assert_eq!(transaction.len(), 2);
    assert_eq!(
        CharStatements::UPD_BATTLEPAY_ADD_AT_LOGIN_FLAG.sql(),
        "UPDATE characters SET at_login = at_login | ? WHERE guid = ? AND account = ?"
    );
}

#[test]
fn transfer_moves_an_offline_character_and_drops_its_guild_membership() {
    let transaction = transfer_delivery_transaction_like_cpp(
        &receipt(),
        &BattlePayCharacterTransferLikeCpp {
            character_guid: 9,
            from_account_id: 3,
            to_account_id: 4,
            add_at_login_flags: 0,
        },
    );
    assert_eq!(transaction.len(), 3);
    assert!(
        CharStatements::UPD_BATTLEPAY_TRANSFER_ACCOUNT
            .sql()
            .ends_with("WHERE guid = ? AND account = ? AND online = 0")
    );
}

#[test]
fn boost_queue_matches_legioncore_guards() {
    assert!(
        CharStatements::UPD_BATTLEPAY_CHARACTER_BOOST_QUEUED
            .sql()
            .ends_with("AND online = 0 AND deleteInfos_Account IS NULL AND level < ?")
    );
    let transaction = boost_completion_transaction_like_cpp(
        &receipt(),
        &BattlePayBoostCompletionLikeCpp {
            character_guid: 9,
            remove_at_login_flags: 0x400,
            money: 5_000_000,
        },
        &[],
    );
    assert_eq!(transaction.len(), 2);
}
