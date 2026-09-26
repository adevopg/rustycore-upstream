//! MariaDB adapter of the worldserver Battle.net friends manager.
//!
//! LegionCore 7.3.5 `Battlenet::FriendsMgr` statements over the auth tables
//! `battlenet_account_friends` / `battlenet_account_friend_invitations` /
//! `battlenet_accounts.battle_tag` (`sql/updates/auth/wotlk_classic/2026_09_26_00_auth.sql`).
//! Friendship changes commit both directions in one Login DB transaction; an
//! unknown COMMIT outcome is reported as `Unknown` so the manager keeps its
//! in-memory state unchanged instead of guessing.

use std::sync::Arc;

use wow_persistence::{
    BnetAccountIdentityLikeCpp, BnetAccountLookupLikeCpp, BnetFriendInvitationRowLikeCpp,
    BnetFriendLinkRowLikeCpp, BnetFriendsLoadLikeCpp, BnetFriendsPersistencePortLikeCpp,
    PersistenceFutureLikeCpp, PersistenceOutcomeLikeCpp,
};

use crate::battle_pay_adapter::{column_u64_like_cpp, outcome_from_commit_like_cpp};
use crate::result::SqlResult;
use crate::{LoginDatabase, LoginStatements, PreparedStatement, SqlTransaction};

fn column_string_like_cpp(result: &SqlResult, column: usize) -> String {
    result.try_read::<String>(column).unwrap_or_default()
}

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

fn identity_like_cpp(result: &SqlResult) -> BnetAccountIdentityLikeCpp {
    BnetAccountIdentityLikeCpp {
        account_id: column_u64_like_cpp(result, 0) as u32,
        email: column_string_like_cpp(result, 1),
        battle_tag: column_string_like_cpp(result, 2),
    }
}

fn link_like_cpp(result: &SqlResult) -> BnetFriendLinkRowLikeCpp {
    BnetFriendLinkRowLikeCpp {
        account_id: column_u64_like_cpp(result, 0) as u32,
        friend_id: column_u64_like_cpp(result, 1) as u32,
        note: column_string_like_cpp(result, 2),
        role: column_u64_like_cpp(result, 3) as u32,
    }
}

fn invitation_like_cpp(result: &SqlResult) -> BnetFriendInvitationRowLikeCpp {
    BnetFriendInvitationRowLikeCpp {
        id: column_u64_like_cpp(result, 0),
        inviter_id: column_u64_like_cpp(result, 1) as u32,
        invitee_id: column_u64_like_cpp(result, 2) as u32,
        message: column_string_like_cpp(result, 3),
        created: column_u64_like_cpp(result, 4),
        role: column_u64_like_cpp(result, 5) as u32,
    }
}

fn insert_friend_statement_like_cpp(
    account_id: u32,
    friend_id: u32,
    role: u32,
) -> PreparedStatement {
    let mut insert = PreparedStatement::for_statement(LoginStatements::INS_BNET_FRIEND);
    insert.set_u32(0, account_id);
    insert.set_u32(1, friend_id);
    insert.set_string(2, "");
    insert.set_u32(3, role);
    insert
}

fn delete_friend_statement_like_cpp(account_id: u32, friend_id: u32) -> PreparedStatement {
    let mut delete = PreparedStatement::for_statement(LoginStatements::DEL_BNET_FRIEND);
    delete.set_u32(0, account_id);
    delete.set_u32(1, friend_id);
    delete
}

fn delete_invitation_statement_like_cpp(invitation_id: u64) -> PreparedStatement {
    let mut delete = PreparedStatement::for_statement(LoginStatements::DEL_BNET_FRIEND_INVITATION);
    delete.set_u64(0, invitation_id);
    delete
}

/// Accept: `inviter -> invitee`, `invitee -> inviter`, invitation consumed.
pub(crate) fn accept_invitation_transaction_like_cpp(
    invitation: &BnetFriendInvitationRowLikeCpp,
) -> SqlTransaction {
    let mut transaction = SqlTransaction::new();
    transaction.append_expect_rows_affected(
        insert_friend_statement_like_cpp(
            invitation.inviter_id,
            invitation.invitee_id,
            invitation.role,
        ),
        1,
    );
    transaction.append_expect_rows_affected(
        insert_friend_statement_like_cpp(
            invitation.invitee_id,
            invitation.inviter_id,
            invitation.role,
        ),
        1,
    );
    transaction.append_expect_rows_affected(delete_invitation_statement_like_cpp(invitation.id), 1);
    transaction
}

/// Remove: both directions.
pub(crate) fn delete_friendship_transaction_like_cpp(
    account_id: u32,
    friend_id: u32,
) -> SqlTransaction {
    let mut transaction = SqlTransaction::new();
    transaction.append(delete_friend_statement_like_cpp(account_id, friend_id));
    transaction.append(delete_friend_statement_like_cpp(friend_id, account_id));
    transaction
}

pub(crate) fn insert_invitation_statement_like_cpp(
    invitation: &BnetFriendInvitationRowLikeCpp,
) -> PreparedStatement {
    let mut insert = PreparedStatement::for_statement(LoginStatements::INS_BNET_FRIEND_INVITATION);
    insert.set_u64(0, invitation.id);
    insert.set_u32(1, invitation.inviter_id);
    insert.set_u32(2, invitation.invitee_id);
    insert.set_string(3, invitation.message.clone());
    insert.set_u64(4, invitation.created);
    insert.set_u32(5, invitation.role);
    insert
}

pub struct MariaDbBnetFriendsPersistenceAdapterLikeCpp {
    login_db: Arc<LoginDatabase>,
}

impl MariaDbBnetFriendsPersistenceAdapterLikeCpp {
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

    async fn commit_like_cpp(&self, transaction: SqlTransaction) -> PersistenceOutcomeLikeCpp {
        outcome_from_commit_like_cpp(
            self.login_db
                .commit_transaction_with_outcome_like_cpp(transaction)
                .await,
        )
    }

    async fn query_rows_like_cpp<T>(
        &self,
        stmt: PreparedStatement,
        decode: impl Fn(&SqlResult) -> T,
    ) -> Result<Vec<T>, String> {
        let result = self
            .login_db
            .query(&stmt)
            .await
            .map_err(|error| error.to_string())?;
        Ok(rows_like_cpp(result, decode))
    }
}

impl BnetFriendsPersistencePortLikeCpp for MariaDbBnetFriendsPersistenceAdapterLikeCpp {
    fn load_all_like_cpp(
        &self,
    ) -> PersistenceFutureLikeCpp<'_, Result<BnetFriendsLoadLikeCpp, String>> {
        Box::pin(async move {
            let accounts = self
                .query_rows_like_cpp(
                    self.login_db
                        .prepare(LoginStatements::SEL_BNET_ACCOUNT_IDENTITIES_ALL),
                    identity_like_cpp,
                )
                .await?;
            let links = self
                .query_rows_like_cpp(
                    self.login_db.prepare(LoginStatements::SEL_BNET_FRIENDS_ALL),
                    link_like_cpp,
                )
                .await?;
            let invitations = self
                .query_rows_like_cpp(
                    self.login_db
                        .prepare(LoginStatements::SEL_BNET_FRIEND_INVITATIONS_ALL),
                    invitation_like_cpp,
                )
                .await?;
            Ok(BnetFriendsLoadLikeCpp {
                accounts,
                links,
                invitations,
            })
        })
    }

    fn find_account_like_cpp(
        &self,
        lookup: BnetAccountLookupLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BnetAccountIdentityLikeCpp>, String>> {
        Box::pin(async move {
            let stmt = match lookup {
                BnetAccountLookupLikeCpp::Id(account_id) => {
                    let mut stmt = self
                        .login_db
                        .prepare(LoginStatements::SEL_BNET_ACCOUNT_IDENTITY_BY_ID);
                    stmt.set_u32(0, account_id);
                    stmt
                }
                BnetAccountLookupLikeCpp::BattleTag(battle_tag) => {
                    let mut stmt = self
                        .login_db
                        .prepare(LoginStatements::SEL_BNET_ACCOUNT_IDENTITY_BY_BATTLE_TAG);
                    stmt.set_string(0, battle_tag);
                    stmt
                }
                BnetAccountLookupLikeCpp::Email(email) => {
                    let mut stmt = self
                        .login_db
                        .prepare(LoginStatements::SEL_BNET_ACCOUNT_IDENTITY_BY_EMAIL);
                    stmt.set_string(0, email);
                    stmt
                }
            };
            let rows = self.query_rows_like_cpp(stmt, identity_like_cpp).await?;
            Ok(rows.into_iter().next())
        })
    }

    fn insert_invitation_like_cpp(
        &self,
        invitation: BnetFriendInvitationRowLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            self.execute_like_cpp(insert_invitation_statement_like_cpp(&invitation))
                .await
        })
    }

    fn delete_invitation_like_cpp(
        &self,
        invitation_id: u64,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            self.execute_like_cpp(delete_invitation_statement_like_cpp(invitation_id))
                .await
        })
    }

    fn accept_invitation_like_cpp(
        &self,
        invitation: BnetFriendInvitationRowLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            self.commit_like_cpp(accept_invitation_transaction_like_cpp(&invitation))
                .await
        })
    }

    fn delete_friendship_like_cpp(
        &self,
        account_id: u32,
        friend_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            self.commit_like_cpp(delete_friendship_transaction_like_cpp(
                account_id, friend_id,
            ))
            .await
        })
    }

    fn update_friend_note_like_cpp(
        &self,
        account_id: u32,
        friend_id: u32,
        note: String,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        Box::pin(async move {
            let mut update =
                PreparedStatement::for_statement(LoginStatements::UPD_BNET_FRIEND_NOTE);
            update.set_string(0, note);
            update.set_u32(1, account_id);
            update.set_u32(2, friend_id);
            self.execute_like_cpp(update).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StatementDef;

    fn invitation() -> BnetFriendInvitationRowLikeCpp {
        BnetFriendInvitationRowLikeCpp {
            id: 42,
            inviter_id: 3,
            invitee_id: 9,
            message: String::new(),
            created: 1_700_000_000,
            role: 1,
        }
    }

    #[test]
    fn accept_writes_both_directions_and_consumes_the_invitation_in_one_transaction() {
        let transaction = accept_invitation_transaction_like_cpp(&invitation());
        assert_eq!(transaction.len(), 3);
        assert_eq!(
            LoginStatements::INS_BNET_FRIEND.sql(),
            "INSERT INTO battlenet_account_friends (account_id, friend_id, note, role) VALUES (?, ?, ?, ?)"
        );
        assert_eq!(
            LoginStatements::DEL_BNET_FRIEND_INVITATION.sql(),
            "DELETE FROM battlenet_account_friend_invitations WHERE id = ?"
        );
    }

    #[test]
    fn remove_friend_deletes_both_directions_in_one_transaction() {
        assert_eq!(delete_friendship_transaction_like_cpp(3, 9).len(), 2);
        assert_eq!(
            LoginStatements::DEL_BNET_FRIEND.sql(),
            "DELETE FROM battlenet_account_friends WHERE account_id = ? AND friend_id = ?"
        );
    }

    #[test]
    fn statements_target_the_legioncore_tables() {
        assert!(LoginStatements::SEL_BNET_FRIENDS_ALL.sql().starts_with(
            "SELECT account_id, friend_id, note, role FROM battlenet_account_friends"
        ));
        assert!(
            LoginStatements::SEL_BNET_FRIEND_INVITATIONS_ALL
                .sql()
                .contains("UNIX_TIMESTAMP(created)")
        );
        assert!(
            LoginStatements::INS_BNET_FRIEND_INVITATION
                .sql()
                .ends_with("VALUES (?, ?, ?, ?, FROM_UNIXTIME(?), ?)")
        );
        assert_eq!(
            LoginStatements::UPD_BNET_FRIEND_NOTE.sql(),
            "UPDATE battlenet_account_friends SET note = ? WHERE account_id = ? AND friend_id = ?"
        );
        for statement in [
            LoginStatements::SEL_BNET_ACCOUNT_IDENTITIES_ALL,
            LoginStatements::SEL_BNET_ACCOUNT_IDENTITY_BY_ID,
            LoginStatements::SEL_BNET_ACCOUNT_IDENTITY_BY_BATTLE_TAG,
            LoginStatements::SEL_BNET_ACCOUNT_IDENTITY_BY_EMAIL,
        ] {
            assert!(
                statement
                    .sql()
                    .starts_with("SELECT id, email, battle_tag FROM battlenet_accounts"),
                "{statement:?}"
            );
        }
        assert!(
            LoginStatements::SEL_BNET_ACCOUNT_IDENTITY_BY_BATTLE_TAG
                .sql()
                .ends_with("WHERE battle_tag = ?")
        );
    }

    #[test]
    fn insert_invitation_binds_every_column() {
        let statement = insert_invitation_statement_like_cpp(&invitation());
        assert_eq!(statement.params().len(), 6);
    }
}
