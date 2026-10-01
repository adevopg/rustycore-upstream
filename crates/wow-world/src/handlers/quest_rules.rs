// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Quest handler rules that read no session state.
//!
//! Moved out of the owner under #680. Every one was already receiver-free,
//! so it cannot read or write the owner's state: these are rules, not owner
//! behaviour. Bodies and signatures are unchanged.

use crate::conditions::QUEST_STATUS_COMPLETE_LIKE_CPP;
use crate::conditions::QUEST_STATUS_INCOMPLETE_LIKE_CPP;
use crate::handlers::quest::PlayerQuestStatus;
use crate::handlers::quest::QUEST_FLAGS_COMPLETION_AREA_TRIGGER_LIKE_CPP;
use crate::handlers::quest::QUEST_FLAGS_COMPLETION_EVENT_LIKE_CPP;
use crate::handlers::quest::QUEST_FLAGS_EX_IS_WORLD_QUEST_LIKE_CPP;
use crate::handlers::quest::QUEST_FLAGS_EX_REWARDS_IGNORE_CAPS_LIKE_CPP;
use crate::handlers::quest::QUEST_OBJECTIVE_AREA_TRIGGER_ENTER_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_AREA_TRIGGER_EXIT_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_AREATRIGGER_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_CRITERIA_TREE_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_CURRENCY_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_DEFEATBATTLEPET_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_FLAG_OPTIONAL_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_FLAG_PART_OF_PROGRESS_BAR_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_FLAG_SEQUENCED_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_GAMEOBJECT_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_HAVE_CURRENCY_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_INCREASE_REPUTATION_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_ITEM_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_LEARNSPELL_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_MAX_REPUTATION_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_MIN_REPUTATION_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_MONEY_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_MONSTER_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_OBTAIN_CURRENCY_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_PLAYERKILLS_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_PROGRESS_BAR_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_TALKTO_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_WINPETBATTLEAGAINSTNPC_LIKE_CPP_LOCAL;
use crate::handlers::quest::QUEST_OBJECTIVE_WINPVPPETBATTLES_LIKE_CPP_LOCAL;
use crate::handlers::quest::QuestChoiceItemLikeCpp;
use crate::session::*;
use std::collections::HashMap;
use std::collections::HashSet;
use wow_core::GameTime;
use wow_data::quest::QuestStore;

/// The live player state C++ `Player::IsQuestObjectiveComplete` reads for the five
/// objective types that are not decided by stored progress
/// (`Entities/Player/Player.cpp:16970-16998`): a faction's standing, the carried
/// money, whether a spell is known and a currency's quantity.
///
/// Resolved by the owner for one quest's own objectives and borrowed in, so these
/// rules stay pure and no reader of session state leaks into them. The maps hold
/// only the ids that quest names, which is one or two entries in practice.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RepresentedQuestObjectivePlayerFactsLikeCpp<'a> {
    /// C++ `Player::GetMoney`.
    pub money: u64,
    /// C++ `ReputationMgr::GetReputation(faction_id)`, by `QuestObjective::ObjectID`.
    pub reputation_standings: &'a HashMap<i32, i32>,
    /// C++ `Player::HasSpell(ObjectID)`, for the spell ids this quest names.
    pub known_spell_ids: &'a HashSet<i32>,
    /// C++ `Player::GetCurrencyQuantity(ObjectID)`, by `QuestObjective::ObjectID`.
    pub currency_quantities: &'a HashMap<i32, u32>,
}

impl RepresentedQuestObjectivePlayerFactsLikeCpp<'_> {
    /// C++ `Player::HasEnoughMoney(int64)` (`Entities/Player/Player.h:1663-1664`):
    /// a negative requirement is always satisfied.
    fn has_enough_money_like_cpp(&self, amount: i32) -> bool {
        amount < 0 || self.money >= amount as u64
    }

    /// C++ `Player::HasCurrency` (`Entities/Player/Player.cpp:7250-7254`): the
    /// currency has to be in the storage *and* at least the amount.
    fn has_currency_like_cpp(&self, currency_id: i32, amount: i32) -> bool {
        self.currency_quantities
            .get(&currency_id)
            .is_some_and(|quantity| i64::from(*quantity) >= i64::from(amount))
    }

    /// A faction the owner could not resolve reads as `0`, which is what C++
    /// `ReputationMgr::GetReputation(uint32)` returns for an id that is not in
    /// `FactionStore` (`Reputation/ReputationMgr.cpp:114-125`). For a faction that
    /// *is* in the store the owner always resolves a value, because
    /// `ReputationMgr::Initialize` gives every one of them a `FactionState`.
    fn reputation_like_cpp(&self, faction_id: i32) -> i32 {
        self.reputation_standings
            .get(&faction_id)
            .copied()
            .unwrap_or(0)
    }
}

/// Facts for a caller whose objectives are all decided by stored progress.
///
/// C++ reads the Player unconditionally, but the five live-state branches of
/// `IsQuestObjectiveComplete` cannot be reached from a scan restricted to one
/// counter-based objective type, so there is nothing to read and no session
/// lookup to pay for. Named rather than `Default` so a caller that *does* need
/// the player cannot pick it by accident.
pub(crate) fn stored_progress_only_player_facts_like_cpp()
-> RepresentedQuestObjectivePlayerFactsLikeCpp<'static> {
    static REPUTATIONS: std::sync::LazyLock<HashMap<i32, i32>> =
        std::sync::LazyLock::new(HashMap::new);
    static SPELLS: std::sync::LazyLock<HashSet<i32>> = std::sync::LazyLock::new(HashSet::new);
    static CURRENCIES: std::sync::LazyLock<HashMap<i32, u32>> =
        std::sync::LazyLock::new(HashMap::new);
    RepresentedQuestObjectivePlayerFactsLikeCpp {
        money: 0,
        reputation_standings: &REPUTATIONS,
        known_spell_ids: &SPELLS,
        currency_quantities: &CURRENCIES,
    }
}

/// The raid context C++ `Player::UpdateQuestObjectiveProgress` reads at
/// `Entities/Player/Player.cpp:16644-16646`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RepresentedQuestRaidContextLikeCpp {
    /// C++ `GetGroup() && GetGroup()->isRaidGroup()`.
    pub in_raid_group: bool,
    /// C++ `GetMap()->GetDifficultyID()`.
    pub map_difficulty_id: u8,
    /// C++ `CONFIG_QUEST_IGNORE_RAID` (`Quests.IgnoreRaid`).
    pub quests_ignore_raid: bool,
}

/// C++ `QuestObjective::CanAlwaysBeProgressedInRaid` (`Quests/QuestDef.h:489-507`):
/// the objective types a raid group never blocks, because none of them is earned
/// by being somewhere or killing something.
pub(crate) fn represented_objective_can_always_be_progressed_in_raid_like_cpp(
    objective_type: u8,
) -> bool {
    matches!(
        objective_type,
        QUEST_OBJECTIVE_ITEM_LIKE_CPP_LOCAL
            | QUEST_OBJECTIVE_CURRENCY_LIKE_CPP_LOCAL
            | QUEST_OBJECTIVE_LEARNSPELL_LIKE_CPP_LOCAL
            | QUEST_OBJECTIVE_MIN_REPUTATION_LIKE_CPP_LOCAL
            | QUEST_OBJECTIVE_MAX_REPUTATION_LIKE_CPP_LOCAL
            | QUEST_OBJECTIVE_MONEY_LIKE_CPP_LOCAL
            | QUEST_OBJECTIVE_HAVE_CURRENCY_LIKE_CPP_LOCAL
            | QUEST_OBJECTIVE_INCREASE_REPUTATION_LIKE_CPP_LOCAL
    )
}

/// Every gate C++ `Player::UpdateQuestObjectiveProgress` applies to one matched
/// objective before it touches its progress (`Entities/Player/Player.cpp:16644-16655`),
/// in that order:
///
/// 1. the raid gate: unless the type can always progress in a raid, a raid group
///    blocks the objective for a quest that is not allowed in raid;
/// 2. `IsQuestObjectiveCompletable`, which the sequenced and progress-bar rules own;
/// 3. `QUEST_FLAGS_EX_NO_CREDIT_FOR_PROXY`, which refuses a `QUEST_OBJECTIVE_MONSTER`
///    credit carrying no victim — that empty GUID is exactly how C++
///    `Player::KilledMonster` (`:16568-16570`) marks the credit it grants for a
///    `CreatureTemplate::KillCredit` proxy rather than for the unit that died.
///
/// Returns true when the objective may take the progress.
pub(crate) fn represented_quest_objective_progress_allowed_like_cpp(
    status: &PlayerQuestStatus,
    quest: &wow_data::quest::QuestTemplate,
    objective_index: usize,
    objective: &wow_data::quest::QuestObjective,
    raid: &RepresentedQuestRaidContextLikeCpp,
    victim_guid_is_empty: bool,
    facts: &RepresentedQuestObjectivePlayerFactsLikeCpp<'_>,
) -> bool {
    if !represented_objective_can_always_be_progressed_in_raid_like_cpp(objective.obj_type)
        && raid.in_raid_group
        && !quest.is_allowed_in_raid_like_cpp(raid.map_difficulty_id, raid.quests_ignore_raid)
    {
        return false;
    }
    if !represented_quest_objective_completable_like_cpp(status, quest, objective_index, facts) {
        return false;
    }
    if quest.has_no_credit_for_proxy_like_cpp()
        && objective.obj_type == QUEST_OBJECTIVE_MONSTER_LIKE_CPP_LOCAL
        && victim_guid_is_empty
    {
        return false;
    }
    true
}

/// Is this `QUEST_OBJECTIVE_GAMEOBJECT` objective still waiting on the player?
///
/// C++ `Player::HasQuestForGO` asks `IsQuestObjectiveCompletable` and
/// `!IsQuestObjectiveComplete` for the matching objective
/// (`Entities/Player/Player.cpp:16806-16830`). Both questions belong together and
/// both are answered from stored progress for this type, so the rule owns the
/// pairing and the caller does not have to resolve player facts it cannot need.
pub(crate) fn represented_gameobject_objective_is_pending_like_cpp(
    status: &PlayerQuestStatus,
    quest: &wow_data::quest::QuestTemplate,
    objective_index: usize,
    objective: &wow_data::quest::QuestObjective,
) -> bool {
    let facts = stored_progress_only_player_facts_like_cpp();
    represented_quest_objective_completable_like_cpp(status, quest, objective_index, &facts)
        && !represented_quest_objective_complete_like_cpp(status, quest, objective, &facts)
}

/// Pure form of C++ `Player::ItemAddedQuestCheck(entry, count)`
/// (`Entities/Player/Player.cpp:16533-16536`), which is
/// `UpdateQuestObjectiveProgress(QUEST_OBJECTIVE_ITEM, entry, count)`.
///
/// That loop (`:16631-16772`) walks every objective matching the
/// `(QUEST_OBJECTIVE_ITEM, objectId)` key and credits each one. The target
/// build never reads `QuestObjective::Flags2`: it is loaded in
/// `Quests/QuestDef.cpp:262` and written to the client in
/// `Server/Packets/QuestPackets.cpp:208`, and nothing else looks at it, so
/// there is no objective an item-store may skip and no objective whose credit
/// replaces storing the item.
pub(crate) fn apply_quest_item_added_to_statuses_like_cpp(
    quest_store: &QuestStore,
    rewarded_quests: &HashSet<u32>,
    player_quests: &mut HashMap<u32, PlayerQuestStatus>,
    entry_id: u32,
    quest_log_item_id: u32,
    count: u32,
    facts: &RepresentedQuestObjectivePlayerFactsLikeCpp<'_>,
) -> Vec<u32> {
    let entry_object_id = i32::try_from(entry_id).unwrap_or(i32::MAX);
    // C++ `ItemAddedQuestCheck` credits the item entry only; see D-M15.
    let objective_ids = [entry_object_id];
    let _ = quest_log_item_id;
    let count = i32::try_from(count).unwrap_or(i32::MAX);
    let mut changed_quest_ids = Vec::new();
    let mut quests_to_complete = Vec::new();

    for status in player_quests.values_mut() {
        if status.status != QUEST_STATUS_INCOMPLETE_LIKE_CPP {
            continue;
        }
        let Some(quest) = quest_store.get(status.quest_id) else {
            continue;
        };
        for (objective_index, objective) in quest.objectives.iter().enumerate() {
            if objective.obj_type != QUEST_OBJECTIVE_ITEM_LIKE_CPP_LOCAL
                || !objective_ids.contains(&objective.object_id)
                || !represented_quest_objective_completable_like_cpp(
                    status,
                    quest,
                    objective_index,
                    facts,
                )
            {
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
            let new_count = current.saturating_add(count).clamp(0, objective.amount);
            status.objective_counts[storage_index] = new_count;
            changed_quest_ids.push(status.quest_id);
            if new_count >= objective.amount
                && represented_can_complete_quest_after_objective_like_cpp(
                    status,
                    quest,
                    objective.id,
                    rewarded_quests.contains(&status.quest_id),
                    facts,
                )
            {
                quests_to_complete.push(status.quest_id);
            }
        }
    }
    for quest_id in quests_to_complete {
        if let Some(status) = player_quests.get_mut(&quest_id) {
            status.status = QUEST_STATUS_COMPLETE_LIKE_CPP;
        }
    }
    changed_quest_ids.sort_unstable();
    changed_quest_ids.dedup();
    changed_quest_ids
}

pub(crate) fn apply_quest_item_removed_to_statuses_like_cpp(
    quest_store: &QuestStore,
    player_quests: &mut HashMap<u32, PlayerQuestStatus>,
    entry_id: u32,
    new_non_bank_item_count: u32,
    facts: &RepresentedQuestObjectivePlayerFactsLikeCpp<'_>,
) -> Vec<u32> {
    let Ok(object_id) = i32::try_from(entry_id) else {
        return Vec::new();
    };
    let new_item_count = i32::try_from(new_non_bank_item_count).unwrap_or(i32::MAX);
    let mut changed_quest_ids = Vec::new();

    for status in player_quests.values_mut() {
        let Some(quest) = quest_store.get(status.quest_id) else {
            continue;
        };
        for (objective_index, objective) in quest.objectives.iter().enumerate() {
            if objective.obj_type != QUEST_OBJECTIVE_ITEM_LIKE_CPP_LOCAL
                || objective.object_id != object_id
                || !represented_quest_objective_completable_like_cpp(
                    status,
                    quest,
                    objective_index,
                    facts,
                )
            {
                continue;
            }
            let Ok(storage_index) = usize::try_from(objective.storage_index) else {
                continue;
            };
            if new_item_count >= objective.amount {
                continue;
            }
            if status.objective_counts.len() <= storage_index {
                status.objective_counts.resize(storage_index + 1, 0);
            }
            if status.objective_counts[storage_index] == new_item_count
                && status.status == QUEST_STATUS_INCOMPLETE_LIKE_CPP
            {
                continue;
            }
            status.objective_counts[storage_index] = new_item_count.max(0);
            status.status = QUEST_STATUS_INCOMPLETE_LIKE_CPP;
            changed_quest_ids.push(status.quest_id);
        }
    }
    changed_quest_ids.sort_unstable();
    changed_quest_ids.dedup();
    changed_quest_ids
}

pub(crate) fn quest_reward_currency_gain_source_like_cpp(
    quest: &wow_data::quest::QuestTemplate,
) -> CurrencyGainSourceLikeCpp {
    if (quest.flags_ex & QUEST_FLAGS_EX_REWARDS_IGNORE_CAPS_LIKE_CPP) != 0 {
        if (quest.flags_ex & QUEST_FLAGS_EX_IS_WORLD_QUEST_LIKE_CPP) != 0 {
            return CurrencyGainSourceLikeCpp::WorldQuestRewardIgnoreCaps;
        }

        return CurrencyGainSourceLikeCpp::QuestRewardIgnoreCaps;
    }

    if quest.is_daily_like_cpp() {
        CurrencyGainSourceLikeCpp::DailyQuestReward
    } else if quest.is_weekly_like_cpp() {
        CurrencyGainSourceLikeCpp::WeeklyQuestReward
    } else if (quest.flags_ex & QUEST_FLAGS_EX_IS_WORLD_QUEST_LIKE_CPP) != 0 {
        CurrencyGainSourceLikeCpp::WorldQuestReward
    } else {
        CurrencyGainSourceLikeCpp::QuestReward
    }
}

pub(crate) fn represented_accept_and_end_time_for_new_quest_like_cpp(
    quest: &wow_data::quest::QuestTemplate,
) -> (i64, i64) {
    let accept_time = GameTime::now().as_secs() as i64;
    let end_time = if quest.limit_time_secs > 0 {
        accept_time.saturating_add(quest.limit_time_secs)
    } else {
        0
    };
    (accept_time, end_time)
}

pub(crate) fn represented_can_complete_quest_after_objective_like_cpp(
    status: &PlayerQuestStatus,
    quest: &wow_data::quest::QuestTemplate,
    ignored_objective_id: u32,
    quest_already_rewarded: bool,
    facts: &RepresentedQuestObjectivePlayerFactsLikeCpp<'_>,
) -> bool {
    if quest.id == 0 {
        return false;
    }

    if !quest.is_repeatable() && quest_already_rewarded {
        return false;
    }

    if status.status != QUEST_STATUS_INCOMPLETE_LIKE_CPP {
        return false;
    }

    for objective in &quest.objectives {
        if ignored_objective_id != 0 && objective.id == ignored_objective_id {
            continue;
        }

        if (objective.flags
            & (QUEST_OBJECTIVE_FLAG_OPTIONAL_LIKE_CPP_LOCAL
                | QUEST_OBJECTIVE_FLAG_PART_OF_PROGRESS_BAR_LIKE_CPP_LOCAL))
            != 0
        {
            continue;
        }

        if !represented_quest_objective_complete_like_cpp(status, quest, objective, facts) {
            return false;
        }
    }

    if (quest.flags
        & (QUEST_FLAGS_COMPLETION_EVENT_LIKE_CPP | QUEST_FLAGS_COMPLETION_AREA_TRIGGER_LIKE_CPP))
        != 0
        && !status.explored
    {
        return false;
    }

    if quest.limit_time_secs > 0 && status.end_time_secs == 0 {
        return false;
    }

    true
}

pub(crate) fn represented_quest_objective_completable_like_cpp(
    status: &PlayerQuestStatus,
    quest: &wow_data::quest::QuestTemplate,
    objective_index: usize,
    facts: &RepresentedQuestObjectivePlayerFactsLikeCpp<'_>,
) -> bool {
    let Some(objective) = quest.objectives.get(objective_index) else {
        return false;
    };

    if (objective.flags & QUEST_OBJECTIVE_FLAG_PART_OF_PROGRESS_BAR_LIKE_CPP_LOCAL) != 0 {
        let Some((progress_bar_index, progress_bar_objective)) =
            quest.objectives.iter().enumerate().find(|(_, other)| {
                other.obj_type == QUEST_OBJECTIVE_PROGRESS_BAR_LIKE_CPP_LOCAL
                    && (other.flags & QUEST_OBJECTIVE_FLAG_PART_OF_PROGRESS_BAR_LIKE_CPP_LOCAL) == 0
            })
        else {
            return false;
        };

        return represented_quest_objective_completable_like_cpp(
            status,
            quest,
            progress_bar_index,
            facts,
        ) && !represented_quest_objective_complete_like_cpp(
            status,
            quest,
            progress_bar_objective,
            facts,
        );
    }

    if objective_index == 0 {
        return true;
    }

    let mut previous_index = objective_index - 1;
    let mut objective_sequence_satisfied = true;
    let mut previous_sequenced_objective_complete = false;
    let mut previous_sequenced_objective_index = None;

    loop {
        let previous_objective = &quest.objectives[previous_index];
        if (previous_objective.flags & QUEST_OBJECTIVE_FLAG_SEQUENCED_LIKE_CPP_LOCAL) != 0 {
            previous_sequenced_objective_index = Some(previous_index);
            previous_sequenced_objective_complete = represented_quest_objective_complete_like_cpp(
                status,
                quest,
                previous_objective,
                facts,
            );
            break;
        }

        if objective_sequence_satisfied {
            objective_sequence_satisfied = represented_quest_objective_complete_like_cpp(
                status,
                quest,
                previous_objective,
                facts,
            ) || (previous_objective.flags
                & (QUEST_OBJECTIVE_FLAG_OPTIONAL_LIKE_CPP_LOCAL
                    | QUEST_OBJECTIVE_FLAG_PART_OF_PROGRESS_BAR_LIKE_CPP_LOCAL))
                != 0;
        }

        if previous_index == 0 {
            break;
        }
        previous_index -= 1;
    }

    if (objective.flags & QUEST_OBJECTIVE_FLAG_SEQUENCED_LIKE_CPP_LOCAL) != 0 {
        if previous_sequenced_objective_index.is_none() {
            return objective_sequence_satisfied;
        }
        if !previous_sequenced_objective_complete || !objective_sequence_satisfied {
            return false;
        }
    } else if !previous_sequenced_objective_complete {
        if let Some(previous_sequenced_objective_index) = previous_sequenced_objective_index {
            if !represented_quest_objective_completable_like_cpp(
                status,
                quest,
                previous_sequenced_objective_index,
                facts,
            ) {
                return false;
            }
        }
    }

    true
}

pub(crate) fn represented_quest_objective_complete_like_cpp(
    status: &PlayerQuestStatus,
    quest: &wow_data::quest::QuestTemplate,
    objective: &wow_data::quest::QuestObjective,
    facts: &RepresentedQuestObjectivePlayerFactsLikeCpp<'_>,
) -> bool {
    match objective.obj_type {
        QUEST_OBJECTIVE_MONSTER_LIKE_CPP_LOCAL
        | QUEST_OBJECTIVE_ITEM_LIKE_CPP_LOCAL
        | QUEST_OBJECTIVE_GAMEOBJECT_LIKE_CPP_LOCAL
        | QUEST_OBJECTIVE_TALKTO_LIKE_CPP_LOCAL
        | QUEST_OBJECTIVE_PLAYERKILLS_LIKE_CPP_LOCAL
        | QUEST_OBJECTIVE_WINPVPPETBATTLES_LIKE_CPP_LOCAL
        | QUEST_OBJECTIVE_HAVE_CURRENCY_LIKE_CPP_LOCAL
        | QUEST_OBJECTIVE_OBTAIN_CURRENCY_LIKE_CPP_LOCAL
        | QUEST_OBJECTIVE_INCREASE_REPUTATION_LIKE_CPP_LOCAL => {
            let Ok(storage_index) = usize::try_from(objective.storage_index) else {
                return false;
            };
            status
                .objective_counts
                .get(storage_index)
                .copied()
                .unwrap_or(0)
                >= objective.amount
        }
        // C++ `Player::IsQuestObjectiveComplete` groups the flag-storing types
        // apart (`Entities/Player/Player.cpp:16982-16990`): any non-zero stored
        // value completes them, and `QuestObjective::IsStoringFlag` is what
        // decides where that value lives. `QUEST_OBJECTIVE_CRITERIA_TREE`
        // belongs here, not with the counter types.
        QUEST_OBJECTIVE_AREATRIGGER_LIKE_CPP_LOCAL
        | QUEST_OBJECTIVE_WINPETBATTLEAGAINSTNPC_LIKE_CPP_LOCAL
        | QUEST_OBJECTIVE_DEFEATBATTLEPET_LIKE_CPP_LOCAL
        | QUEST_OBJECTIVE_CRITERIA_TREE_LIKE_CPP_LOCAL
        | QUEST_OBJECTIVE_AREA_TRIGGER_ENTER_LIKE_CPP_LOCAL
        | QUEST_OBJECTIVE_AREA_TRIGGER_EXIT_LIKE_CPP_LOCAL => {
            let Ok(storage_index) = usize::try_from(objective.storage_index) else {
                return false;
            };
            status
                .objective_counts
                .get(storage_index)
                .copied()
                .unwrap_or(0)
                != 0
        }
        QUEST_OBJECTIVE_PROGRESS_BAR_LIKE_CPP_LOCAL => {
            represented_quest_objective_progress_bar_complete_like_cpp(status, quest)
        }
        // The five types C++ decides from live player state rather than from
        // stored progress (`Entities/Player/Player.cpp:16970-16998`).
        QUEST_OBJECTIVE_MIN_REPUTATION_LIKE_CPP_LOCAL => {
            facts.reputation_like_cpp(objective.object_id) >= objective.amount
        }
        QUEST_OBJECTIVE_MAX_REPUTATION_LIKE_CPP_LOCAL => {
            facts.reputation_like_cpp(objective.object_id) <= objective.amount
        }
        QUEST_OBJECTIVE_MONEY_LIKE_CPP_LOCAL => facts.has_enough_money_like_cpp(objective.amount),
        QUEST_OBJECTIVE_LEARNSPELL_LIKE_CPP_LOCAL => {
            facts.known_spell_ids.contains(&objective.object_id)
        }
        QUEST_OBJECTIVE_CURRENCY_LIKE_CPP_LOCAL => {
            facts.has_currency_like_cpp(objective.object_id, objective.amount)
        }
        // C++ logs an error and refuses an objective type it does not know
        // (`:17003-17006`), which is what failing closed means here.
        _ => false,
    }
}

pub(crate) fn represented_quest_objective_progress_bar_complete_like_cpp(
    status: &PlayerQuestStatus,
    quest: &wow_data::quest::QuestTemplate,
) -> bool {
    let mut progress = 0.0_f32;
    for objective in &quest.objectives {
        if (objective.flags & QUEST_OBJECTIVE_FLAG_PART_OF_PROGRESS_BAR_LIKE_CPP_LOCAL) == 0 {
            continue;
        }

        let Ok(storage_index) = usize::try_from(objective.storage_index) else {
            continue;
        };
        let count = status
            .objective_counts
            .get(storage_index)
            .copied()
            .unwrap_or(0);
        progress += count as f32 * objective.progress_bar_weight;
        if progress >= 100.0 {
            return true;
        }
    }
    false
}
