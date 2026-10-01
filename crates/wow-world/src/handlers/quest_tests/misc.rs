//! Misc scenarios for [`super`].
//!
//! Split out of quest_tests.rs under #628; assertions and
//! registrations are unchanged and shared fixtures stay in the parent module.

use super::*;

#[test]
fn represented_objective_negative_storage_index_does_not_alias_slot_zero_like_cpp() {
    let quest_id = 7101;
    let mut quest = quest_template(quest_id);
    let objective = QuestObjective {
        id: quest_id * 10,
        quest_id,
        obj_type: QUEST_OBJECTIVE_ITEM_LIKE_CPP_LOCAL,
        order: 0,
        storage_index: -1,
        object_id: 55,
        amount: 1,
        flags: 0,
        flags2: 0,
        progress_bar_weight: 0.0,
        description: String::new(),
    };
    quest.objectives = vec![objective.clone()];
    let status = PlayerQuestStatus {
        quest_id,
        status: QUEST_STATUS_INCOMPLETE_LIKE_CPP,
        explored: false,
        accept_time_secs: 0,
        end_time_secs: 0,
        objective_counts: vec![1],
        slot: 0,
    };

    assert!(
        !crate::handlers::quest_rules::represented_quest_objective_complete_like_cpp(
            &status,
            &quest,
            &objective,
            &crate::handlers::quest::ResolvedQuestObjectivePlayerFactsLikeCpp::default()
                .borrow_like_cpp(),
        )
    );
}
#[test]
fn represented_progress_bar_part_objective_stops_when_progress_bar_complete_like_cpp() {
    let quest_id = 7120;
    let mut quest = quest_template(quest_id);
    quest.objectives = vec![
        QuestObjective {
            id: quest_id * 10,
            quest_id,
            obj_type: QUEST_OBJECTIVE_ITEM_LIKE_CPP_LOCAL,
            order: 0,
            storage_index: 0,
            object_id: 99,
            amount: 2,
            flags: QUEST_OBJECTIVE_FLAG_PART_OF_PROGRESS_BAR_LIKE_CPP_LOCAL,
            flags2: 0,
            progress_bar_weight: 50.0,
            description: String::new(),
        },
        QuestObjective {
            id: quest_id * 10 + 1,
            quest_id,
            obj_type: QUEST_OBJECTIVE_PROGRESS_BAR_LIKE_CPP_LOCAL,
            order: 1,
            storage_index: 1,
            object_id: 0,
            amount: 100,
            flags: 0,
            flags2: 0,
            progress_bar_weight: 0.0,
            description: String::new(),
        },
    ];
    let status = PlayerQuestStatus {
        quest_id,
        status: QUEST_STATUS_INCOMPLETE_LIKE_CPP,
        explored: false,
        accept_time_secs: 0,
        end_time_secs: 0,
        objective_counts: vec![2, 0],
        slot: 0,
    };

    assert!(
        !crate::handlers::quest_rules::represented_quest_objective_completable_like_cpp(
            &status,
            &quest,
            0,
            &crate::handlers::quest::ResolvedQuestObjectivePlayerFactsLikeCpp::default()
                .borrow_like_cpp(),
        )
    );
}

/// One objective of `obj_type` on `object_id` wanting `amount`, in a quest the
/// player has in the log with nothing stored for it.
#[cfg(test)]
fn live_state_objective_fixture_like_cpp(
    quest_id: u32,
    obj_type: u8,
    object_id: i32,
    amount: i32,
) -> (QuestTemplate, PlayerQuestStatus) {
    let mut quest = quest_template(quest_id);
    quest.objectives.push(QuestObjective {
        id: quest_id * 10,
        quest_id,
        obj_type,
        order: 0,
        storage_index: 0,
        object_id,
        amount,
        flags: 0,
        flags2: 0,
        progress_bar_weight: 0.0,
        description: String::new(),
    });
    let status = PlayerQuestStatus {
        quest_id,
        status: crate::conditions::QUEST_STATUS_INCOMPLETE_LIKE_CPP,
        explored: false,
        accept_time_secs: 0,
        end_time_secs: 0,
        objective_counts: vec![0],
        slot: 0,
    };
    (quest, status)
}

/// C++ `Player::IsQuestObjectiveComplete`'s five live-state branches
/// (`Entities/Player/Player.cpp:16970-16998`). None of them reads stored progress:
/// each asks the player directly, and each is checked here on both sides of its
/// boundary.
#[test]
fn live_state_objectives_read_the_player_like_cpp() {
    use crate::handlers::quest::ResolvedQuestObjectivePlayerFactsLikeCpp as Facts;
    let complete = |quest: &QuestTemplate, status: &PlayerQuestStatus, facts: &Facts| {
        crate::handlers::quest_rules::represented_quest_objective_complete_like_cpp(
            status,
            quest,
            &quest.objectives[0],
            &facts.borrow_like_cpp(),
        )
    };

    // QUEST_OBJECTIVE_MONEY: `!HasEnoughMoney(Amount)` (`Player.h:1663-1664`).
    let (quest, status) = live_state_objective_fixture_like_cpp(7_201, 8, 0, 300);
    assert!(!complete(
        &quest,
        &status,
        &Facts::for_test_like_cpp(299, [], [], [])
    ));
    assert!(complete(
        &quest,
        &status,
        &Facts::for_test_like_cpp(300, [], [], [])
    ));
    // A negative requirement is always satisfied, money or not.
    let (free, free_status) = live_state_objective_fixture_like_cpp(7_202, 8, 0, -5);
    assert!(complete(
        &free,
        &free_status,
        &Facts::for_test_like_cpp(0, [], [], [])
    ));

    // QUEST_OBJECTIVE_MIN_REPUTATION: `GetReputation(ObjectID) < Amount`.
    let (min_rep, min_status) = live_state_objective_fixture_like_cpp(7_203, 6, 93, 3_000);
    assert!(!complete(
        &min_rep,
        &min_status,
        &Facts::for_test_like_cpp(0, [(93, 2_999)], [], [])
    ));
    assert!(complete(
        &min_rep,
        &min_status,
        &Facts::for_test_like_cpp(0, [(93, 3_000)], [], [])
    ));
    // An unknown faction reads as 0, which C++ returns for a faction with no state.
    assert!(!complete(&min_rep, &min_status, &Facts::default()));

    // QUEST_OBJECTIVE_MAX_REPUTATION: `GetReputation(ObjectID) > Amount`.
    let (max_rep, max_status) = live_state_objective_fixture_like_cpp(7_204, 7, 93, 3_000);
    assert!(complete(
        &max_rep,
        &max_status,
        &Facts::for_test_like_cpp(0, [(93, 3_000)], [], [])
    ));
    assert!(!complete(
        &max_rep,
        &max_status,
        &Facts::for_test_like_cpp(0, [(93, 3_001)], [], [])
    ));

    // QUEST_OBJECTIVE_LEARNSPELL: `!HasSpell(ObjectID)`.
    let (spell, spell_status) = live_state_objective_fixture_like_cpp(7_205, 5, 1_234, 1);
    assert!(!complete(
        &spell,
        &spell_status,
        &Facts::for_test_like_cpp(0, [], [9_999], [])
    ));
    assert!(complete(
        &spell,
        &spell_status,
        &Facts::for_test_like_cpp(0, [], [1_234], [])
    ));

    // QUEST_OBJECTIVE_CURRENCY: `!HasCurrency(ObjectID, Amount)`
    // (`Player.cpp:7250-7254`) — the currency has to be present *and* enough.
    let (currency, currency_status) = live_state_objective_fixture_like_cpp(7_206, 4, 42, 10);
    assert!(!complete(&currency, &currency_status, &Facts::default()));
    assert!(!complete(
        &currency,
        &currency_status,
        &Facts::for_test_like_cpp(0, [], [], [(42, 9)])
    ));
    assert!(complete(
        &currency,
        &currency_status,
        &Facts::for_test_like_cpp(0, [], [], [(42, 10)])
    ));
}

/// The whole point: a quest whose only objective is one of those five now
/// completes, where it could not before.
#[test]
fn a_money_objective_can_complete_the_quest_like_cpp() {
    use crate::handlers::quest::ResolvedQuestObjectivePlayerFactsLikeCpp as Facts;
    let (quest, status) = live_state_objective_fixture_like_cpp(7_207, 8, 0, 300);
    let can_complete = |facts: &Facts| {
        crate::handlers::quest_rules::represented_can_complete_quest_after_objective_like_cpp(
            &status,
            &quest,
            0,
            false,
            &facts.borrow_like_cpp(),
        )
    };

    assert!(!can_complete(&Facts::for_test_like_cpp(299, [], [], [])));
    assert!(can_complete(&Facts::for_test_like_cpp(300, [], [], [])));
}
