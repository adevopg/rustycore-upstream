//! MariaDB adapter for C++ character-list administration.

use std::sync::Arc;

use wow_persistence::{
    CharacterAdministrationLoadOutcomeLikeCpp as LoadOutcome,
    CharacterAdministrationMutationOutcomeLikeCpp as MutationOutcome,
    CharacterAdministrationPersistencePortLikeCpp, CharacterCreateItemPersistenceLikeCpp,
    CharacterCreatePersistenceRequestLikeCpp, CharacterCustomizationPersistenceLikeCpp,
    CharacterCustomizeCandidateLikeCpp, CharacterRaceOrFactionChangeCandidateLikeCpp,
    CharacterRaceOrFactionChangeCommitLikeCpp, CharacterRenameCandidateLikeCpp,
    PersistenceFutureLikeCpp,
};

use crate::{
    CharStatements, CharacterDatabase, PreparedStatement, SqlTransaction, WorldDatabase,
    WorldStatements,
};
use crate::{CharacterIdentityCacheEntryLikeCpp, CharacterIdentityCacheLikeCpp};

mod race_faction_change;

/// C++ `Item::SaveToDB` (`CHAR_REP_ITEM_INSTANCE`) plus
/// `Player::_SaveInventory` (`CHAR_REP_INVENTORY_ITEM`) for one initial item
/// of a new character. New items carry no random properties, enchantments
/// or charges string in the represented Rust item-instance insert.
fn character_create_item_statements_like_cpp(
    owner_guid: u64,
    item: &CharacterCreateItemPersistenceLikeCpp,
) -> [PreparedStatement; 2] {
    let mut instance =
        PreparedStatement::for_statement(CharStatements::INS_ITEM_INSTANCE_WITH_RANDOM_CONTEXT);
    instance.set_u64(0, item.item_guid);
    instance.set_u32(1, item.item_id);
    instance.set_u64(2, owner_guid);
    instance.set_u32(3, item.count);
    instance.set_u32(4, item.durability);
    instance.set_u32(5, item.dynamic_flags);
    instance.set_i32(6, 0);
    instance.set_i32(7, 0);
    instance.set_u8(8, item.item_context);

    let mut link = PreparedStatement::for_statement(CharStatements::REP_CHAR_INVENTORY_ITEM);
    link.set_u64(0, owner_guid);
    link.set_u64(1, item.bag_guid);
    link.set_u8(2, item.slot);
    link.set_u64(3, item.item_guid);
    [instance, link]
}

pub struct MariaDbCharacterAdministrationPersistenceAdapterLikeCpp {
    character_db: Arc<CharacterDatabase>,
    world_db: Arc<WorldDatabase>,
    identity_cache: Arc<CharacterIdentityCacheLikeCpp>,
}

impl MariaDbCharacterAdministrationPersistenceAdapterLikeCpp {
    pub fn new(
        character_db: Arc<CharacterDatabase>,
        world_db: Arc<WorldDatabase>,
        identity_cache: Arc<CharacterIdentityCacheLikeCpp>,
    ) -> Self {
        Self {
            character_db,
            world_db,
            identity_cache,
        }
    }
}

/// C++ `Player::DeleteFromDB`'s `CHAR_DELETE_REMOVE` branch
/// (`Entities/Player/Player.cpp`, `Player::DeleteFromDB`): every dependent row of
/// the character goes with it, in **one** CharacterDatabase transaction, each
/// statement bound only with the character guid.
///
/// The order below is the C++ append order, statement for statement. None of this
/// is new SQL: all 49 identities already existed and simply had no caller, which is
/// why a delete used to leave inventory, items, skills, glyphs, reputation and
/// homebind rows behind.
///
/// **Scope contract — intentional, bounded departure.** Three C++ steps are *not*
/// reproduced here, because each needs a read or a second database this path does
/// not have, and inventing them would be worse than recording them:
///   * the COD-mail refund and the per-mail-id item deletes, which C++ drives from
///     `CHAR_SEL_CHAR_COD_ITEM_MAIL` and `CHAR_SEL_MAILITEMS` before the
///     transaction. The unconditional `DEL_MAIL`/`DEL_MAIL_ITEMS` below still remove
///     the character's own mail rows.
///   * the pet-id walk from `CHAR_SEL_CHAR_PET_IDS`. `DEL_CHAR_PET_BY_OWNER` and
///     `DEL_CHAR_PET_DECLINEDNAME_BY_OWNER` below still remove the owner's rows.
///   * `LOGIN_DEL_BATTLE_PET_DECLINED_NAME_BY_OWNER` and
///     `LOGIN_DEL_BATTLE_PETS_BY_OWNER`, which are LoginDatabase statements.
fn character_delete_transaction_like_cpp(guid: u64) -> SqlTransaction {
    let mut transaction = SqlTransaction::new();
    for statement in character_delete_statements_like_cpp(guid) {
        transaction.append(statement);
    }
    transaction
}

/// The statements themselves, in C++ order, separated from the transaction so the
/// order and the binds can be asserted without an accessor on `SqlTransaction`.
fn character_delete_statements_like_cpp(guid: u64) -> Vec<PreparedStatement> {
    let mut statements = Vec::new();
    let push_guid_only = |statement: CharStatements, into: &mut Vec<PreparedStatement>| {
        let mut prepared = PreparedStatement::for_statement(statement);
        prepared.set_u64(0, guid);
        into.push(prepared);
    };

    for statement in CHARACTER_DELETE_GUID_ONLY_STATEMENTS_BEFORE_GUILD_EVENTLOG_LIKE_CPP {
        push_guid_only(statement, &mut statements);
    }

    // C++ binds the guid twice here: the row matches either participant column.
    let mut guild_eventlog =
        PreparedStatement::for_statement(CharStatements::DEL_GUILD_EVENTLOG_BY_PLAYER);
    guild_eventlog.set_u64(0, guid);
    guild_eventlog.set_u64(1, guid);
    statements.push(guild_eventlog);

    for statement in CHARACTER_DELETE_GUID_ONLY_STATEMENTS_AFTER_GUILD_EVENTLOG_LIKE_CPP {
        push_guid_only(statement, &mut statements);
    }

    statements
}

/// The C++ append order up to `CHAR_DEL_GUILD_EVENTLOG_BY_PLAYER`.
const CHARACTER_DELETE_GUID_ONLY_STATEMENTS_BEFORE_GUILD_EVENTLOG_LIKE_CPP: [CharStatements; 36] = [
    CharStatements::DEL_CHARACTER,
    CharStatements::DEL_CHARACTER_CUSTOMIZATIONS,
    CharStatements::DEL_PLAYER_ACCOUNT_DATA,
    CharStatements::DEL_CHAR_DECLINED_NAME,
    CharStatements::DEL_CHAR_ACTION,
    CharStatements::DEL_CHARACTER_ARENA_STATS,
    CharStatements::DEL_CHAR_AURA_EFFECT,
    CharStatements::DEL_CHAR_AURA,
    CharStatements::DEL_PLAYER_BGDATA,
    CharStatements::DEL_BATTLEGROUND_RANDOM,
    CharStatements::DEL_CHAR_CUF_PROFILES,
    CharStatements::DEL_PLAYER_CURRENCY,
    CharStatements::DEL_CHAR_GIFT,
    CharStatements::DEL_PLAYER_HOMEBIND,
    CharStatements::DEL_CHARACTER_INSTANCE_LOCK_BY_GUID,
    CharStatements::DEL_CHAR_INVENTORY,
    CharStatements::DEL_CHAR_QUESTSTATUS,
    CharStatements::DEL_CHAR_QUESTSTATUS_OBJECTIVES,
    CharStatements::DEL_CHAR_QUESTSTATUS_REWARDED,
    CharStatements::DEL_CHAR_REPUTATION,
    CharStatements::DEL_CHAR_SPELL,
    CharStatements::DEL_CHAR_SPELL_COOLDOWNS,
    CharStatements::DEL_CHAR_SPELL_CHARGES,
    CharStatements::DEL_ITEM_INSTANCE_GEMS_BY_OWNER,
    CharStatements::DEL_ITEM_INSTANCE_TRANSMOG_BY_OWNER,
    CharStatements::DEL_ITEM_INSTANCE_BY_OWNER,
    CharStatements::DEL_CHAR_SOCIAL_BY_FRIEND,
    CharStatements::DEL_CHAR_SOCIAL_BY_GUID,
    CharStatements::DEL_MAIL,
    CharStatements::DEL_MAIL_ITEMS,
    CharStatements::DEL_CHAR_PET_BY_OWNER,
    CharStatements::DEL_CHAR_PET_DECLINEDNAME_BY_OWNER,
    CharStatements::DEL_CHAR_ACHIEVEMENTS,
    CharStatements::DEL_CHAR_ACHIEVEMENT_PROGRESS,
    CharStatements::DEL_CHAR_EQUIPMENTSETS,
    CharStatements::DEL_CHAR_TRANSMOG_OUTFITS,
];

/// The C++ append order after `CHAR_DEL_GUILD_EVENTLOG_BY_PLAYER`.
const CHARACTER_DELETE_GUID_ONLY_STATEMENTS_AFTER_GUILD_EVENTLOG_LIKE_CPP: [CharStatements; 12] = [
    CharStatements::DEL_GUILD_BANK_EVENTLOG_BY_PLAYER,
    CharStatements::DEL_CHAR_GLYPHS,
    CharStatements::DEL_CHARACTER_QUESTSTATUS_DAILY,
    CharStatements::DEL_CHARACTER_QUESTSTATUS_WEEKLY,
    CharStatements::DEL_CHARACTER_QUESTSTATUS_MONTHLY,
    CharStatements::DEL_CHARACTER_QUESTSTATUS_SEASONAL,
    CharStatements::DEL_CHAR_TALENT,
    CharStatements::DEL_CHAR_SKILLS,
    CharStatements::DEL_CHAR_STATS,
    CharStatements::DEL_CHAR_FISHINGSTEPS,
    CharStatements::DEL_CHARACTER_FAVORITE_AUCTIONS_BY_CHAR,
    CharStatements::DEL_CHARACTER_AURA_STORED_LOCATIONS_BY_GUID,
];

impl CharacterAdministrationPersistencePortLikeCpp
    for MariaDbCharacterAdministrationPersistenceAdapterLikeCpp
{
    fn find_character_name_like_cpp(
        &self,
        name: &str,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<()>> {
        let name = name.to_owned();
        Box::pin(async move {
            let mut statement = self.character_db.prepare(CharStatements::SEL_CHECK_NAME);
            statement.set_string(0, &name);
            match self.character_db.query(&statement).await {
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
            let mut statement = self.character_db.prepare(CharStatements::SEL_SUM_CHARS);
            statement.set_u32(0, account_id);
            match self.character_db.query(&statement).await {
                Ok(result) if result.is_empty() => LoadOutcome::NotFound,
                Ok(result) => LoadOutcome::Loaded(result.try_read(0).unwrap_or(0)),
                Err(error) => LoadOutcome::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn create_character_like_cpp(
        &self,
        request: CharacterCreatePersistenceRequestLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        Box::pin(async move {
            let mut statement = self.character_db.prepare(CharStatements::INS_CHARACTER);
            statement.set_u64(0, request.guid);
            statement.set_u32(1, request.account_id);
            statement.set_string(2, &request.name);
            statement.set_u8(3, request.race);
            statement.set_u8(4, request.class);
            statement.set_u8(5, request.sex);
            statement.set_u8(6, 1);
            statement.set_u64(7, 0);
            statement.set_u64(8, 0);
            statement.set_u32(9, 16);
            statement.set_u32(10, 0);
            statement.set_u8(11, request.rest_state);
            statement.set_u32(12, 0);
            statement.set_u32(13, 0);
            statement.set_i32(14, request.map_id);
            statement.set_u32(15, 0);
            statement.set_u8(16, 0);
            statement.set_u8(17, 0);
            statement.set_u8(18, 0);
            statement.set_f32(19, request.position[0]);
            statement.set_f32(20, request.position[1]);
            statement.set_f32(21, request.position[2]);
            statement.set_f32(22, request.position[3]);
            for index in 23..=26 {
                statement.set_f32(index, 0.0);
            }
            statement.set_u64(27, 0);
            statement.set_string(28, "");
            statement.set_i64(29, request.create_time);
            statement.set_u8(30, 0);
            statement.set_u8(31, 0);
            statement.set_u32(32, 0);
            statement.set_u32(33, 0);
            statement.set_f32(34, 0.0);
            statement.set_u64(35, request.create_time.max(0) as u64);
            statement.set_u8(36, 0);
            statement.set_u32(37, 0);
            statement.set_u32(38, 0);
            statement.set_u8(39, 0);
            statement.set_u8(40, 0);
            statement.set_u32(41, 0);
            statement.set_u32(42, 0);
            statement.set_u32(43, 0x20);
            statement.set_u32(44, 0);
            statement.set_string(45, "");
            for index in 46..=49 {
                statement.set_u32(index, 0);
            }
            statement.set_i32(50, 0);
            statement.set_u8(51, 0);
            statement.set_u32(52, request.health);
            statement.set_u32(53, request.power1);
            for index in 54..=64 {
                statement.set_u32(index, 0);
            }
            statement.set_string(65, "");
            statement.set_string(66, &request.equipment_cache);
            statement.set_string(67, "");
            statement.set_u8(68, 0);
            statement.set_u32(69, request.last_login_build);

            // C++ `HandleCharCreateOpcode` saves the character row and its
            // initial inventory (`Player::SaveToDB` -> `_SaveInventory`) in
            // one character transaction; keep those rows atomic here.
            let mut transaction = SqlTransaction::new();
            transaction.append(statement);
            for item in &request.items {
                for item_statement in character_create_item_statements_like_cpp(request.guid, item)
                {
                    transaction.append(item_statement);
                }
            }
            if let Err(error) = self.character_db.commit_transaction(transaction).await {
                return MutationOutcome::Failed {
                    reason: error.to_string(),
                };
            }

            // Preserve the existing best-effort order after that commit:
            // choices, then initial action buttons. C++-parity atomic creation
            // of those rows remains a gameplay gap.
            for customization in &request.customizations {
                let mut statement = self
                    .character_db
                    .prepare(CharStatements::INS_CHAR_CUSTOMIZATION);
                statement.set_u64(0, request.guid);
                statement.set_i32(1, customization.option_id);
                statement.set_i32(2, customization.choice_id);
                let _ = self.character_db.execute(&statement).await;
            }

            let action_statement = self
                .world_db
                .prepare(WorldStatements::SEL_PLAYER_CREATEINFO_ACTION);
            if let Ok(mut rows) = self.world_db.query(&action_statement).await {
                if !rows.is_empty() {
                    loop {
                        let race: u8 = rows.read(0);
                        let class: u8 = rows.read(1);
                        let action: i32 = rows.try_read(3).unwrap_or(0);
                        if race == request.race && class == request.class && action > 0 {
                            let mut insert = self
                                .character_db
                                .prepare(CharStatements::INS_CHARACTER_ACTION);
                            insert.set_u64(0, request.guid);
                            insert.set_u8(1, rows.read(2));
                            insert.set_i32(2, action);
                            insert.set_u8(3, rows.try_read(4).unwrap_or(0));
                            let _ = self.character_db.execute(&insert).await;
                        }
                        if !rows.next_row() {
                            break;
                        }
                    }
                }
            }
            self.identity_cache
                .upsert(CharacterIdentityCacheEntryLikeCpp {
                    guid_low: request.guid,
                    name: request.name,
                    account_id: request.account_id,
                    race: request.race,
                    class: request.class,
                    sex: request.sex,
                    level: 1,
                    is_deleted: false,
                });
            MutationOutcome::Applied
        })
    }

    fn delete_owned_character_like_cpp(
        &self,
        guid: u64,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        Box::pin(async move {
            let mut check = self
                .character_db
                .prepare(CharStatements::SEL_CHAR_DEL_CHECK);
            check.set_u32(0, guid as u32);
            check.set_u32(1, account_id);
            if let Ok(result) = self.character_db.query(&check).await {
                if result.is_empty() {
                    return MutationOutcome::Failed {
                        reason: "character is not owned by account".into(),
                    };
                }
            }
            match self
                .character_db
                .commit_transaction(character_delete_transaction_like_cpp(guid))
                .await
            {
                Ok(_) => {
                    self.identity_cache.remove(guid);
                    MutationOutcome::Applied
                }
                Err(error) => MutationOutcome::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn load_rename_candidate_like_cpp(
        &self,
        guid: u64,
        new_name: &str,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<CharacterRenameCandidateLikeCpp>> {
        let new_name = new_name.to_owned();
        Box::pin(async move {
            let mut statement = self.character_db.prepare(CharStatements::SEL_FREE_NAME);
            statement.set_u64(0, guid);
            statement.set_string(1, &new_name);
            match self.character_db.query(&statement).await {
                Ok(result) if result.is_empty() => LoadOutcome::NotFound,
                Ok(result) => LoadOutcome::Loaded(CharacterRenameCandidateLikeCpp {
                    old_name: result.read_string(0),
                    at_login_flags: result.try_read(1).unwrap_or(0),
                }),
                Err(error) => LoadOutcome::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn commit_rename_like_cpp(
        &self,
        guid: u64,
        new_name: &str,
        at_login_flags: u16,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        let new_name = new_name.to_owned();
        Box::pin(async move {
            let mut transaction = SqlTransaction::new();
            let mut update = self
                .character_db
                .prepare(CharStatements::UPD_CHAR_NAME_AT_LOGIN);
            update.set_string(0, &new_name);
            update.set_u16(1, at_login_flags);
            update.set_u64(2, guid);
            transaction.append(update);
            let mut delete = self
                .character_db
                .prepare(CharStatements::DEL_CHAR_DECLINED_NAME);
            delete.set_u64(0, guid);
            transaction.append(delete);
            match self.character_db.commit_transaction(transaction).await {
                Ok(_) => {
                    self.identity_cache.update_name(guid, &new_name);
                    MutationOutcome::Applied
                }
                Err(error) => MutationOutcome::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn load_race_or_faction_change_candidate_like_cpp(
        &self,
        guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<CharacterRaceOrFactionChangeCandidateLikeCpp>>
    {
        Box::pin(async move {
            let mut cache = self
                .character_db
                .prepare(CharStatements::SEL_CHAR_RACE_OR_FACTION_CHANGE_CACHE);
            cache.set_u64(0, guid);
            let mut infos = self
                .character_db
                .prepare(CharStatements::SEL_CHAR_RACE_OR_FACTION_CHANGE_INFOS);
            infos.set_u64(0, guid);
            let cache = match self.character_db.query(&cache).await {
                Ok(result) if result.is_empty() => return LoadOutcome::NotFound,
                Ok(result) => result,
                Err(error) => {
                    return LoadOutcome::Failed {
                        reason: error.to_string(),
                    };
                }
            };
            match self.character_db.query(&infos).await {
                Ok(result) if result.is_empty() => LoadOutcome::NotFound,
                Ok(result) => LoadOutcome::Loaded(
                    race_faction_change::candidate_from_rows_like_cpp(&cache, &result),
                ),
                Err(error) => LoadOutcome::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn load_reputation_standing_like_cpp(
        &self,
        guid: u64,
        faction_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<i32>> {
        Box::pin(async move {
            let mut statement = self
                .character_db
                .prepare(CharStatements::SEL_CHAR_REP_BY_FACTION);
            statement.set_u32(0, faction_id);
            statement.set_u64(1, guid);
            match self.character_db.query(&statement).await {
                Ok(result) if result.is_empty() => LoadOutcome::NotFound,
                Ok(result) => LoadOutcome::Loaded(crate::battle_pay_adapter::column_i64_like_cpp(
                    &result, 0,
                ) as i32),
                Err(error) => LoadOutcome::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn commit_race_or_faction_change_like_cpp(
        &self,
        request: CharacterRaceOrFactionChangeCommitLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        Box::pin(async move {
            let mut new_guild_leader = None;
            if let Some(guild) = request
                .faction
                .as_ref()
                .and_then(|faction| faction.guild)
                .filter(|guild| guild.is_leader)
            {
                let mut statement = self
                    .character_db
                    .prepare(CharStatements::SEL_GUILD_NEW_LEADER_CANDIDATE);
                statement.set_u64(0, guild.guild_id);
                statement.set_u64(1, request.guid);
                match self.character_db.query(&statement).await {
                    Ok(result) if result.is_empty() => {}
                    Ok(result) => {
                        new_guild_leader =
                            Some(crate::battle_pay_adapter::column_u64_like_cpp(&result, 0));
                    }
                    Err(error) => {
                        return MutationOutcome::Failed {
                            reason: error.to_string(),
                        };
                    }
                }
            }
            let transaction = race_faction_change::race_or_faction_change_transaction_like_cpp(
                &request,
                new_guild_leader,
            );
            match self.character_db.commit_transaction(transaction).await {
                Ok(_) => {
                    // C++ sCharacterCache->UpdateCharacterData(guid, name, sex, race).
                    self.identity_cache.update_identity(
                        request.guid,
                        request.name.clone(),
                        Some(request.race),
                        Some(request.sex),
                    );
                    MutationOutcome::Applied
                }
                Err(error) => MutationOutcome::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn load_customize_candidate_like_cpp(
        &self,
        guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<CharacterCustomizeCandidateLikeCpp>> {
        Box::pin(async move {
            let mut statement = self
                .character_db
                .prepare(CharStatements::SEL_CHAR_CUSTOMIZE_INFO);
            statement.set_u64(0, guid);
            match self.character_db.query(&statement).await {
                Ok(result) if result.is_empty() => LoadOutcome::NotFound,
                Ok(result) => LoadOutcome::Loaded(CharacterCustomizeCandidateLikeCpp {
                    old_name: result.read_string(0),
                    race: result.try_read(1).unwrap_or(0),
                    class: result.try_read(2).unwrap_or(0),
                    gender: result.try_read(3).unwrap_or(0),
                    at_login_flags: result.try_read(4).unwrap_or(0),
                }),
                Err(error) => LoadOutcome::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn commit_customize_like_cpp(
        &self,
        guid: u64,
        name: &str,
        at_login_flags: u16,
        customizations: Vec<CharacterCustomizationPersistenceLikeCpp>,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        let name = name.to_owned();
        Box::pin(async move {
            let mut transaction = SqlTransaction::new();
            let mut delete = self
                .character_db
                .prepare(CharStatements::DEL_CHARACTER_CUSTOMIZATIONS);
            delete.set_u64(0, guid);
            transaction.append(delete);
            for customization in customizations {
                let mut insert = self
                    .character_db
                    .prepare(CharStatements::INS_CHAR_CUSTOMIZATION);
                insert.set_u64(0, guid);
                insert.set_i32(1, customization.option_id);
                insert.set_i32(2, customization.choice_id);
                transaction.append(insert);
            }
            let mut update = self
                .character_db
                .prepare(CharStatements::UPD_CHAR_NAME_AT_LOGIN);
            update.set_string(0, &name);
            update.set_u16(1, at_login_flags);
            update.set_u64(2, guid);
            transaction.append(update);
            let mut delete_declined = self
                .character_db
                .prepare(CharStatements::DEL_CHAR_DECLINED_NAME);
            delete_declined.set_u64(0, guid);
            transaction.append(delete_declined);
            match self.character_db.commit_transaction(transaction).await {
                Ok(_) => {
                    self.identity_cache.update_name(guid, &name);
                    MutationOutcome::Applied
                }
                Err(error) => MutationOutcome::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StatementDef;

    #[test]
    fn create_item_statements_insert_instance_then_inventory_link_like_cpp() {
        let [instance, link] = character_create_item_statements_like_cpp(
            7,
            &CharacterCreateItemPersistenceLikeCpp {
                item_guid: 100,
                item_id: 6948,
                count: 1,
                durability: 0,
                dynamic_flags: 1,
                item_context: 75,
                bag_guid: 0,
                slot: 35,
            },
        );
        assert_eq!(
            instance.sql(),
            CharStatements::INS_ITEM_INSTANCE_WITH_RANDOM_CONTEXT.sql()
        );
        assert_eq!(instance.sql().matches('?').count(), 9);
        assert_eq!(link.sql(), CharStatements::REP_CHAR_INVENTORY_ITEM.sql());
        assert_eq!(link.sql().matches('?').count(), 4);
    }
}

#[cfg(test)]
mod character_delete_tests {
    use super::*;
    use crate::params::SqlParam;

    /// The C++ `CHAR_DELETE_REMOVE` append order, transcribed from
    /// `Player::DeleteFromDB`. A delete that drops one of these leaks that table.
    const CPP_DELETE_ORDER: [&str; 49] = [
        "DELETE FROM characters WHERE guid = ?",
        "DELETE FROM character_customizations WHERE guid = ?",
        "DELETE FROM character_account_data WHERE guid = ?",
        "DELETE FROM character_declinedname WHERE guid = ?",
        "DELETE FROM character_action WHERE guid = ?",
        "DELETE FROM character_arena_stats WHERE guid = ?",
        "DELETE FROM character_aura_effect WHERE guid = ?",
        "DELETE FROM character_aura WHERE guid = ?",
        "DELETE FROM character_battleground_data WHERE guid = ?",
        "DELETE FROM character_battleground_random WHERE guid = ?",
        "DELETE FROM character_cuf_profiles WHERE guid = ?",
        "DELETE FROM character_currency WHERE CharacterGuid = ?",
        "DELETE FROM character_gifts WHERE guid = ?",
        "DELETE FROM character_homebind WHERE guid = ?",
        "DELETE FROM character_instance_lock WHERE guid = ?",
        "DELETE FROM character_inventory WHERE guid = ?",
        "DELETE FROM character_queststatus WHERE guid = ?",
        "DELETE FROM character_queststatus_objectives WHERE guid = ?",
        "DELETE FROM character_queststatus_rewarded WHERE guid = ?",
        "DELETE FROM character_reputation WHERE guid = ?",
        "DELETE FROM character_spell WHERE guid = ?",
        "DELETE FROM character_spell_cooldown WHERE guid = ?",
        "DELETE FROM character_spell_charges WHERE guid = ?",
        "DELETE iig FROM item_instance_gems iig LEFT JOIN item_instance ii ON iig.itemGuid = ii.guid WHERE ii.owner_guid = ?",
        "DELETE iit FROM item_instance_transmog iit LEFT JOIN item_instance ii ON iit.itemGuid = ii.guid WHERE ii.owner_guid = ?",
        "DELETE FROM item_instance WHERE owner_guid = ?",
        "DELETE FROM character_social WHERE friend = ?",
        "DELETE FROM character_social WHERE guid = ?",
        "DELETE FROM mail WHERE receiver = ?",
        "DELETE FROM mail_items WHERE receiver = ?",
        "DELETE FROM character_pet WHERE owner = ?",
        "DELETE FROM character_pet_declinedname WHERE owner = ?",
        "DELETE FROM character_achievement WHERE guid = ? AND achievement NOT IN (456,457,458,459,460,461,462,463,464,465,466,467,1400,1402,1404,1405,1406,1407,1408,1409,1410,1411,1412,1413,1414,1415,1416,1417,1418,1419,1420,1421,1422,1423,1424,1425,1426,1427,1463,3117,3259,4078,4576,4998,4999,5000,5001,5002,5003,5004,5005,5006,5007,5008,5381,5382,5383,5384,5385,5386,5387,5388,5389,5390,5391,5392,5393,5394,5395,5396,6433,6523,6524,6743,6744,6745,6746,6747,6748,6749,6750,6751,6752,6829,6859,6860,6861,6862,6863,6864,6865,6866,6867,6868,6869,6870,6871,6872,6873)",
        "DELETE FROM character_achievement_progress WHERE guid = ?",
        "DELETE FROM character_equipmentsets WHERE guid = ?",
        "DELETE FROM character_transmog_outfits WHERE guid = ?",
        "DELETE FROM guild_eventlog WHERE PlayerGuid1 = ? OR PlayerGuid2 = ?",
        "DELETE FROM guild_bank_eventlog WHERE PlayerGuid = ?",
        "DELETE FROM character_glyphs WHERE guid = ?",
        "DELETE FROM character_queststatus_daily WHERE guid = ?",
        "DELETE FROM character_queststatus_weekly WHERE guid = ?",
        "DELETE FROM character_queststatus_monthly WHERE guid = ?",
        "DELETE FROM character_queststatus_seasonal WHERE guid = ?",
        "DELETE FROM character_talent WHERE guid = ?",
        "DELETE FROM character_skills WHERE guid = ?",
        "DELETE FROM character_stats WHERE guid = ?",
        "DELETE FROM character_fishingsteps WHERE guid = ?",
        "DELETE FROM character_favorite_auctions WHERE guid = ?",
        "DELETE FROM character_aura_stored_location WHERE Guid = ?",
    ];

    #[test]
    fn the_delete_transaction_follows_the_cpp_append_order_and_binds_only_the_guid() {
        let statements = character_delete_statements_like_cpp(4242);
        assert_eq!(
            statements.len(),
            CPP_DELETE_ORDER.len(),
            "C++ appends {} statements in its CHAR_DELETE_REMOVE branch",
            CPP_DELETE_ORDER.len()
        );
        for (index, expected) in CPP_DELETE_ORDER.iter().enumerate() {
            assert_eq!(
                statements[index].sql(),
                *expected,
                "statement {index} out of C++ order"
            );
            let binds = statements[index].params();
            let wanted = expected.matches('?').count();
            assert_eq!(binds.len(), wanted, "statement {index} bind count");
            for bind in binds {
                assert_eq!(
                    *bind,
                    SqlParam::U64(4242),
                    "statement {index} binds only the character guid"
                );
            }
        }
    }

    #[test]
    fn every_family_the_live_reproduction_leaked_is_now_deleted() {
        // The tables measured as orphaned after a live delete, before this repair:
        // 20 inventory rows, 20 item_instance, 30 skills, 48 glyphs,
        // 210 reputation and 2 homebind, for four level-1 characters.
        let statements = character_delete_statements_like_cpp(7);
        let sql: Vec<&str> = statements.iter().map(|statement| statement.sql()).collect();
        for table in [
            "character_inventory",
            "item_instance",
            "character_skills",
            "character_glyphs",
            "character_reputation",
            "character_homebind",
        ] {
            assert!(
                sql.iter().any(|statement| statement.contains(table)),
                "{table} would still be orphaned by a delete"
            );
        }
    }
}
