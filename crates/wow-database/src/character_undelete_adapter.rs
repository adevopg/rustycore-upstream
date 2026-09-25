//! MariaDB adapter for character deletion (unlink) and undelete.
//!
//! C++ anchors (TDB343.24081): `Player::DeleteFromDB` `CHAR_DELETE_UNLINK`
//! (`Player.cpp:4193-4200`), `WorldSession::HandleGetUndeleteCooldownStatus` and
//! `HandleCharUndeleteOpcode` (`CharacterHandler.cpp:2612-2735`),
//! `CharacterDatabase.cpp` `CHAR_UPD_DELETE_INFO` / `CHAR_UPD_RESTORE_DELETE_INFO` /
//! `CHAR_SEL_CHAR_DEL_INFO_BY_GUID` / `CHAR_SEL_CHECK_NAME` / `CHAR_SEL_SUM_CHARS`,
//! `LoginDatabase.cpp` `LOGIN_SEL_LAST_CHAR_UNDELETE` / `LOGIN_UPD_LAST_CHAR_UNDELETE`.

use std::sync::Arc;

use wow_persistence::{
    CharacterAdministrationLoadOutcomeLikeCpp as LoadOutcome,
    CharacterAdministrationMutationOutcomeLikeCpp as MutationOutcome,
    CharacterDeleteCandidateLikeCpp, CharacterUndeletePersistencePortLikeCpp,
    DeletedCharacterInfoLikeCpp, PersistenceFutureLikeCpp,
};

use crate::{
    CharStatements, CharacterDatabase, CharacterIdentityCacheLikeCpp, LoginDatabase,
    LoginStatements, SqlResult,
};

/// Unsigned integer column whatever its declared width.
fn column_u64_like_cpp(result: &SqlResult, column: usize) -> u64 {
    result
        .try_read::<u64>(column)
        .or_else(|| result.try_read::<u32>(column).map(u64::from))
        .or_else(|| result.try_read::<i64>(column).map(|v| v.max(0) as u64))
        .or_else(|| result.try_read::<i32>(column).map(|v| v.max(0) as u64))
        .unwrap_or(0)
}

pub struct MariaDbCharacterUndeletePersistenceAdapterLikeCpp {
    character_db: Arc<CharacterDatabase>,
    login_db: Arc<LoginDatabase>,
    identity_cache: Arc<CharacterIdentityCacheLikeCpp>,
}

impl MariaDbCharacterUndeletePersistenceAdapterLikeCpp {
    pub fn new(
        character_db: Arc<CharacterDatabase>,
        login_db: Arc<LoginDatabase>,
        identity_cache: Arc<CharacterIdentityCacheLikeCpp>,
    ) -> Self {
        Self {
            character_db,
            login_db,
            identity_cache,
        }
    }
}

fn failed(error: impl ToString) -> MutationOutcome {
    MutationOutcome::Failed {
        reason: error.to_string(),
    }
}

impl CharacterUndeletePersistencePortLikeCpp for MariaDbCharacterUndeletePersistenceAdapterLikeCpp {
    fn load_delete_candidate_like_cpp(
        &self,
        guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<CharacterDeleteCandidateLikeCpp>> {
        Box::pin(async move {
            match self.identity_cache.get(guid) {
                Some(entry) => LoadOutcome::Loaded(CharacterDeleteCandidateLikeCpp {
                    class: entry.class,
                    level: entry.level,
                }),
                None => LoadOutcome::NotFound,
            }
        })
    }

    fn unlink_owned_character_like_cpp(
        &self,
        guid: u64,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        Box::pin(async move {
            // Same ownership guard as the RustyCore remove path.
            let mut check = self
                .character_db
                .prepare(CharStatements::SEL_CHAR_DEL_CHECK);
            check.set_u32(0, guid as u32);
            check.set_u32(1, account_id);
            match self.character_db.query(&check).await {
                Ok(result) if result.is_empty() => {
                    return failed("character is not owned by account");
                }
                Ok(_) => {}
                Err(error) => return failed(error),
            }
            let mut unlink = self.character_db.prepare(CharStatements::UPD_DELETE_INFO);
            unlink.set_u64(0, guid);
            match self.character_db.execute(&unlink).await {
                Ok(_) => {
                    self.identity_cache.update_deleted(guid, true, "");
                    MutationOutcome::Applied
                }
                Err(error) => failed(error),
            }
        })
    }

    fn load_last_character_undelete_like_cpp(
        &self,
        battlenet_account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<u32>> {
        Box::pin(async move {
            let mut stmt = self
                .login_db
                .prepare(LoginStatements::SEL_LAST_CHAR_UNDELETE);
            stmt.set_u32(0, battlenet_account_id);
            match self.login_db.query(&stmt).await {
                Ok(result) if result.is_empty() => LoadOutcome::NotFound,
                Ok(result) => LoadOutcome::Loaded(column_u64_like_cpp(&result, 0) as u32),
                Err(error) => LoadOutcome::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn load_deleted_character_like_cpp(
        &self,
        guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<DeletedCharacterInfoLikeCpp>> {
        Box::pin(async move {
            let mut stmt = self
                .character_db
                .prepare(CharStatements::SEL_CHAR_DEL_INFO_BY_GUID);
            stmt.set_u64(0, guid);
            match self.character_db.query(&stmt).await {
                Ok(result) if result.is_empty() => LoadOutcome::NotFound,
                Ok(result) => LoadOutcome::Loaded(DeletedCharacterInfoLikeCpp {
                    name: result.try_read::<String>(1).unwrap_or_default(),
                    account_id: column_u64_like_cpp(&result, 2) as u32,
                }),
                Err(error) => LoadOutcome::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn find_character_name_like_cpp(
        &self,
        name: String,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<()>> {
        Box::pin(async move {
            let mut stmt = self.character_db.prepare(CharStatements::SEL_CHECK_NAME);
            stmt.set_string(0, &name);
            match self.character_db.query(&stmt).await {
                Ok(result) if result.is_empty() => LoadOutcome::NotFound,
                Ok(_) => LoadOutcome::Loaded(()),
                Err(error) => LoadOutcome::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn load_account_character_count_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<u64>> {
        Box::pin(async move {
            let mut stmt = self.character_db.prepare(CharStatements::SEL_SUM_CHARS);
            stmt.set_u32(0, account_id);
            match self.character_db.query(&stmt).await {
                Ok(result) if result.is_empty() => LoadOutcome::NotFound,
                Ok(result) => LoadOutcome::Loaded(column_u64_like_cpp(&result, 0)),
                Err(error) => LoadOutcome::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn restore_deleted_character_like_cpp(
        &self,
        guid: u64,
        name: String,
        account_id: u32,
        battlenet_account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        Box::pin(async move {
            let mut restore = self
                .character_db
                .prepare(CharStatements::UPD_RESTORE_DELETE_INFO);
            restore.set_string(0, &name);
            restore.set_u32(1, account_id);
            restore.set_u64(2, guid);
            match self.character_db.execute(&restore).await {
                // `deleteDate IS NOT NULL` guard: a concurrent restore already won.
                Ok(0) => return failed("character is no longer deleted"),
                Ok(_) => {}
                Err(error) => return failed(error),
            }
            // C++ executes both statements independently; the character row is the
            // authority, so a failed cooldown stamp is only logged by the caller.
            self.identity_cache.update_deleted(guid, false, &name);
            let mut stamp = self
                .login_db
                .prepare(LoginStatements::UPD_LAST_CHAR_UNDELETE);
            stamp.set_u32(0, battlenet_account_id);
            match self.login_db.execute(&stamp).await {
                Ok(_) => MutationOutcome::Applied,
                Err(error) => {
                    tracing::warn!(
                        guid,
                        battlenet_account_id,
                        %error,
                        "character restored but LastCharacterUndelete was not stamped"
                    );
                    MutationOutcome::Applied
                }
            }
        })
    }

    fn reset_undelete_cooldown_like_cpp(
        &self,
        battlenet_account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        Box::pin(async move {
            let mut stmt = self
                .login_db
                .prepare(LoginStatements::RES_LAST_CHAR_UNDELETE);
            stmt.set_u32(0, battlenet_account_id);
            match self.login_db.execute(&stmt).await {
                Ok(_) => MutationOutcome::Applied,
                Err(error) => failed(error),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{CharStatements, LoginStatements, StatementDef};

    #[test]
    fn unlink_and_restore_statements_are_the_cpp_ones() {
        assert_eq!(
            CharStatements::UPD_DELETE_INFO.sql(),
            "UPDATE characters SET deleteInfos_Name = name, deleteInfos_Account = account, deleteDate = UNIX_TIMESTAMP(), name = '', account = 0 WHERE guid = ?"
        );
        assert_eq!(
            CharStatements::UPD_RESTORE_DELETE_INFO.sql(),
            "UPDATE characters SET name = ?, account = ?, deleteDate = NULL, deleteInfos_Name = NULL, deleteInfos_Account = NULL WHERE deleteDate IS NOT NULL AND guid = ?"
        );
        assert_eq!(
            CharStatements::SEL_CHAR_DEL_INFO_BY_GUID.sql(),
            "SELECT guid, deleteInfos_Name, deleteInfos_Account, deleteDate FROM characters WHERE deleteDate IS NOT NULL AND guid = ?"
        );
        assert_eq!(
            CharStatements::SEL_SUM_CHARS.sql(),
            "SELECT COUNT(guid) FROM characters WHERE account = ? AND deleteDate IS NULL"
        );
    }

    #[test]
    fn undelete_cooldown_statements_use_the_battlenet_account() {
        assert_eq!(
            LoginStatements::SEL_LAST_CHAR_UNDELETE.sql(),
            "SELECT LastCharacterUndelete FROM battlenet_accounts WHERE Id = ?"
        );
        assert_eq!(
            LoginStatements::UPD_LAST_CHAR_UNDELETE.sql(),
            "UPDATE battlenet_accounts SET LastCharacterUndelete = UNIX_TIMESTAMP() WHERE Id = ?"
        );
        assert_eq!(
            LoginStatements::RES_LAST_CHAR_UNDELETE.sql(),
            "UPDATE battlenet_accounts SET LastCharacterUndelete = 0 WHERE Id = ?"
        );
    }
}
