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

/// C++ `ItemAddedQuestCheck(entry, count)` passes the item entry and nothing else
/// (`Entities/Player/Player.cpp:16533-16536`), so an objective keyed on the
/// template's `QuestLogItemId` is never credited by storing that item. The field
/// appears exactly once in the target build, as a commented-out packet assignment
/// at `:13869`; D-M15 has the detail.
#[tokio::test]
async fn an_item_objective_keyed_on_quest_log_item_id_is_not_credited_like_cpp() {
    let (mut session, _send_rx) = make_session();
    let quest_id = 7_208;
    let stored_item_id = 9_401_u32;
    let quest_log_item_id = 9_501_u32;
    let mut quest = quest_template(quest_id);
    for (storage_index, object_id) in [(0_i8, stored_item_id), (1_i8, quest_log_item_id)] {
        quest.objectives.push(QuestObjective {
            id: quest_id * 10 + storage_index as u32,
            quest_id,
            obj_type: QUEST_OBJECTIVE_ITEM_LIKE_CPP_LOCAL,
            order: storage_index as u8,
            storage_index,
            object_id: object_id as i32,
            amount: 1,
            flags: 0,
            flags2: 0,
            progress_bar_weight: 0.0,
            description: String::new(),
        });
    }
    session.set_quest_store(Arc::new(QuestStore::from_quests_like_cpp([quest])));
    add_active_quest_in_slot(&mut session, quest_id, 0);

    let changed = session
        .apply_quest_item_added_objective_progress_like_cpp(stored_item_id, quest_log_item_id, 1)
        .await;

    assert_eq!(changed, vec![quest_id]);
    // Only storage index 0 is written. Index 1 is not even allocated, because
    // nothing credited it: C++ would have passed only the entry too.
    assert_eq!(
        session.player_quests[&quest_id].objective_counts,
        vec![1],
        "the entry's objective advances and the QuestLogItemId one is untouched"
    );
}

/// One `QUEST_OBJECTIVE_MONSTER` objective on `entry`, with the quest in the log
/// and nothing stored for it.
fn monster_objective_fixture_like_cpp(
    quest_id: u32,
    entry: i32,
) -> (QuestTemplate, PlayerQuestStatus) {
    let mut quest = quest_template(quest_id);
    quest.objectives = vec![QuestObjective {
        id: quest_id * 10,
        quest_id,
        obj_type: QUEST_OBJECTIVE_MONSTER_LIKE_CPP_LOCAL,
        order: 0,
        storage_index: 0,
        object_id: entry,
        amount: 3,
        flags: 0,
        flags2: 0,
        progress_bar_weight: 0.0,
        description: String::new(),
    }];
    let status = PlayerQuestStatus {
        quest_id,
        status: QUEST_STATUS_INCOMPLETE_LIKE_CPP,
        explored: false,
        accept_time_secs: 0,
        end_time_secs: 0,
        objective_counts: vec![0],
        slot: 0,
    };
    (quest, status)
}

fn no_raid_context_like_cpp() -> crate::handlers::quest_rules::RepresentedQuestRaidContextLikeCpp {
    crate::handlers::quest_rules::RepresentedQuestRaidContextLikeCpp {
        in_raid_group: false,
        map_difficulty_id: 0,
        quests_ignore_raid: false,
    }
}

/// C++ `Player::UpdateQuestObjectiveProgress` refuses a `QUEST_OBJECTIVE_MONSTER`
/// credit with an empty victim GUID when the quest carries
/// `QUEST_FLAGS_EX_NO_CREDIT_FOR_PROXY` (`Entities/Player/Player.cpp:16653-16655`).
/// That empty GUID is how `Player::KilledMonster` (`:16568-16570`) marks the credit
/// granted for a `CreatureTemplate::KillCredit` proxy rather than for the unit that
/// actually died.
#[test]
fn no_credit_for_proxy_refuses_only_the_guidless_monster_credit_like_cpp() {
    let (mut quest, status) = monster_objective_fixture_like_cpp(7401, 721);
    let facts = crate::handlers::quest::ResolvedQuestObjectivePlayerFactsLikeCpp::default();
    let allowed = |quest: &QuestTemplate, victim_guid_is_empty: bool| {
        crate::handlers::quest_rules::represented_quest_objective_progress_allowed_like_cpp(
            &status,
            quest,
            0,
            &quest.objectives[0],
            &no_raid_context_like_cpp(),
            victim_guid_is_empty,
            &facts.borrow_like_cpp(),
        )
    };

    // Without the flag, both the real kill and the proxy credit advance.
    assert!(allowed(&quest, false));
    assert!(allowed(&quest, true));

    quest.flags_ex |= wow_data::quest::QUEST_FLAGS_EX_NO_CREDIT_FOR_PROXY_LIKE_CPP;
    assert!(
        allowed(&quest, false),
        "the creature that died still counts"
    );
    assert!(!allowed(&quest, true), "its credit proxies do not");
}

/// The proxy refusal is only for `QUEST_OBJECTIVE_MONSTER`: C++ checks the type
/// before it looks at the GUID, so a flagged quest's other objectives are
/// untouched by a guidless credit.
#[test]
fn no_credit_for_proxy_leaves_other_objective_types_alone_like_cpp() {
    let (mut quest, status) = monster_objective_fixture_like_cpp(7402, 721);
    quest.flags_ex |= wow_data::quest::QUEST_FLAGS_EX_NO_CREDIT_FOR_PROXY_LIKE_CPP;
    quest.objectives[0].obj_type = QUEST_OBJECTIVE_TALKTO_LIKE_CPP_LOCAL;
    let facts = crate::handlers::quest::ResolvedQuestObjectivePlayerFactsLikeCpp::default();
    assert!(
        crate::handlers::quest_rules::represented_quest_objective_progress_allowed_like_cpp(
            &status,
            &quest,
            0,
            &quest.objectives[0],
            &no_raid_context_like_cpp(),
            true,
            &facts.borrow_like_cpp(),
        )
    );
}

/// C++ blocks every objective a raid group cannot earn unless the quest is
/// allowed in raid (`Player.cpp:16644-16646`, `Quests/QuestDef.cpp:511-549`).
#[test]
fn a_raid_group_blocks_a_kill_objective_unless_the_quest_allows_raid_like_cpp() {
    let (mut quest, status) = monster_objective_fixture_like_cpp(7403, 721);
    let facts = crate::handlers::quest::ResolvedQuestObjectivePlayerFactsLikeCpp::default();
    let allowed =
        |quest: &QuestTemplate,
         raid: crate::handlers::quest_rules::RepresentedQuestRaidContextLikeCpp| {
            crate::handlers::quest_rules::represented_quest_objective_progress_allowed_like_cpp(
                &status,
                quest,
                0,
                &quest.objectives[0],
                &raid,
                false,
                &facts.borrow_like_cpp(),
            )
        };
    let in_raid = crate::handlers::quest_rules::RepresentedQuestRaidContextLikeCpp {
        in_raid_group: true,
        ..no_raid_context_like_cpp()
    };

    assert!(
        allowed(&quest, no_raid_context_like_cpp()),
        "solo kill counts"
    );
    assert!(!allowed(&quest, in_raid), "a raid group does not");

    // `Quests.IgnoreRaid` is the operator escape hatch C++ falls back on.
    assert!(allowed(
        &quest,
        crate::handlers::quest_rules::RepresentedQuestRaidContextLikeCpp {
            in_raid_group: true,
            map_difficulty_id: 0,
            quests_ignore_raid: true,
        }
    ));

    // `QUEST_FLAGS_RAID_GROUP_OK` makes the quest a raid quest at any difficulty.
    quest.flags |= wow_data::quest::QUEST_FLAGS_RAID_GROUP_OK_LIKE_CPP;
    assert!(allowed(&quest, in_raid));
}

/// The eight objective types C++ `QuestObjective::CanAlwaysBeProgressedInRaid`
/// lists (`Quests/QuestDef.h:489-507`) are never blocked by a raid group, and
/// nothing else is exempt.
#[test]
fn only_the_cpp_objective_types_are_always_progressable_in_raid_like_cpp() {
    for objective_type in [
        QUEST_OBJECTIVE_ITEM_LIKE_CPP_LOCAL,
        QUEST_OBJECTIVE_CURRENCY_LIKE_CPP_LOCAL,
        QUEST_OBJECTIVE_LEARNSPELL_LIKE_CPP_LOCAL,
        QUEST_OBJECTIVE_MIN_REPUTATION_LIKE_CPP_LOCAL,
        QUEST_OBJECTIVE_MAX_REPUTATION_LIKE_CPP_LOCAL,
        QUEST_OBJECTIVE_MONEY_LIKE_CPP_LOCAL,
        QUEST_OBJECTIVE_HAVE_CURRENCY_LIKE_CPP_LOCAL,
        QUEST_OBJECTIVE_INCREASE_REPUTATION_LIKE_CPP_LOCAL,
    ] {
        assert!(
            crate::handlers::quest_rules::represented_objective_can_always_be_progressed_in_raid_like_cpp(
                objective_type
            ),
            "type {objective_type} is in the C++ list"
        );
    }
    for objective_type in [
        QUEST_OBJECTIVE_MONSTER_LIKE_CPP_LOCAL,
        QUEST_OBJECTIVE_GAMEOBJECT_LIKE_CPP_LOCAL,
        QUEST_OBJECTIVE_TALKTO_LIKE_CPP_LOCAL,
        QUEST_OBJECTIVE_PLAYERKILLS_LIKE_CPP_LOCAL,
        QUEST_OBJECTIVE_AREATRIGGER_LIKE_CPP_LOCAL,
        QUEST_OBJECTIVE_CRITERIA_TREE_LIKE_CPP_LOCAL,
    ] {
        assert!(
            !crate::handlers::quest_rules::represented_objective_can_always_be_progressed_in_raid_like_cpp(
                objective_type
            ),
            "type {objective_type} is not"
        );
    }
}
