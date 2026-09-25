//! Account collection statement selection and persistence-result translation.
//! Private MariaDB implementation; the port remains independent of driver errors.

use crate::statements::StatementDef;

use crate::params::PreparedStatement;
use crate::statements::LoginStatements;
use wow_persistence::{AccountCollectionLoadRequestLikeCpp, AccountLastPlayedCharacterSaveLikeCpp};

/// C++ `Player::SaveToDB` Login-transaction tail
/// (TrinityCore `78bcc3f5` `Player.cpp:20152-20166`): delete the sub-region's
/// row, then insert the current character.
pub(super) fn last_played_character_statements_like_cpp(
    save: &AccountLastPlayedCharacterSaveLikeCpp,
) -> [PreparedStatement; 2] {
    let mut delete =
        PreparedStatement::for_statement(LoginStatements::DEL_BNET_LAST_PLAYER_CHARACTERS);
    delete.set_u32(0, save.account_id);
    delete.set_u8(1, save.region);
    delete.set_u8(2, save.battlegroup);

    let mut insert =
        PreparedStatement::for_statement(LoginStatements::INS_BNET_LAST_PLAYER_CHARACTERS);
    insert.set_u32(0, save.account_id);
    insert.set_u8(1, save.region);
    insert.set_u8(2, save.battlegroup);
    insert.set_u32(3, save.realm_id);
    insert.set_string(4, save.character_name.clone());
    insert.set_u64(5, save.character_guid);
    insert.set_u32(6, save.last_played_time);

    [delete, insert]
}

pub(super) fn account_collection_commit_outcome_like_cpp(
    result: Result<(), crate::SqlTransactionCommitError>,
    rows: u64,
) -> wow_persistence::PersistenceOutcomeLikeCpp {
    use crate::SqlTransactionCommitError;
    use wow_persistence::PersistenceOutcomeLikeCpp;

    match result {
        Ok(()) => PersistenceOutcomeLikeCpp::Applied { rows },
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

#[cfg(test)]
mod tests {
    use super::{
        account_collection_commit_outcome_like_cpp, last_played_character_statements_like_cpp,
    };
    use crate::params::SqlParam;
    use crate::statements::{LoginStatements, StatementDef};
    use wow_persistence::AccountLastPlayedCharacterSaveLikeCpp;

    #[test]
    fn last_played_character_statements_match_cpp_order_and_binds() {
        let [delete, insert] =
            last_played_character_statements_like_cpp(&AccountLastPlayedCharacterSaveLikeCpp {
                account_id: 7,
                region: 1,
                battlegroup: 2,
                realm_id: 3,
                character_name: "Innaa".to_owned(),
                character_guid: 42,
                last_played_time: 1_758_800_000,
            });
        assert_eq!(
            delete.sql(),
            LoginStatements::DEL_BNET_LAST_PLAYER_CHARACTERS.sql()
        );
        assert_eq!(
            delete.params(),
            [SqlParam::U32(7), SqlParam::U8(1), SqlParam::U8(2)]
        );
        assert_eq!(
            insert.sql(),
            LoginStatements::INS_BNET_LAST_PLAYER_CHARACTERS.sql()
        );
        assert_eq!(
            insert.params(),
            [
                SqlParam::U32(7),
                SqlParam::U8(1),
                SqlParam::U8(2),
                SqlParam::U32(3),
                SqlParam::String("Innaa".to_owned()),
                SqlParam::U64(42),
                SqlParam::U32(1_758_800_000),
            ]
        );
    }
    use crate::{DatabaseError, SqlTransactionCommitError};
    use wow_persistence::PersistenceOutcomeLikeCpp;

    #[test]
    fn collection_commit_preserves_confirmation_and_row_count() {
        for rows in [0, 1, 19] {
            assert_eq!(
                account_collection_commit_outcome_like_cpp(Ok(()), rows),
                PersistenceOutcomeLikeCpp::Applied { rows }
            );
        }
    }

    #[test]
    fn collection_commit_preserves_known_rollback() {
        let error = DatabaseError::Transaction("statement rejected".into());
        let reason = error.to_string();
        assert_eq!(
            account_collection_commit_outcome_like_cpp(
                Err(SqlTransactionCommitError::DefinitelyRolledBack(error)),
                19
            ),
            PersistenceOutcomeLikeCpp::Failed { reason }
        );
    }

    #[test]
    fn collection_commit_never_relabels_unknown_as_rollback() {
        let error = DatabaseError::Transaction("COMMIT reply lost".into());
        let reason = error.to_string();
        assert_eq!(
            account_collection_commit_outcome_like_cpp(
                Err(SqlTransactionCommitError::CommitOutcomeUnknown(error)),
                19
            ),
            PersistenceOutcomeLikeCpp::Unknown { reason }
        );
    }
}

pub(super) fn account_collection_load_statements_like_cpp(
    request: AccountCollectionLoadRequestLikeCpp,
) -> Vec<PreparedStatement> {
    let (bnet_account_id, statements) = match request {
        AccountCollectionLoadRequestLikeCpp::Mounts { bnet_account_id } => {
            (bnet_account_id, vec![LoginStatements::SEL_ACCOUNT_MOUNTS])
        }
        AccountCollectionLoadRequestLikeCpp::Toys { bnet_account_id } => {
            (bnet_account_id, vec![LoginStatements::SEL_ACCOUNT_TOYS])
        }
        AccountCollectionLoadRequestLikeCpp::Heirlooms { bnet_account_id } => (
            bnet_account_id,
            vec![LoginStatements::SEL_ACCOUNT_HEIRLOOMS],
        ),
        AccountCollectionLoadRequestLikeCpp::ItemAppearances { bnet_account_id } => (
            bnet_account_id,
            vec![
                LoginStatements::SEL_BNET_ITEM_APPEARANCES,
                LoginStatements::SEL_BNET_ITEM_FAVORITE_APPEARANCES,
            ],
        ),
        AccountCollectionLoadRequestLikeCpp::TransmogIllusions { bnet_account_id } => (
            bnet_account_id,
            vec![LoginStatements::SEL_BNET_TRANSMOG_ILLUSIONS],
        ),
    };

    statements
        .into_iter()
        .map(|statement| {
            let mut prepared = PreparedStatement::new(statement.sql());
            prepared.set_u32(0, bnet_account_id);
            prepared
        })
        .collect()
}
