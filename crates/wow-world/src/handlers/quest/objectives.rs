// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Quest objective progress and completion.

use super::*;

impl WorldSession {
    pub(crate) async fn quest_source_item_quest_log_item_id_like_cpp(
        &mut self,
        entry_id: u32,
    ) -> u32 {
        if let Some(quest_log_item_id) =
            self.item_template_addon_quest_log_item_id_like_cpp(entry_id)
        {
            return quest_log_item_id;
        }

        let Some(port) = self.item_template_addon_catalog_persistence_port_like_cpp() else {
            return 0;
        };

        let quest_log_item_id = match port
            .load_item_template_addon_loot_metadata_like_cpp(
                wow_persistence::ItemTemplateAddonCatalogRequestLikeCpp {
                    item_entry: entry_id,
                },
            )
            .await
        {
            wow_persistence::ItemTemplateAddonLootMetadataOutcomeLikeCpp::Found(row) => {
                row.quest_log_item_id.try_into().unwrap_or(0)
            }
            wow_persistence::ItemTemplateAddonLootMetadataOutcomeLikeCpp::Missing => 0,
            wow_persistence::ItemTemplateAddonLootMetadataOutcomeLikeCpp::Failed { reason } => {
                warn!(
                    account = self.account_id,
                    entry_id,
                    error = %reason,
                    "QuestConfirmAccept: failed to load item_template_addon QuestLogItemId"
                );
                0
            }
        };
        self.cache_item_template_addon_quest_log_item_id_like_cpp(entry_id, quest_log_item_id);
        quest_log_item_id
    }

    #[cfg(test)]
    pub(crate) async fn apply_quest_item_added_objective_progress_like_cpp(
        &mut self,
        entry_id: u32,
        quest_log_item_id: u32,
        count: u32,
    ) -> Vec<u32> {
        let generators = self.id_generators_for_test_like_cpp();
        self.apply_quest_item_added_objective_progress_with_generator_like_cpp(
            generators.item.as_ref(),
            entry_id,
            quest_log_item_id,
            count,
        )
        .await
    }

    /// C++ `Player::ItemAddedQuestCheck(entry, count)`
    /// (`Entities/Player/Player.cpp:16533-16536`), i.e.
    /// `UpdateQuestObjectiveProgress(QUEST_OBJECTIVE_ITEM, entry, count)`
    /// (`:16631-16772`): every incomplete objective keyed on the item is
    /// credited, and the credit message for `QUEST_OBJECTIVE_ITEM` is the
    /// item push itself, never `SMSG_QUEST_UPDATE_ADD_CREDIT` (`:16676-16678`).
    ///
    /// The target build reads no objective flag here. `QuestObjective::Flags2`
    /// is loaded (`Quests/QuestDef.cpp:262`) and forwarded to the client
    /// (`Server/Packets/QuestPackets.cpp:208`) and never consulted again, so no
    /// objective is exempt and no credit stands in for storing the item.
    pub(crate) async fn apply_quest_item_added_objective_progress_with_generator_like_cpp(
        &mut self,
        item_guid_generator: &wow_core::ObjectGuidGenerator,
        entry_id: u32,
        quest_log_item_id: u32,
        count: u32,
    ) -> Vec<u32> {
        use wow_packet::packets::quest::QuestUpdateComplete;

        let Some(quest_store) = self.quests.store.clone() else {
            return Vec::new();
        };
        let count = i32::try_from(count).unwrap_or(i32::MAX);
        let entry_object_id = i32::try_from(entry_id).unwrap_or(i32::MAX);
        // C++ `ItemAddedQuestCheck(entry, count)` passes the item entry and nothing
        // else (`Entities/Player/Player.cpp:16533-16536`). `QuestLogItemId` never
        // reaches `UpdateQuestObjectiveProgress`: the field appears exactly once in
        // the target build, as a commented-out packet assignment at `:13869`. See
        // D-M15 for why the parameter is still carried here.
        let objective_ids = [entry_object_id];
        let _ = quest_log_item_id;

        // The loop below walks the whole quest log, so the live state its
        // completion rules read is resolved for every quest in it, once, before the
        // owner is borrowed mutably.
        let active_quests: Vec<_> = self
            .player_quest_gameplay_snapshot_like_cpp()
            .map(|state| {
                state
                    .statuses_like_cpp()
                    .keys()
                    .filter_map(|quest_id| quest_store.get(*quest_id).cloned())
                    .collect()
            })
            .unwrap_or_default();
        let player_facts =
            self.resolved_quest_objective_player_facts_for_quests_like_cpp(active_quests.iter());
        let Some((changed_quest_ids, quests_to_complete)) = self
            .mutate_player_quest_gameplay_like_cpp(|state| {
                let player_facts = player_facts.borrow_like_cpp();
                let rewarded_quest_ids = state.rewarded_quest_ids_like_cpp().clone();
                let mut changed_quest_ids = Vec::new();
                let mut quests_to_complete = Vec::new();
                for status in state.statuses_mut_like_cpp() {
                    if status.status != QUEST_STATUS_INCOMPLETE_LIKE_CPP {
                        continue;
                    }

                    let Some(quest) = quest_store.get(status.quest_id) else {
                        continue;
                    };

                    for (objective_index, objective) in quest.objectives.iter().enumerate() {
                        if objective.obj_type != QUEST_OBJECTIVE_ITEM_LIKE_CPP_LOCAL {
                            continue;
                        }
                        if !objective_ids.contains(&objective.object_id) {
                            continue;
                        }
                        if !crate::handlers::quest_rules::represented_quest_objective_completable_like_cpp(
                            status,
                            quest,
                            objective_index,
                            &player_facts,
                        ) {
                            continue;
                        }

                        let Ok(storage_index) = usize::try_from(objective.storage_index) else {
                            continue;
                        };
                        if status.objective_counts.len() <= storage_index {
                            status.objective_counts.resize(storage_index + 1, 0);
                        }
                        let current = status.objective_counts[storage_index];
                        if current >= objective.amount {
                            continue;
                        }
                        status.objective_counts[storage_index] =
                            current.saturating_add(count).clamp(0, objective.amount);
                        let new_count = status.objective_counts[storage_index];
                        if !changed_quest_ids.contains(&status.quest_id) {
                            changed_quest_ids.push(status.quest_id);
                        }
                        let quest_already_rewarded =
                            rewarded_quest_ids.contains(&status.quest_id);
                        if new_count >= objective.amount
                            && crate::handlers::quest_rules::represented_can_complete_quest_after_objective_like_cpp(
                                status,
                                quest,
                                objective.id,
                                quest_already_rewarded,
                                &player_facts,
                            )
                        {
                            quests_to_complete.push(status.quest_id);
                        }
                    }
                }
                (changed_quest_ids, quests_to_complete)
            })
        else {
            return Vec::new();
        };
        for quest_id in quests_to_complete {
            if let Some(quest) = quest_store.get(quest_id).cloned() {
                let completed = self
                    .complete_represented_quest_after_add_with_generator_like_cpp(
                        item_guid_generator,
                        &quest,
                    )
                    .await;
                if completed
                    && self
                        .player_quest_gameplay_snapshot_like_cpp()
                        .is_some_and(|state| {
                            state
                                .statuses_like_cpp()
                                .get(&quest_id)
                                .is_some_and(|status| {
                                    status.status == QUEST_STATUS_COMPLETE_LIKE_CPP
                                })
                        })
                {
                    self.send_packet(&QuestUpdateComplete { quest_id });
                }
            }
        }
        self.sync_player_registry_state_like_cpp();
        changed_quest_ids
    }

    /// C++ `Player::ItemRemovedQuestCheck`: after the inventory mutation,
    /// recompute matching item objectives from carried (non-bank) contents and
    /// move completed quests back to incomplete when the requirement is lost.
    pub(crate) fn apply_quest_item_removed_like_cpp(&mut self, entry_id: u32) -> Option<Vec<u32>> {
        self.invalidate_player_quest_status_authority_like_cpp();
        let Some(quest_store) = self.quests.store.clone() else {
            return Some(Vec::new());
        };
        let new_non_bank_item_count = self.represented_non_bank_item_count_like_cpp(entry_id)?;
        let player_facts = self.resolved_quest_objective_player_facts_for_quest_log_like_cpp();
        let changed_quest_ids = self.mutate_player_quest_gameplay_like_cpp(|state| {
            let mut statuses = state
                .statuses_like_cpp()
                .iter()
                .map(|(&id, status)| (id, status.clone()))
                .collect();
            let changed =
                crate::handlers::quest_rules::apply_quest_item_removed_to_statuses_like_cpp(
                    quest_store.as_ref(),
                    &mut statuses,
                    entry_id,
                    new_non_bank_item_count,
                    &player_facts.borrow_like_cpp(),
                );
            state.replace_statuses_like_cpp(
                statuses.into_iter().collect(),
                state.status_authority_complete_like_cpp(),
            );
            changed
        })?;
        let snapshot = self.player_quest_gameplay_snapshot_like_cpp()?;
        let changed_slots = changed_quest_ids
            .iter()
            .filter_map(|quest_id| {
                snapshot
                    .statuses_like_cpp()
                    .get(quest_id)
                    .map(|status| status.slot)
            })
            .collect::<Vec<_>>();
        for slot in changed_slots {
            self.send_represented_quest_log_slot_update_like_cpp(slot);
        }
        let _ = self.update_visible_gameobjects_or_spell_clicks_like_cpp();
        self.sync_player_registry_state_like_cpp();
        Some(changed_quest_ids)
    }

    pub(crate) fn apply_quest_item_added_state_like_cpp(
        &mut self,
        entry_id: u32,
        quest_log_item_id: u32,
        count: u32,
    ) -> Vec<u32> {
        self.invalidate_player_quest_status_authority_like_cpp();
        let Some(quest_store) = self.quests.store.clone() else {
            return Vec::new();
        };
        let player_facts = self.resolved_quest_objective_player_facts_for_quest_log_like_cpp();
        self.mutate_player_quest_gameplay_like_cpp(|state| {
            let rewarded = state
                .rewarded_quest_ids_like_cpp()
                .iter()
                .copied()
                .collect();
            let mut statuses = state
                .statuses_like_cpp()
                .iter()
                .map(|(&id, status)| (id, status.clone()))
                .collect();
            let changed = crate::handlers::quest_rules::apply_quest_item_added_to_statuses_like_cpp(
                quest_store.as_ref(),
                &rewarded,
                &mut statuses,
                entry_id,
                quest_log_item_id,
                count,
                &player_facts.borrow_like_cpp(),
            );
            state.replace_statuses_like_cpp(
                statuses.into_iter().collect(),
                state.status_authority_complete_like_cpp(),
            );
            changed
        })
        .unwrap_or_default()
    }

    pub(crate) fn publish_quest_item_added_status_changes_like_cpp(
        &mut self,
        changed_quest_ids: &[u32],
    ) {
        use wow_packet::packets::quest::QuestUpdateComplete;

        let Some(state) = self.player_quest_gameplay_snapshot_like_cpp() else {
            return;
        };
        let mut changed_slots = changed_quest_ids
            .iter()
            .filter_map(|quest_id| {
                state
                    .statuses_like_cpp()
                    .get(quest_id)
                    .map(|status| status.slot)
            })
            .collect::<Vec<_>>();
        changed_slots.sort_unstable();
        changed_slots.dedup();
        for slot in changed_slots {
            self.send_represented_quest_log_slot_update_like_cpp(slot);
        }
        for &quest_id in changed_quest_ids {
            if state
                .statuses_like_cpp()
                .get(&quest_id)
                .is_some_and(|status| status.status == QUEST_STATUS_COMPLETE_LIKE_CPP)
            {
                self.send_packet(&QuestUpdateComplete { quest_id });
            }
        }
        self.sync_player_registry_state_like_cpp();
    }
}
