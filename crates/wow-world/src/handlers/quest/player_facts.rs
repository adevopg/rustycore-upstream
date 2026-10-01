// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! The live player state a quest's objective-completion rules read.

use super::*;
use crate::handlers::quest_rules::RepresentedQuestObjectivePlayerFactsLikeCpp;

/// Owned form of [`RepresentedQuestObjectivePlayerFactsLikeCpp`], resolved once per
/// quest and borrowed into the rules.
///
/// C++ `Player::IsQuestObjectiveComplete` reaches straight into the Player for
/// these (`Entities/Player/Player.cpp:16970-16998`). The rules are pure, so the
/// owner resolves them first and only for the ids that quest's own objectives
/// name — one or two entries in practice, not the whole reputation list.
#[derive(Debug, Default, Clone)]
pub(crate) struct ResolvedQuestObjectivePlayerFactsLikeCpp {
    money: u64,
    reputation_standings: HashMap<i32, i32>,
    known_spell_ids: HashSet<i32>,
    currency_quantities: HashMap<i32, u32>,
}

impl ResolvedQuestObjectivePlayerFactsLikeCpp {
    /// Build the facts a scenario wants without a session behind them.
    #[cfg(test)]
    pub(crate) fn for_test_like_cpp(
        money: u64,
        reputation_standings: impl IntoIterator<Item = (i32, i32)>,
        known_spell_ids: impl IntoIterator<Item = i32>,
        currency_quantities: impl IntoIterator<Item = (i32, u32)>,
    ) -> Self {
        Self {
            money,
            reputation_standings: reputation_standings.into_iter().collect(),
            known_spell_ids: known_spell_ids.into_iter().collect(),
            currency_quantities: currency_quantities.into_iter().collect(),
        }
    }

    pub(crate) fn borrow_like_cpp(&self) -> RepresentedQuestObjectivePlayerFactsLikeCpp<'_> {
        RepresentedQuestObjectivePlayerFactsLikeCpp {
            money: self.money,
            reputation_standings: &self.reputation_standings,
            known_spell_ids: &self.known_spell_ids,
            currency_quantities: &self.currency_quantities,
        }
    }
}

impl WorldSession {
    /// Resolve the live state `quest`'s objectives need, and nothing else.
    ///
    /// A quest with no money, reputation, spell or currency objective resolves to
    /// the default, which costs no session read at all: those four types are rare
    /// (in the installed world database only `MIN_REPUTATION` and `MONEY` appear),
    /// and every other objective type is decided from stored progress.
    pub(crate) fn resolved_quest_objective_player_facts_like_cpp(
        &self,
        quest: &wow_data::quest::QuestTemplate,
    ) -> ResolvedQuestObjectivePlayerFactsLikeCpp {
        self.resolved_quest_objective_player_facts_for_quests_like_cpp(std::iter::once(quest))
    }

    /// The same, for every quest currently in the player's log.
    ///
    /// The item appliers walk the whole log rather than one quest, so they resolve
    /// the live state once for all of it before the owner is borrowed mutably.
    pub(crate) fn resolved_quest_objective_player_facts_for_quest_log_like_cpp(
        &self,
    ) -> ResolvedQuestObjectivePlayerFactsLikeCpp {
        let Some(store) = self.quests.store.clone() else {
            return ResolvedQuestObjectivePlayerFactsLikeCpp::default();
        };
        let quests: Vec<_> = self
            .player_quest_gameplay_snapshot_like_cpp()
            .map(|state| {
                state
                    .statuses_like_cpp()
                    .keys()
                    .filter_map(|quest_id| store.get(*quest_id).cloned())
                    .collect()
            })
            .unwrap_or_default();
        self.resolved_quest_objective_player_facts_for_quests_like_cpp(quests.iter())
    }

    /// The same, for every quest a rule is about to walk — the appliers iterate
    /// the whole quest log, so one merged resolution covers them all. The maps are
    /// keyed by `QuestObjective::ObjectID`, so merging is just insertion.
    pub(crate) fn resolved_quest_objective_player_facts_for_quests_like_cpp<'a>(
        &self,
        quests: impl Iterator<Item = &'a wow_data::quest::QuestTemplate>,
    ) -> ResolvedQuestObjectivePlayerFactsLikeCpp {
        let mut facts = ResolvedQuestObjectivePlayerFactsLikeCpp::default();
        let mut wants_money = false;
        let mut faction_ids = Vec::new();
        let mut spell_ids = Vec::new();
        let mut currency_ids = Vec::new();
        for objective in quests.flat_map(|quest| quest.objectives.iter()) {
            match objective.obj_type {
                QUEST_OBJECTIVE_MONEY_LIKE_CPP_LOCAL => wants_money = true,
                QUEST_OBJECTIVE_MIN_REPUTATION_LIKE_CPP_LOCAL
                | QUEST_OBJECTIVE_MAX_REPUTATION_LIKE_CPP_LOCAL => {
                    faction_ids.push(objective.object_id)
                }
                QUEST_OBJECTIVE_LEARNSPELL_LIKE_CPP_LOCAL => spell_ids.push(objective.object_id),
                QUEST_OBJECTIVE_CURRENCY_LIKE_CPP_LOCAL => currency_ids.push(objective.object_id),
                _ => {}
            }
        }

        if wants_money {
            // C++ `Player::GetMoney`. An unresolved owner leaves it at zero, which
            // keeps the objective incomplete rather than granting it.
            facts.money = self.resolved_player_money_like_cpp().unwrap_or(0);
        }
        if !faction_ids.is_empty() {
            let race = self.player_race_like_cpp();
            let class_id = self.player_class_like_cpp();
            let faction_store = self.faction_store().cloned();
            let standings = self.with_reputation_mgr_like_cpp(|manager| {
                let mut standings = HashMap::new();
                for faction_id in &faction_ids {
                    // C++ `ReputationMgr::GetReputation(uint32)` logs and returns 0
                    // for an unknown faction id (`ReputationMgr.cpp:114-125`).
                    let standing = u32::try_from(*faction_id)
                        .ok()
                        .and_then(|faction_id| {
                            faction_store.as_deref()?.get(faction_id).map(|entry| {
                                manager.reputation_for_faction_like_cpp(entry, race, class_id)
                            })
                        })
                        .unwrap_or(0);
                    standings.insert(*faction_id, standing);
                }
                standings
            });
            facts.reputation_standings = standings.unwrap_or_default();
        }
        if !spell_ids.is_empty() {
            let known = self.known_spells_like_cpp();
            facts.known_spell_ids = spell_ids
                .into_iter()
                .filter(|spell_id| known.contains(spell_id))
                .collect();
        }
        if !currency_ids.is_empty() {
            if let Some(currencies) = self.player_currencies_like_cpp() {
                for currency_id in currency_ids {
                    if let Some(currency) = u32::try_from(currency_id)
                        .ok()
                        .and_then(|id| currencies.get(&id))
                    {
                        facts
                            .currency_quantities
                            .insert(currency_id, currency.quantity);
                    }
                }
            }
        }
        facts
    }
}
