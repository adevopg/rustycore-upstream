//! Spell scenarios for [`super`].
//!
//! Split out of spell_tests.rs under #628; assertions and
//! registrations are unchanged and shared fixtures stay in the parent module.

use super::*;

#[test]
fn spell_proc_event_spell_info_is_affected_matches_cpp_zero_family_name() {
    let event_spell = SpellProcEventSpellInfoLikeCpp {
        spell_family_name: 3,
        spell_family_mask: [0, 0, 0, 0],
    };

    assert!(event_spell.is_affected_like_cpp(0, [0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF]));
}
#[test]
fn implicit_proc_aura_info_matches_cpp_trigger_table() {
    assert_eq!(
        implicit_proc_aura_info_like_cpp(aura_types::SPELL_AURA_DUMMY),
        Some(ImplicitProcAuraInfoLikeCpp {
            spell_type_mask: PROC_SPELL_TYPE_MASK_ALL_LIKE_CPP,
            triggered_can_proc: false,
        })
    );
    assert_eq!(
        implicit_proc_aura_info_like_cpp(aura_types::SPELL_AURA_SCHOOL_ABSORB),
        Some(ImplicitProcAuraInfoLikeCpp {
            spell_type_mask: PROC_SPELL_TYPE_MASK_ALL_LIKE_CPP,
            triggered_can_proc: true,
        })
    );
    assert_eq!(
        implicit_proc_aura_info_like_cpp(aura_types::SPELL_AURA_MOD_STEALTH),
        Some(ImplicitProcAuraInfoLikeCpp {
            spell_type_mask: PROC_SPELL_TYPE_DAMAGE_LIKE_CPP | PROC_SPELL_TYPE_NO_DMG_HEAL_LIKE_CPP,
            triggered_can_proc: true,
        })
    );
    assert_eq!(
        implicit_proc_aura_info_like_cpp(aura_types::SPELL_AURA_MOD_CONFUSE),
        Some(ImplicitProcAuraInfoLikeCpp {
            spell_type_mask: PROC_SPELL_TYPE_DAMAGE_LIKE_CPP,
            triggered_can_proc: true,
        })
    );
    assert_eq!(
        implicit_proc_aura_info_like_cpp(aura_types::SPELL_AURA_MOUNTED),
        None
    );
}
#[test]
fn implicit_spell_proc_entry_matches_cpp_default_generation() {
    let mut source = test_implicit_spell_proc_source_like_cpp();
    source.proc_flags = [
        PROC_FLAG_DEAL_HARMFUL_SPELL_LIKE_CPP | PROC_FLAG_KILL_LIKE_CPP,
        0,
    ];
    source.spell_family_name = 42;
    source.proc_chance = 25.0;
    source.proc_cooldown_ms = 1500;
    source.proc_charges = 3;
    source.effects = vec![
        test_implicit_proc_effect_like_cpp(
            0,
            aura_types::SPELL_AURA_PROC_TRIGGER_SPELL,
            [0x10, 0, 0, 0],
        ),
        test_implicit_proc_effect_like_cpp(1, aura_types::SPELL_AURA_MOUNTED, [0, 0, 0, 0]),
    ];

    let entry = implicit_spell_proc_entry_like_cpp(&source).unwrap();

    assert_eq!(entry.proc_flags, source.proc_flags);
    assert_eq!(entry.spell_family_name, 42);
    assert_eq!(entry.spell_family_mask, [0x10, 0, 0, 0]);
    assert_eq!(entry.spell_type_mask, PROC_SPELL_TYPE_MASK_ALL_LIKE_CPP);
    assert_eq!(entry.spell_phase_mask, PROC_SPELL_PHASE_HIT_LIKE_CPP);
    assert_eq!(entry.disable_effects_mask, 1 << 1);
    assert_eq!(entry.attributes_mask, PROC_ATTR_REQ_EXP_OR_HONOR_LIKE_CPP);
    assert_eq!(entry.chance, 25.0);
    assert_eq!(entry.cooldown_ms, 1500);
    assert_eq!(entry.charges, 3);
}
#[test]
fn implicit_spell_proc_entry_sets_special_phase_and_hit_masks_like_cpp() {
    let mut source = test_implicit_spell_proc_source_like_cpp();
    source.proc_flags = [
        PROC_FLAG_DEAL_MELEE_SWING_LIKE_CPP,
        PROC_FLAG_2_CAST_SUCCESSFUL_LIKE_CPP,
    ];
    source.effects = vec![test_implicit_proc_effect_like_cpp(
        0,
        aura_types::SPELL_AURA_MOD_BLOCK_PERCENT,
        [0, 0, 0, 0],
    )];

    let entry = implicit_spell_proc_entry_like_cpp(&source).unwrap();

    assert_eq!(entry.spell_phase_mask, PROC_SPELL_PHASE_CAST_LIKE_CPP);
    assert_eq!(entry.hit_mask, PROC_HIT_BLOCK_LIKE_CPP);

    source.effects = vec![test_implicit_proc_effect_like_cpp(
        0,
        aura_types::SPELL_AURA_REFLECT_SPELLS,
        [0, 0, 0, 0],
    )];
    assert_eq!(
        implicit_spell_proc_entry_like_cpp(&source)
            .unwrap()
            .hit_mask,
        PROC_HIT_REFLECT_LIKE_CPP
    );

    source.effects = vec![test_implicit_proc_effect_with_calc_like_cpp(
        0,
        aura_types::SPELL_AURA_MOD_HIT_CHANCE,
        -100,
    )];
    assert_eq!(
        implicit_spell_proc_entry_like_cpp(&source)
            .unwrap()
            .hit_mask,
        PROC_HIT_MISS_LIKE_CPP
    );
}
#[test]
fn implicit_spell_proc_entry_applies_taken_trigger_attr_and_skips_invalid_like_cpp() {
    let mut source = test_implicit_spell_proc_source_like_cpp();
    source.proc_flags = [PROC_FLAG_TAKE_HARMFUL_SPELL_LIKE_CPP, 0];
    source.effects = vec![test_implicit_proc_effect_like_cpp(
        0,
        aura_types::SPELL_AURA_PROC_TRIGGER_DAMAGE,
        [0, 0, 0, 0],
    )];

    let entry = implicit_spell_proc_entry_like_cpp(&source).unwrap();
    assert_eq!(entry.attributes_mask, PROC_ATTR_TRIGGERED_CAN_PROC_LIKE_CPP);

    source.proc_flags = [0, 0];
    assert!(implicit_spell_proc_entry_like_cpp(&source).is_none());

    source.proc_flags = [PROC_FLAG_DEAL_HARMFUL_SPELL_LIKE_CPP, 0];
    source.effects = vec![test_implicit_proc_effect_like_cpp(
        0,
        aura_types::SPELL_AURA_MOUNTED,
        [0, 0, 0, 0],
    )];
    assert!(implicit_spell_proc_entry_like_cpp(&source).is_none());
}
#[test]
fn implicit_spell_proc_entry_rejects_can_proc_from_procs_loop_like_cpp() {
    let mut source = test_implicit_spell_proc_source_like_cpp();
    source.proc_flags = [PROC_FLAG_DEAL_HARMFUL_SPELL_LIKE_CPP, 0];
    source.proc_chance = 100.0;
    source.attributes3 = attributes::SPELL_ATTR3_CAN_PROC_FROM_PROCS;
    let mut effect = test_implicit_proc_effect_like_cpp(
        0,
        aura_types::SPELL_AURA_PROC_TRIGGER_SPELL,
        [0, 0, 0, 0],
    );
    effect.trigger_spell = 123;
    source.effects = vec![effect];

    assert!(implicit_spell_proc_entry_like_cpp(&source).is_none());
}
#[test]
fn spell_learn_spell_store_validates_sql_rows_like_cpp() {
    let outcome = SpellLearnSpellStoreLikeCpp::from_sources_like_cpp(
        [
            SpellLearnSpellSqlRowLikeCpp {
                entry: 10,
                spell_id: 20,
                active: false,
            },
            SpellLearnSpellSqlRowLikeCpp {
                entry: 11,
                spell_id: 21,
                active: true,
            },
            SpellLearnSpellSqlRowLikeCpp {
                entry: 12,
                spell_id: 22,
                active: true,
            },
            SpellLearnSpellSqlRowLikeCpp {
                entry: 13,
                spell_id: 23,
                active: true,
            },
        ],
        [],
        [],
        |spell_id| match spell_id {
            10 => Some(learn_source(10, false, false, false, Vec::new())),
            12 => Some(learn_source(12, false, false, false, Vec::new())),
            13 => Some(learn_source(13, true, false, false, Vec::new())),
            _ => None,
        },
        |spell_id| matches!(spell_id, 20 | 23),
    );

    assert!(!outcome.sql_result_empty);
    assert_eq!(outcome.sql_loaded_row_count, 1);
    assert_eq!(outcome.dbc_loaded_row_count, 0);
    assert_eq!(
        outcome
            .errors
            .iter()
            .map(|error| error.kind)
            .collect::<Vec<_>>(),
        vec![
            SpellLearnSpellLoadErrorKindLikeCpp::SqlSourceSpellMissing,
            SpellLearnSpellLoadErrorKindLikeCpp::SqlLearnedSpellMissing,
            SpellLearnSpellLoadErrorKindLikeCpp::SqlSourceIsTalent,
        ]
    );
    assert_eq!(
        outcome.store.get_spell_learn_spell_map_bounds_like_cpp(10),
        &[SpellLearnSpellNodeLikeCpp {
            spell: 20,
            overrides_spell: 0,
            active: false,
            auto_learned: false,
        }]
    );
    assert!(outcome.store.is_spell_learn_spell_like_cpp(10));
    assert!(outcome.store.is_spell_learn_to_spell_like_cpp(10, 20));
    assert!(!outcome.store.is_spell_learn_to_spell_like_cpp(10, 21));
}
#[test]
fn spell_learn_spell_store_keeps_effect_and_db2_edges_when_world_sql_is_empty() {
    let outcome = SpellLearnSpellStoreLikeCpp::from_sources_like_cpp(
        [],
        [learn_source(
            100,
            false,
            false,
            false,
            vec![SpellLearnSpellEffectLikeCpp {
                trigger_spell: 101,
                target_unit_pet: false,
            }],
        )],
        [crate::spell_db2::SpellLearnSpellEntry {
            id: 1,
            spell_id: 200,
            learn_spell_id: 201,
            overrides_spell_id: 0,
        }],
        |_| None,
        |_| true,
    );

    assert!(outcome.sql_result_empty);
    assert_eq!(outcome.sql_loaded_row_count, 0);
    assert_eq!(outcome.dbc_loaded_row_count, 2);
    assert_eq!(
        outcome.store.get_spell_learn_spell_map_bounds_like_cpp(100),
        &[SpellLearnSpellNodeLikeCpp {
            spell: 101,
            overrides_spell: 0,
            active: true,
            auto_learned: false,
        }]
    );
    assert_eq!(
        outcome.store.get_spell_learn_spell_map_bounds_like_cpp(200),
        &[SpellLearnSpellNodeLikeCpp {
            spell: 201,
            overrides_spell: 0,
            active: true,
            auto_learned: false,
        }]
    );
    assert!(outcome.errors.is_empty());
    assert!(outcome.warnings.is_empty());
}
#[test]
fn spell_learn_spell_store_adds_spellinfo_effects_like_cpp() {
    let outcome = SpellLearnSpellStoreLikeCpp::from_sources_like_cpp(
        [SpellLearnSpellSqlRowLikeCpp {
            entry: 10,
            spell_id: 20,
            active: true,
        }],
        [
            learn_source(
                10,
                false,
                false,
                false,
                vec![SpellLearnSpellEffectLikeCpp {
                    trigger_spell: 20,
                    target_unit_pet: false,
                }],
            ),
            learn_source(
                30,
                false,
                true,
                false,
                vec![SpellLearnSpellEffectLikeCpp {
                    trigger_spell: 31,
                    target_unit_pet: false,
                }],
            ),
            SpellLearnSourceSpellInfoLikeCpp {
                spell_id: 40,
                difficulty_none: false,
                is_talent: false,
                is_passive: false,
                has_skill_step_effect: false,
                learn_spell_effects: vec![SpellLearnSpellEffectLikeCpp {
                    trigger_spell: 41,
                    target_unit_pet: true,
                }],
            },
        ],
        [],
        |spell_id| match spell_id {
            10 => Some(learn_source(10, false, false, false, Vec::new())),
            _ => None,
        },
        |spell_id| matches!(spell_id, 20 | 31 | 41),
    );

    assert_eq!(outcome.sql_loaded_row_count, 1);
    assert_eq!(outcome.dbc_loaded_row_count, 1);
    assert_eq!(outcome.warnings.len(), 1);
    assert_eq!(
        outcome.warnings[0].kind,
        SpellLearnSpellLoadWarningKindLikeCpp::RedundantSqlRowForSpellEffect {
            source_spell: 10,
            learned_spell: 20,
        }
    );
    assert_eq!(
        outcome.store.get_spell_learn_spell_map_bounds_like_cpp(30),
        &[SpellLearnSpellNodeLikeCpp {
            spell: 31,
            overrides_spell: 0,
            active: true,
            auto_learned: true,
        }]
    );
    assert!(
        outcome
            .store
            .get_spell_learn_spell_map_bounds_like_cpp(40)
            .is_empty()
    );
}
#[test]
fn spell_learn_spell_store_adds_db2_rows_after_sql_and_spell_effects_like_cpp() {
    let outcome = SpellLearnSpellStoreLikeCpp::from_sources_like_cpp(
        [SpellLearnSpellSqlRowLikeCpp {
            entry: 10,
            spell_id: 20,
            active: true,
        }],
        [learn_source(
            30,
            false,
            false,
            false,
            vec![SpellLearnSpellEffectLikeCpp {
                trigger_spell: 31,
                target_unit_pet: true,
            }],
        )],
        [
            crate::spell_db2::SpellLearnSpellEntry {
                id: 1,
                spell_id: 10,
                learn_spell_id: 20,
                overrides_spell_id: 0,
            },
            crate::spell_db2::SpellLearnSpellEntry {
                id: 2,
                spell_id: 30,
                learn_spell_id: 31,
                overrides_spell_id: 0,
            },
            crate::spell_db2::SpellLearnSpellEntry {
                id: 3,
                spell_id: 40,
                learn_spell_id: 41,
                overrides_spell_id: 42,
            },
            crate::spell_db2::SpellLearnSpellEntry {
                id: 4,
                spell_id: 50,
                learn_spell_id: 51,
                overrides_spell_id: 0,
            },
        ],
        |spell_id| match spell_id {
            10 => Some(learn_source(10, false, false, false, Vec::new())),
            _ => None,
        },
        |spell_id| matches!(spell_id, 10 | 20 | 30 | 31 | 40 | 41 | 51),
    );

    assert_eq!(outcome.sql_loaded_row_count, 1);
    assert_eq!(
        outcome.dbc_loaded_row_count, 2,
        "one SpellInfo effect plus one non-redundant SpellLearnSpell.db2 row"
    );
    assert_eq!(outcome.warnings.len(), 1);
    assert_eq!(
        outcome.warnings[0].kind,
        SpellLearnSpellLoadWarningKindLikeCpp::RedundantSqlRowForDb2 {
            source_spell: 10,
            learned_spell: 20,
        }
    );
    assert_eq!(
        outcome.store.get_spell_learn_spell_map_bounds_like_cpp(40),
        &[SpellLearnSpellNodeLikeCpp {
            spell: 41,
            overrides_spell: 42,
            active: true,
            auto_learned: false,
        }]
    );
    assert!(
        outcome
            .store
            .get_spell_learn_spell_map_bounds_like_cpp(50)
            .is_empty(),
        "C++ silently skips SpellLearnSpell.db2 rows whose source spell is missing"
    );
}
#[test]
fn serverside_spell_effect_store_groups_valid_effects_like_cpp() {
    let mut heroic = serverside_effect_row(100, 1);
    heroic.difficulty_id = 2;
    heroic.effect_radius_index_1 = 7;
    heroic.effect_radius_index_2 = 8;
    heroic.effect_spell_class_mask = [1, 2, 3, 4];
    heroic.implicit_target_1 = implicit_targets::TARGET_DEST_DB as i32;

    let outcome = ServersideSpellEffectStoreLikeCpp::from_rows_like_cpp(
        [heroic],
        |_| false,
        |difficulty| difficulty == 2,
        |radius| matches!(radius, 7 | 8),
    );

    assert_eq!(outcome.loaded_effect_count, 1);
    assert!(outcome.errors.is_empty());
    assert!(outcome.warnings.is_empty());
    let effects = outcome
        .store
        .effects_for_spell_difficulty_like_cpp(100, 2)
        .expect("valid serverside effect should be staged");
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].effect_index, 1);
    assert_eq!(effects[0].effect_spell_class_mask, [1, 2, 3, 4]);
    assert_eq!(
        effects[0].implicit_target,
        [implicit_targets::TARGET_DEST_DB as i32, 0]
    );
}
#[test]
fn serverside_spell_effect_store_skips_invalid_rows_like_cpp() {
    let mut regular_spell = serverside_effect_row(10, 0);
    let mut missing_difficulty = serverside_effect_row(20, 0);
    missing_difficulty.difficulty_id = 3;
    let effect_index = serverside_effect_row(30, MAX_SPELL_EFFECTS_LIKE_CPP);
    let mut effect_type = serverside_effect_row(40, 0);
    effect_type.effect = TOTAL_SPELL_EFFECTS_LIKE_CPP;
    let mut aura_type = serverside_effect_row(50, 0);
    aura_type.effect_aura = TOTAL_AURAS_LIKE_CPP;
    let mut target_a = serverside_effect_row(60, 0);
    target_a.implicit_target_1 = TOTAL_SPELL_TARGETS_LIKE_CPP;
    let mut target_b = serverside_effect_row(70, 0);
    target_b.implicit_target_2 = TOTAL_SPELL_TARGETS_LIKE_CPP;
    regular_spell.effect_base_points = 10.0;

    let outcome = ServersideSpellEffectStoreLikeCpp::from_rows_like_cpp(
        [
            regular_spell,
            missing_difficulty,
            effect_index,
            effect_type,
            aura_type,
            target_a,
            target_b,
        ],
        |spell_id| spell_id == 10,
        |_| false,
        |_| true,
    );

    assert_eq!(outcome.loaded_effect_count, 0);
    assert_eq!(
        outcome
            .errors
            .iter()
            .map(|error| error.kind)
            .collect::<Vec<_>>(),
        vec![
            ServersideSpellEffectLoadErrorKindLikeCpp::RegularSpellAlreadyLoaded,
            ServersideSpellEffectLoadErrorKindLikeCpp::DifficultyMissing,
            ServersideSpellEffectLoadErrorKindLikeCpp::EffectIndexOutOfRange,
            ServersideSpellEffectLoadErrorKindLikeCpp::EffectTypeOutOfRange,
            ServersideSpellEffectLoadErrorKindLikeCpp::AuraTypeOutOfRange,
            ServersideSpellEffectLoadErrorKindLikeCpp::ImplicitTarget1OutOfRange,
            ServersideSpellEffectLoadErrorKindLikeCpp::ImplicitTarget2OutOfRange,
        ]
    );
}
#[test]
fn serverside_spell_effect_store_preserves_cpp_radius_warning_without_skip() {
    let mut row = serverside_effect_row(100, -1);
    row.effect_radius_index_1 = 77;
    row.effect_radius_index_2 = 88;

    let outcome = ServersideSpellEffectStoreLikeCpp::from_rows_like_cpp(
        [row],
        |_| false,
        |_| true,
        |_| false,
    );

    assert_eq!(outcome.loaded_effect_count, 1);
    assert!(outcome.errors.is_empty());
    assert_eq!(
        outcome
            .warnings
            .iter()
            .map(|warning| warning.kind)
            .collect::<Vec<_>>(),
        vec![
            ServersideSpellEffectLoadWarningKindLikeCpp::EffectRadius1Missing,
            ServersideSpellEffectLoadWarningKindLikeCpp::EffectRadius2Missing,
        ]
    );
    let effects = outcome
        .store
        .effects_for_spell_difficulty_like_cpp(100, 0)
        .expect("C++ still pushes effects with invalid radius rows");
    assert_eq!(effects[0].effect_index, -1);
    assert_eq!(effects[0].effect_radius_index, [77, 88]);
}
#[test]
fn serverside_spell_check_shapeshift_rejects_excluded_form_like_cpp() {
    let spell = serverside_spell_info_for_shapeshift(0, 1 << 2, 0, 0);
    let form = shapeshift_form(shapeshift_form_flags::STANCE);

    assert_eq!(
        spell.check_shapeshift_like_cpp(3, |_| Some(&form)),
        SpellCastResult::NotShapeshift
    );
}
#[test]
fn serverside_spell_check_shapeshift_allows_explicit_form_like_cpp() {
    let spell = serverside_spell_info_for_shapeshift(1 << 4, 0, 0, 0);
    let form = shapeshift_form(shapeshift_form_flags::STANCE);

    assert_eq!(
        spell.check_shapeshift_like_cpp(5, |_| Some(&form)),
        SpellCastResult::Success
    );
}
#[test]
fn serverside_spell_check_shapeshift_missing_form_allows_like_cpp() {
    let spell =
        serverside_spell_info_for_shapeshift(0, 0, attributes::SPELL_ATTR0_NOT_SHAPESHIFTED, 0);

    assert_eq!(
        spell.check_shapeshift_like_cpp(7, |_| None),
        SpellCastResult::Success
    );
}
#[test]
fn serverside_spell_check_shapeshift_rejects_not_shapeshifted_attr_like_cpp() {
    let spell =
        serverside_spell_info_for_shapeshift(0, 0, attributes::SPELL_ATTR0_NOT_SHAPESHIFTED, 0);
    let form = shapeshift_form(0);

    assert_eq!(
        spell.check_shapeshift_like_cpp(1, |_| Some(&form)),
        SpellCastResult::NotShapeshift
    );
}
#[test]
fn serverside_spell_check_shapeshift_rejects_can_only_cast_shapeshift_spells_like_cpp() {
    let spell = serverside_spell_info_for_shapeshift(0, 0, 0, 0);
    let form = shapeshift_form(shapeshift_form_flags::CAN_ONLY_CAST_SHAPESHIFT_SPELLS);

    assert_eq!(
        spell.check_shapeshift_like_cpp(1, |_| Some(&form)),
        SpellCastResult::NotShapeshift
    );
}
#[test]
fn serverside_spell_check_shapeshift_requires_other_shifted_form_like_cpp() {
    let spell = serverside_spell_info_for_shapeshift(1 << 4, 0, 0, 0);
    let form = shapeshift_form(0);

    assert_eq!(
        spell.check_shapeshift_like_cpp(2, |_| Some(&form)),
        SpellCastResult::OnlyShapeshift
    );
}
#[test]
fn serverside_spell_check_shapeshift_requires_form_when_unshifted_like_cpp() {
    let spell = serverside_spell_info_for_shapeshift(1 << 4, 0, 0, 0);

    assert_eq!(
        spell.check_shapeshift_like_cpp(0, |_| None),
        SpellCastResult::OnlyShapeshift
    );
}
#[test]
fn serverside_spell_check_shapeshift_allows_unshifted_with_attr2_like_cpp() {
    let spell = serverside_spell_info_for_shapeshift(
        1 << 4,
        0,
        0,
        attributes::SPELL_ATTR2_ALLOW_WHILE_NOT_SHAPESHIFTED_CASTER_FORM,
    );

    assert_eq!(
        spell.check_shapeshift_like_cpp(0, |_| None),
        SpellCastResult::Success
    );
}
#[test]
fn serverside_spell_store_composes_rows_with_staged_effects_like_cpp() {
    let effect_outcome = ServersideSpellEffectStoreLikeCpp::from_rows_like_cpp(
        [serverside_effect_row(100, 0)],
        |_| false,
        |_| true,
        |_| true,
    );
    let outcome = ServersideSpellStoreLikeCpp::from_rows_like_cpp(
        [serverside_spell_row(100, 0)],
        &effect_outcome.store,
        |_| false,
    );

    assert_eq!(outcome.loaded_spell_count, 1);
    assert!(outcome.errors.is_empty());
    assert_eq!(
        outcome.store.serverside_spell_names,
        vec![(100, "Serverside 100".to_string())]
    );
    let info = outcome
        .store
        .get_serverside_spell_like_cpp(100, 0)
        .expect("serverside spell should be represented");
    assert_eq!(info.row.attributes_ex[13], 18);
    assert_eq!(info.row.spell_family_flags, [70, 71, 72, 73]);
    assert_eq!(info.effects.len(), 1);
    assert_eq!(info.effects[0].effect_index, 0);
}
#[test]
fn serverside_spell_store_rejects_regular_db2_spell_like_cpp() {
    let outcome = ServersideSpellStoreLikeCpp::from_rows_like_cpp(
        [serverside_spell_row(100, 0)],
        &ServersideSpellEffectStoreLikeCpp::default(),
        |spell_id| spell_id == 100,
    );

    assert_eq!(outcome.loaded_spell_count, 0);
    assert_eq!(outcome.errors.len(), 1);
    assert_eq!(
        outcome.errors[0].kind,
        ServersideSpellLoadErrorKindLikeCpp::RegularSpellAlreadyLoaded
    );
    assert!(outcome.store.serverside_spell_names.is_empty());
    assert!(outcome.store.spell_infos_by_spell_and_difficulty.is_empty());
}
#[test]
fn serverside_spell_store_does_not_validate_main_row_difficulty_like_cpp() {
    let outcome = ServersideSpellStoreLikeCpp::from_rows_like_cpp(
        [serverside_spell_row(100, 999)],
        &ServersideSpellEffectStoreLikeCpp::default(),
        |_| false,
    );

    assert_eq!(outcome.loaded_spell_count, 1);
    assert!(outcome.errors.is_empty());
    assert!(
        outcome
            .store
            .get_serverside_spell_like_cpp(100, 999)
            .is_some(),
        "C++ LoadSpellInfoServerside validates DifficultyID for effect rows, not for the main serverside_spell row"
    );
}
#[test]
fn hydrating_serverside_spells_publishes_the_payload_get_like_cpp() {
    // C++ `SpellMgr::LoadSpellInfoServerside` emplaces every serverside row into
    // the same `mSpellInfoMap` the DB2 spells live in (SpellMgr.cpp:3180), so
    // `GetSpellInfo` returns a full body — not merely "this id exists".
    let effect_outcome = ServersideSpellEffectStoreLikeCpp::from_rows_like_cpp(
        [serverside_effect_row(200, 0)],
        |_| false,
        |_| true,
        |_| true,
    );
    let serverside = ServersideSpellStoreLikeCpp::from_rows_like_cpp(
        [
            serverside_spell_row(200, 0),
            serverside_spell_row(201, 2),
            serverside_spell_row(300, 0),
        ],
        &effect_outcome.store,
        |_| false,
    );
    assert!(serverside.errors.is_empty());

    let mut store = SpellStore::new();
    // 300 stands for a spell the DB2 payload already owns. `emplace` keeps the
    // incumbent, so the serverside row must not replace it.
    store.spells.insert(300, test_spell_info_without_aura(300));

    assert!(
        store.get(200).is_none(),
        "the payload map holds no serverside body before hydration"
    );

    let inserted = store.hydrate_serverside_spell_infos_like_cpp(&serverside.store);

    assert_eq!(
        inserted, 1,
        "only the DIFFICULTY_NONE row enters a map keyed by spell id alone"
    );
    let hydrated = store
        .get(200)
        .expect("a serverside spell answers GetSpellInfo with a body");
    assert_eq!(hydrated.spell_id, 200);
    assert_eq!(hydrated.recovery_time_ms, 38);
    assert_eq!(hydrated.cooldown_ms, 39);
    assert_eq!(hydrated.requires_spell_focus, 23);
    assert_eq!(hydrated.effects.len(), 1);
    assert_eq!(
        hydrated.effects[0].effect,
        spell_effect_types::SPELL_EFFECT_APPLY_AURA
    );
    assert_eq!(hydrated.effects[0].effect_aura, SPELL_AURA_DUMMY_LIKE_CPP);
    assert_eq!(
        hydrated.effects[0].effect_base_points, 1,
        "the float serverside column truncates into int32 BasePoints"
    );
    assert_eq!(
        hydrated.effect_type,
        spell_effect_types::SPELL_EFFECT_APPLY_AURA,
        "the primary-effect summary is derived, not left empty"
    );
    assert_eq!(hydrated.aura_type, Some(SPELL_AURA_DUMMY_LIKE_CPP));

    assert!(
        store.get(201).is_none(),
        "a serverside row that only exists at another difficulty stays out of the payload map"
    );
    let incumbent = store.get(300).expect("the DB2 body survives hydration");
    assert_eq!(
        incumbent.effect_type,
        spell_effect_types::SPELL_EFFECT_NONE,
        "hydration must not overwrite an id the DB2 payload already owns"
    );
    assert_eq!(incumbent.recovery_time_ms, 0);
}

/// C++ `SpellEffectInfo::CalcValue` (`Spells/SpellInfo.cpp:496-597`) with a unit
/// caster.
///
/// The arms, in C++'s order: the `RealPointsPerLevel` term with its
/// `MaxLevel`/`BaseLevel` clamp and the `max(BaseLevel, SpellLevel)` subtraction
/// (`:506-517`), the `DieSides` roll (`:519-526`), then `PointsPerResource`
/// times the caster's combo points (`:531-536`), then `round`.
#[test]
fn calc_value_with_caster_matches_cpp_level_and_combo_arms() {
    use crate::spell::{CalcValueCasterLikeCpp, SpellEffectInfo, SpellLevelsLikeCpp};

    let caster = |level: u32, combo_points: u8| CalcValueCasterLikeCpp {
        level,
        combo_points,
        is_controlled_by_player: true,
        scales_with_creature_level: false,
    };

    // No roll is wanted in these cases, so a die roll would be a bug.
    let no_die = |_min: i32, _max: i32| -> i32 { panic!("DieSides is zero, C++ never rolls") };
    let effect = |base_points: i32, real_points_per_level: f32| SpellEffectInfo {
        effect_base_points: base_points,
        effect_real_points_per_level: real_points_per_level,
        ..Default::default()
    };
    let levels = |base_level: u32, max_level: u32, spell_level: u32| SpellLevelsLikeCpp {
        base_level,
        max_level,
        spell_level,
    };

    // No caster: C++ skips both unit arms, so the per-level term does not apply
    // however large it is.
    assert_eq!(
        effect(100, 5.0).calc_value_with_caster_and_die_roll_like_cpp(
            levels(1, 0, 1),
            None,
            None,
            no_die
        ),
        100
    );

    // A zero `RealPointsPerLevel` skips the arm even with a caster.
    assert_eq!(
        effect(100, 0.0).calc_value_with_caster_and_die_roll_like_cpp(
            levels(1, 0, 1),
            Some(caster(40, 0)),
            None,
            no_die
        ),
        100
    );

    // Level 20, BaseLevel and SpellLevel 1, no MaxLevel: `level -= max(1, 1)`
    // leaves 19 steps of 5.
    assert_eq!(
        effect(100, 5.0).calc_value_with_caster_and_die_roll_like_cpp(
            levels(1, 0, 1),
            Some(caster(20, 0)),
            None,
            no_die
        ),
        195
    );

    // `MaxLevel` caps the level before the subtraction: 10 instead of 20, so 9
    // steps.
    assert_eq!(
        effect(100, 5.0).calc_value_with_caster_and_die_roll_like_cpp(
            levels(1, 10, 1),
            Some(caster(20, 0)),
            None,
            no_die
        ),
        145
    );

    // A caster below `BaseLevel` is raised to it, which is why a low-level
    // caster of a high-level spell gets the spell's own floor.
    assert_eq!(
        effect(100, 5.0).calc_value_with_caster_and_die_roll_like_cpp(
            levels(30, 0, 30),
            Some(caster(5, 0)),
            None,
            no_die
        ),
        100,
        "level is clamped up to BaseLevel 30, then 30 - max(30, 30) is zero"
    );

    // `max(BaseLevel, SpellLevel)` uses the larger of the two, so a SpellLevel
    // above BaseLevel can make the term negative and reduce the base points.
    assert_eq!(
        effect(100, 5.0).calc_value_with_caster_and_die_roll_like_cpp(
            levels(1, 0, 30),
            Some(caster(20, 0)),
            None,
            no_die
        ),
        50,
        "20 - max(1, 30) is -10 steps of 5"
    );

    // `int32(level * basePointsPerLevel)` truncates toward zero rather than
    // rounding: 19 * 0.5 is 9.5, which C++ takes as 9.
    assert_eq!(
        effect(100, 0.5).calc_value_with_caster_and_die_roll_like_cpp(
            levels(1, 0, 1),
            Some(caster(20, 0)),
            None,
            no_die
        ),
        109
    );

    // The die roll lands on top of the level term, in C++'s order.
    assert_eq!(
        effect(100, 5.0).calc_value_with_caster_and_die_roll_like_cpp(
            levels(1, 0, 1),
            Some(caster(20, 0)),
            None,
            |min, max| {
                assert_eq!((min, max), (1, 4));
                3
            }
        ),
        195,
        "DieSides is zero here, so no roll is added"
    );
    let mut rolled = SpellEffectInfo {
        effect_base_points: 100,
        effect_real_points_per_level: 5.0,
        effect_die_sides: 4,
        ..Default::default()
    };
    assert_eq!(
        rolled.calc_value_with_caster_and_die_roll_like_cpp(
            levels(1, 0, 1),
            Some(caster(20, 0)),
            None,
            |min, max| {
                assert_eq!((min, max), (1, 4), "C++ irand(1, DieSides)");
                3
            }
        ),
        198
    );
    // `DieSides == 1` adds the sides rather than rolling (`:522-523`).
    rolled.effect_die_sides = 1;
    assert_eq!(
        rolled.calc_value_with_caster_and_die_roll_like_cpp(
            levels(1, 0, 1),
            Some(caster(20, 0)),
            None,
            no_die
        ),
        196
    );
    // A negative `DieSides` rolls the reversed range (`:525`).
    rolled.effect_die_sides = -4;
    assert_eq!(
        rolled.calc_value_with_caster_and_die_roll_like_cpp(
            levels(1, 0, 1),
            Some(caster(20, 0)),
            None,
            |min, max| {
                assert_eq!((min, max), (-4, 1), "C++ irand(DieSides, 1)");
                -2
            }
        ),
        193
    );

    // Combo points multiply `PointsPerResource`, and only with a unit caster.
    let combo = SpellEffectInfo {
        effect_base_points: 100,
        effect_points_per_resource: 7.5,
        ..Default::default()
    };
    assert_eq!(
        combo.calc_value_with_caster_and_die_roll_like_cpp(
            SpellLevelsLikeCpp::default(),
            Some(caster(20, 4)),
            None,
            no_die
        ),
        130,
        "100 + 7.5 * 4 rounds to 130"
    );
    assert_eq!(
        combo.calc_value_with_caster_and_die_roll_like_cpp(
            SpellLevelsLikeCpp::default(),
            None,
            None,
            no_die
        ),
        100,
        "no unit caster, no combo term"
    );
    assert_eq!(
        combo.calc_value_with_caster_and_die_roll_like_cpp(
            SpellLevelsLikeCpp::default(),
            Some(caster(20, 0)),
            None,
            no_die
        ),
        100,
        "C++ reads the combo term only when GetComboPoints() is non-zero"
    );

    // The no-caster entry point is the same function with both unit arms off,
    // so the two agree by construction rather than by a copied body.
    assert_eq!(
        effect(100, 5.0).calc_value_no_caster_with_die_roll_like_cpp(no_die),
        100
    );
}

/// C++ `CalcValue`'s creature-level multiplication gate (`SpellInfo.cpp:544-594`).
///
/// The predicate is public because a caller with no `NpcManaCostScaler` table in
/// hand still needs to know when C++ would have scaled, so the gap stays
/// countable rather than silent.
#[test]
fn calc_value_creature_level_scaling_gate_matches_cpp() {
    use crate::spell::{SpellEffectInfo, SpellLevelsLikeCpp, spell_effect_types};

    let damage = SpellEffectInfo {
        effect: spell_effect_types::SPELL_EFFECT_SCHOOL_DAMAGE,
        ..Default::default()
    };
    let levels = SpellLevelsLikeCpp {
        base_level: 1,
        max_level: 0,
        spell_level: 10,
    };

    assert!(damage.calc_value_reaches_creature_level_scaling_like_cpp(levels, 20, false, true));
    assert!(
        !damage.calc_value_reaches_creature_level_scaling_like_cpp(levels, 20, true, true),
        "C++ requires !IsControlledByPlayer()"
    );
    assert!(
        !damage.calc_value_reaches_creature_level_scaling_like_cpp(levels, 10, false, true),
        "SpellLevel equal to the caster's level skips it"
    );
    assert!(
        !damage.calc_value_reaches_creature_level_scaling_like_cpp(
            SpellLevelsLikeCpp {
                spell_level: 0,
                ..levels
            },
            20,
            false,
            true
        ),
        "a zero SpellLevel skips it"
    );
    assert!(
        !damage.calc_value_reaches_creature_level_scaling_like_cpp(levels, 20, false, false),
        "SPELL_ATTR0_SCALES_WITH_CREATURE_LEVEL is required"
    );
    assert!(
        !SpellEffectInfo {
            effect: spell_effect_types::SPELL_EFFECT_SCHOOL_DAMAGE,
            effect_real_points_per_level: 5.0,
            ..Default::default()
        }
        .calc_value_reaches_creature_level_scaling_like_cpp(levels, 20, false, true),
        "C++ requires !basePointsPerLevel, the two scalings are exclusive"
    );

    // Neither switch matches, so C++ leaves canEffectScale false.
    assert!(
        !SpellEffectInfo {
            effect: spell_effect_types::SPELL_EFFECT_APPLY_AURA,
            effect_aura: aura_types::SPELL_AURA_MOD_STUN,
            ..Default::default()
        }
        .calc_value_reaches_creature_level_scaling_like_cpp(levels, 20, false, true)
    );
    // The aura switch is enough on its own (`:568-584`).
    assert!(
        SpellEffectInfo {
            effect: spell_effect_types::SPELL_EFFECT_APPLY_AURA,
            effect_aura: aura_types::SPELL_AURA_PERIODIC_DAMAGE,
            ..Default::default()
        }
        .calc_value_reaches_creature_level_scaling_like_cpp(levels, 20, false, true)
    );
}

/// C++ `value *= casterScaler->Scaler / spellScaler->Scaler`
/// (`SpellInfo.cpp:586-592`), with the `NPCManaCostScaler.txt` table C++ reads it
/// from.
#[test]
fn calc_value_creature_level_scaling_applies_the_npc_mana_cost_scaler_like_cpp() {
    use crate::game_tables::NpcManaCostScalerGameTableLikeCpp;
    use crate::spell::{CalcValueCasterLikeCpp, SpellEffectInfo, SpellLevelsLikeCpp};

    let no_die = |_min: i32, _max: i32| -> i32 { panic!("DieSides is zero, C++ never rolls") };
    let effect = SpellEffectInfo {
        effect: spell_effect_types::SPELL_EFFECT_SCHOOL_DAMAGE,
        effect_base_points: 100,
        ..Default::default()
    };
    let levels = SpellLevelsLikeCpp {
        base_level: 1,
        max_level: 0,
        spell_level: 10,
    };
    // Row 0 is the unused default, so level 10 is the tenth scaler and level 20
    // the twentieth.
    let table =
        NpcManaCostScalerGameTableLikeCpp::from_scalers((1..=20).map(|level| level as f32 / 10.0));
    let creature = CalcValueCasterLikeCpp {
        level: 20,
        combo_points: 0,
        is_controlled_by_player: false,
        scales_with_creature_level: true,
    };

    // `casterScaler 2.0 / spellScaler 1.0` doubles the value.
    assert_eq!(
        effect.calc_value_with_caster_and_die_roll_like_cpp(
            levels,
            Some(creature),
            Some(&table),
            no_die
        ),
        200
    );
    // Without the table C++'s `if (spellScaler && casterScaler)` leaves the value
    // alone, which is also what a caller with no table in hand gets.
    assert_eq!(
        effect.calc_value_with_caster_and_die_roll_like_cpp(levels, Some(creature), None, no_die),
        100
    );
    // A level past the end of the table is C++'s null row.
    assert_eq!(
        effect.calc_value_with_caster_and_die_roll_like_cpp(
            levels,
            Some(CalcValueCasterLikeCpp {
                level: 21,
                ..creature
            }),
            Some(&table),
            no_die
        ),
        100
    );
    // A player-controlled caster never reaches the arm, table or not.
    assert_eq!(
        effect.calc_value_with_caster_and_die_roll_like_cpp(
            levels,
            Some(CalcValueCasterLikeCpp {
                is_controlled_by_player: true,
                ..creature
            }),
            Some(&table),
            no_die
        ),
        100
    );
    // The scaling multiplies the value the earlier arms produced, not the raw
    // base points: `(100 + 19 * 2) * 2`.
    assert_eq!(
        SpellEffectInfo {
            effect_real_points_per_level: 2.0,
            ..effect.clone()
        }
        .calc_value_with_caster_and_die_roll_like_cpp(
            SpellLevelsLikeCpp {
                base_level: 1,
                max_level: 0,
                spell_level: 1,
            },
            Some(creature),
            Some(&table),
            no_die
        ),
        138,
        "RealPointsPerLevel and the creature scaling are mutually exclusive in C++"
    );
}

/// The real `NPCManaCostScaler.txt` parses into C++'s row shape.
#[test]
fn npc_mana_cost_scaler_parses_the_installed_game_table_like_cpp() {
    use crate::game_tables::NpcManaCostScalerGameTableLikeCpp;

    let path = std::path::Path::new("/opt/wow-3.4.3/gt/NPCManaCostScaler.txt");
    if !path.exists() {
        // The installed client data is not part of the repository.
        return;
    }
    let table = NpcManaCostScalerGameTableLikeCpp::load_from_path(path)
        .expect("the installed table must parse");
    assert_eq!(
        table.len(),
        101,
        "100 level rows plus LoadGameTable's unused row 0"
    );
    assert_eq!(
        table.row(1).map(|row| row.scaler),
        Some(0.193),
        "the first data row is level 1"
    );
    assert_eq!(
        table.row(0).map(|row| row.scaler),
        Some(0.0),
        "row 0 is the default unused entry"
    );
    assert!(table.row(101).is_none(), "past the end is C++'s null row");
}
