// Copyright (c) 2026 alseif0x
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! MariaDB adapter for canonical-map corpse hydration.

use std::sync::Arc;

use wow_persistence::{
    MapCorpseAuxiliaryLoadOutcomeLikeCpp, MapCorpseCustomizationLoadRowLikeCpp,
    MapCorpseLoadOutcomeLikeCpp, MapCorpseLoadRequestLikeCpp, MapCorpseLoadRowLikeCpp,
    MapCorpsePersistencePortLikeCpp, MapCorpsePhaseLoadRowLikeCpp, MapCorpseSaveOutcomeLikeCpp,
    MapCorpseSaveRowLikeCpp, PersistenceFutureLikeCpp,
};

use crate::CharacterDatabase;
use crate::params::PreparedStatement;
use crate::statements::CharStatements;
use crate::transaction::SqlTransaction;

fn map_corpse_load_statements_like_cpp(
    request: MapCorpseLoadRequestLikeCpp,
) -> [PreparedStatement; 3] {
    let mut corpses = PreparedStatement::for_statement(CharStatements::SEL_CORPSES);
    corpses.set_u32(0, request.map_id);
    corpses.set_u32(1, request.instance_id);

    let mut phases = PreparedStatement::for_statement(CharStatements::SEL_CORPSE_PHASES);
    phases.set_u32(0, request.map_id);
    phases.set_u32(1, request.instance_id);

    let mut customizations =
        PreparedStatement::for_statement(CharStatements::SEL_CORPSE_CUSTOMIZATIONS);
    customizations.set_u32(0, request.map_id);
    customizations.set_u32(1, request.instance_id);

    [corpses, phases, customizations]
}

pub struct MariaDbMapCorpsePersistenceAdapterLikeCpp {
    character_db: Arc<CharacterDatabase>,
}

/// C++ `Corpse::SaveToDB` as one transaction: `DeleteFromDB(trans)` first, then
/// `CHAR_INS_CORPSE`. C++ deletes before inserting "to prevent DB data
/// inconsistence problems and duplicates", and the corpse table is keyed by the
/// owner's guid counter, so this is an upsert for that player.
///
/// The phase and customization inserts of the same C++ function
/// (`CHAR_INS_CORPSE_PHASES`, `CHAR_INS_CORPSE_CUSTOMIZATIONS`) are not written
/// here: `create_player_corpse_on_map_like_cpp` does not set customizations or a
/// phase shift yet, so there is nothing to persist. Their deletes ARE issued, so
/// a stale row from an earlier corpse cannot survive.
fn map_corpse_save_transaction_like_cpp(row: &MapCorpseSaveRowLikeCpp) -> SqlTransaction {
    let mut transaction = SqlTransaction::new();

    let mut delete_corpse = PreparedStatement::for_statement(CharStatements::DEL_CORPSE);
    delete_corpse.set_u64(0, row.owner_guid);
    transaction.append(delete_corpse);

    let mut delete_phases = PreparedStatement::for_statement(CharStatements::DEL_CORPSE_PHASES);
    delete_phases.set_u64(0, row.owner_guid);
    transaction.append(delete_phases);

    let mut delete_customizations =
        PreparedStatement::for_statement(CharStatements::DEL_CORPSE_CUSTOMIZATIONS);
    delete_customizations.set_u64(0, row.owner_guid);
    transaction.append(delete_customizations);

    // Field order mirrors `CHAR_INS_CORPSE` exactly.
    let mut insert = PreparedStatement::for_statement(CharStatements::INS_CORPSE);
    insert.set_u64(0, row.owner_guid);
    insert.set_f32(1, row.pos_x);
    insert.set_f32(2, row.pos_y);
    insert.set_f32(3, row.pos_z);
    insert.set_f32(4, row.orientation);
    insert.set_u16(5, row.map_id);
    insert.set_u32(6, row.display_id);
    insert.set_string(7, row.item_cache.clone());
    insert.set_u8(8, row.race);
    insert.set_u8(9, row.class);
    insert.set_u8(10, row.sex);
    insert.set_u8(11, row.flags);
    insert.set_u8(12, row.dynamic_flags);
    insert.set_u32(13, row.ghost_time);
    insert.set_u8(14, row.corpse_type);
    insert.set_u32(15, row.instance_id);
    transaction.append_expect_rows_affected(insert, 1);

    transaction
}

impl MariaDbMapCorpsePersistenceAdapterLikeCpp {
    pub fn new(character_db: Arc<CharacterDatabase>) -> Self {
        Self { character_db }
    }
}

impl MapCorpsePersistencePortLikeCpp for MariaDbMapCorpsePersistenceAdapterLikeCpp {
    fn persist_corpse_like_cpp<'a>(
        &'a self,
        row: MapCorpseSaveRowLikeCpp,
    ) -> PersistenceFutureLikeCpp<'a, MapCorpseSaveOutcomeLikeCpp> {
        Box::pin(async move {
            let transaction = map_corpse_save_transaction_like_cpp(&row);
            match self.character_db.commit_transaction(transaction).await {
                Ok(()) => MapCorpseSaveOutcomeLikeCpp::Saved,
                Err(error) => MapCorpseSaveOutcomeLikeCpp::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }

    fn load_map_corpses_like_cpp<'a>(
        &'a self,
        request: MapCorpseLoadRequestLikeCpp,
    ) -> PersistenceFutureLikeCpp<'a, MapCorpseLoadOutcomeLikeCpp> {
        Box::pin(async move {
            let [corpse_stmt, phase_stmt, customization_stmt] =
                map_corpse_load_statements_like_cpp(request);
            let mut corpse_result = match self.character_db.query(&corpse_stmt).await {
                Ok(result) => result,
                Err(error) => {
                    return MapCorpseLoadOutcomeLikeCpp::Failed {
                        reason: error.to_string(),
                    };
                }
            };

            if corpse_result.is_empty() {
                return MapCorpseLoadOutcomeLikeCpp::Loaded {
                    corpses: Vec::new(),
                    phases: MapCorpseAuxiliaryLoadOutcomeLikeCpp::Loaded(Vec::new()),
                    customizations: MapCorpseAuxiliaryLoadOutcomeLikeCpp::Loaded(Vec::new()),
                };
            }

            let mut corpses = Vec::with_capacity(corpse_result.row_count_like_cpp());
            loop {
                corpses.push(MapCorpseLoadRowLikeCpp {
                    pos_x: corpse_result.try_read::<f32>(0).unwrap_or(f32::NAN),
                    pos_y: corpse_result.try_read::<f32>(1).unwrap_or(f32::NAN),
                    pos_z: corpse_result.try_read::<f32>(2).unwrap_or(f32::NAN),
                    orientation: corpse_result.try_read::<f32>(3).unwrap_or(f32::NAN),
                    map_id: corpse_result
                        .try_read::<u16>(4)
                        .unwrap_or(request.map_id as u16),
                    display_id: corpse_result.try_read::<u32>(5).unwrap_or(0),
                    item_cache: corpse_result.read_string(6),
                    race: corpse_result.try_read::<u8>(7).unwrap_or(0),
                    class: corpse_result.try_read::<u8>(8).unwrap_or(0),
                    sex: corpse_result.try_read::<u8>(9).unwrap_or(0),
                    flags: corpse_result.try_read::<u8>(10).unwrap_or(0),
                    dynamic_flags: corpse_result.try_read::<u8>(11).unwrap_or(0),
                    ghost_time: corpse_result.try_read::<u32>(12).unwrap_or(0),
                    corpse_type: corpse_result.try_read::<u8>(13).unwrap_or(u8::MAX),
                    instance_id: corpse_result
                        .try_read::<u32>(14)
                        .unwrap_or(request.instance_id),
                    owner_guid: corpse_result.try_read::<u64>(15).unwrap_or(0),
                });
                if !corpse_result.next_row() {
                    break;
                }
            }

            let phases = match self.character_db.query(&phase_stmt).await {
                Ok(mut result) => {
                    let mut rows = Vec::new();
                    if !result.is_empty() {
                        loop {
                            rows.push(MapCorpsePhaseLoadRowLikeCpp {
                                owner_guid: result.try_read::<u64>(0).unwrap_or(0),
                                phase_id: result.try_read::<u32>(1).unwrap_or(0),
                            });
                            if !result.next_row() {
                                break;
                            }
                        }
                    }
                    MapCorpseAuxiliaryLoadOutcomeLikeCpp::Loaded(rows)
                }
                Err(error) => MapCorpseAuxiliaryLoadOutcomeLikeCpp::Failed {
                    reason: error.to_string(),
                },
            };

            let customizations = match self.character_db.query(&customization_stmt).await {
                Ok(mut result) => {
                    let mut rows = Vec::new();
                    if !result.is_empty() {
                        loop {
                            rows.push(MapCorpseCustomizationLoadRowLikeCpp {
                                owner_guid: result.try_read::<u64>(0).unwrap_or(0),
                                option_id: result.try_read::<u32>(1).unwrap_or(0),
                                choice_id: result.try_read::<u32>(2).unwrap_or(0),
                            });
                            if !result.next_row() {
                                break;
                            }
                        }
                    }
                    MapCorpseAuxiliaryLoadOutcomeLikeCpp::Loaded(rows)
                }
                Err(error) => MapCorpseAuxiliaryLoadOutcomeLikeCpp::Failed {
                    reason: error.to_string(),
                },
            };

            MapCorpseLoadOutcomeLikeCpp::Loaded {
                corpses,
                phases,
                customizations,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqlParam;
    use crate::statements::StatementDef;

    fn save_row_like_cpp() -> MapCorpseSaveRowLikeCpp {
        MapCorpseSaveRowLikeCpp {
            owner_guid: 42,
            pos_x: 1.0,
            pos_y: 2.0,
            pos_z: 3.0,
            orientation: 0.5,
            map_id: 571,
            display_id: 0,
            item_cache: String::new(),
            race: 1,
            class: 1,
            sex: 0,
            flags: 0,
            dynamic_flags: 0,
            ghost_time: 1_000,
            corpse_type: 0,
            instance_id: 0,
        }
    }

    #[test]
    fn corpse_save_deletes_before_inserting_like_cpp() {
        // C++ `Corpse::SaveToDB` opens with `DeleteFromDB(trans)` "to prevent DB
        // data inconsistence problems and duplicates", then appends the insert,
        // all inside one transaction. Four statements: the three deletes that
        // clear the owner's previous corpse, phases and customizations, then the
        // insert.
        let transaction = map_corpse_save_transaction_like_cpp(&save_row_like_cpp());
        assert_eq!(
            transaction.len(),
            4,
            "three deletes for the owner's stale rows plus one insert"
        );
    }

    #[test]
    fn corpse_save_statements_are_keyed_by_the_owner_guid_like_cpp() {
        // The corpse table is keyed by the OWNER's guid counter
        // (`GetOwnerGUID().GetCounter()`), which is what makes delete-then-insert
        // an upsert for that player rather than an unbounded accumulation.
        assert_eq!(
            CharStatements::DEL_CORPSE.sql(),
            "DELETE FROM corpse WHERE guid = ?"
        );
        assert_eq!(
            CharStatements::DEL_CORPSE_PHASES.sql(),
            "DELETE FROM corpse_phases WHERE OwnerGuid = ?"
        );
        let insert = CharStatements::INS_CORPSE.sql();
        assert!(
            insert.starts_with("INSERT INTO corpse (guid, posX, posY, posZ, orientation, mapId"),
            "the insert must keep the C++ column order: {insert}"
        );
        assert_eq!(
            insert.matches('?').count(),
            16,
            "C++ binds exactly sixteen fields"
        );
    }

    #[test]
    fn map_corpse_request_maps_to_cpp_statement_order_and_exact_binds() {
        let statements = map_corpse_load_statements_like_cpp(MapCorpseLoadRequestLikeCpp {
            map_id: 571,
            instance_id: 9,
        });

        assert_eq!(
            statements.each_ref().map(|statement| statement.sql()),
            [
                CharStatements::SEL_CORPSES.sql(),
                CharStatements::SEL_CORPSE_PHASES.sql(),
                CharStatements::SEL_CORPSE_CUSTOMIZATIONS.sql(),
            ]
        );
        for statement in statements {
            assert_eq!(
                statement.params(),
                vec![SqlParam::U32(571), SqlParam::U32(9)]
            );
        }
    }
}
