use super::super::save_plan::player_character_save_statements_like_cpp;
use super::{equipment_row, minimal_character_request};
use crate::params::PreparedStatement;
use crate::statements::CharStatements;
use crate::statements::StatementDef;
use wow_persistence::*;
use wow_persistence::{
    PlayerCharacterSaveRequestLikeCpp, PlayerCufProfileSaveLikeCpp, PlayerEquipmentSetStateLikeCpp,
    PlayerEquipmentSetTypeLikeCpp, PlayerSpellSaveGroupLikeCpp, PlayerSpellStateLikeCpp,
    PlayerVoidStorageSaveLikeCpp,
};

#[test]
fn character_save_adapter_preserves_the_frozen_statement_order_like_cpp() {
    let mut equipment_insert = equipment_row();
    equipment_insert.set_guid = 10;
    let mut equipment_update = equipment_row();
    equipment_update.set_guid = 11;
    equipment_update.state = PlayerEquipmentSetStateLikeCpp::Changed;
    let mut equipment_delete = equipment_row();
    equipment_delete.set_guid = 12;
    equipment_delete.state = PlayerEquipmentSetStateLikeCpp::Deleted;
    let mut transmog_insert = equipment_row();
    transmog_insert.set_guid = 13;
    transmog_insert.set_type = PlayerEquipmentSetTypeLikeCpp::Transmog;
    let mut transmog_update = equipment_row();
    transmog_update.set_guid = 14;
    transmog_update.set_type = PlayerEquipmentSetTypeLikeCpp::Transmog;
    transmog_update.state = PlayerEquipmentSetStateLikeCpp::Changed;
    let mut transmog_delete = equipment_row();
    transmog_delete.set_guid = 15;
    transmog_delete.set_type = PlayerEquipmentSetTypeLikeCpp::Transmog;
    transmog_delete.state = PlayerEquipmentSetStateLikeCpp::Deleted;

    let request = PlayerCharacterSaveRequestLikeCpp {
        player_guid: 1,
        account_id: 2,
        wall_clock_unix_secs: 1_700_000_000,
        character: PlayerCharacterSnapshotSaveLikeCpp {
            position: PlayerPositionSaveLikeCpp {
                x: 1.0,
                y: 2.0,
                z: 3.0,
                orientation: 0.5,
                map_id: 0,
                instance_id: 0,
                zone_id: 0,
            },
            level: 1,
            xp: 0,
            money: 7,
            rest_state: 0,
            player_flags: 0,
            rest_bonus: 0.0,
            logout_time: 1_700_000_000,
            is_logout_resting: false,
            health: 9,
            powers: Some([0; 10]),
            talent_reset_cost: 0,
            talent_reset_time: 0,
            explored_zones: String::new(),
            dungeon_difficulty: 0,
            raid_difficulty: 0,
            legacy_raid_difficulty: 0,
            at_login_flags: 0,
        },
        spells: Some(PlayerSpellSaveGroupLikeCpp::Complete {
            rows: vec![PlayerSpellSaveLikeCpp {
                spell_id: 100,
                active: true,
                disabled: false,
                dependent: false,
                favorite: true,
                state: PlayerSpellStateLikeCpp::Changed,
            }],
            fallback_rows_were_present: false,
        }),
        skills: Some(vec![PlayerSkillSaveLikeCpp {
            skill_id: 6,
            value: 300,
            max: 300,
            profession_slot: -1,
        }]),
        glyphs: Some(
            (0..24)
                .map(|glyph_slot| PlayerGlyphSaveLikeCpp {
                    talent_group: 0,
                    glyph_slot,
                    glyph_id: 0,
                })
                .collect(),
        ),
        talents: Some(vec![PlayerTalentSaveLikeCpp {
            talent_id: 200,
            rank: 1,
            talent_group: 0,
        }]),
        spell_cooldowns: Some(vec![PlayerSpellCooldownSaveLikeCpp {
            spell_id: 300,
            item_id: 0,
            cooldown_end_unix_secs: 1_700_000_010,
            category_id: 30,
            category_end_unix_secs: 1_700_000_020,
        }]),
        spell_charges: Some(vec![PlayerSpellChargeSaveLikeCpp {
            category_id: 31,
            recharge_start_unix_secs: 1_700_000_001,
            recharge_end_unix_secs: 1_700_000_030,
        }]),
        action_buttons: Some(PlayerActionButtonsSaveLikeCpp {
            spec: 0,
            trait_config_id: 0,
            rows: vec![PlayerActionButtonSaveLikeCpp {
                button: 0,
                packed_action: 0x0100_0064,
            }],
        }),
        auras: Some(vec![PlayerAuraSaveLikeCpp {
            caster_guid_binary: vec![1; 16],
            item_guid_binary: vec![0; 16],
            spell_id: 700,
            effect_mask: 0b11,
            recalculate_mask: 0b11,
            difficulty: 0,
            stack_count: 1,
            max_duration_ms: 60_000,
            remain_time_ms: 42_000,
            remain_charges: 0,
            cast_item_id: 0,
            cast_item_level: 0,
            effects: vec![
                PlayerAuraEffectSaveRowLikeCpp {
                    effect_index: 0,
                    amount: 7,
                    base_amount: 5,
                },
                PlayerAuraEffectSaveRowLikeCpp {
                    effect_index: 1,
                    amount: 9,
                    base_amount: 9,
                },
            ],
        }]),
        equipment_sets: Some(vec![
            equipment_insert,
            equipment_update,
            equipment_delete,
            transmog_insert,
            transmog_update,
            transmog_delete,
        ]),
        void_storage: Some(vec![
            PlayerVoidStorageSlotSaveLikeCpp {
                slot: 0,
                item: Some(PlayerVoidStorageSaveLikeCpp {
                    item_id: 400,
                    item_entry: 401,
                    creator_guid: 402,
                    fixed_scaling_level: 80,
                    random_properties_id: 0,
                    random_properties_seed: 0,
                    context: 0,
                }),
            },
            PlayerVoidStorageSlotSaveLikeCpp {
                slot: 1,
                item: None,
            },
        ]),
        tutorials: Some(PlayerTutorialsSaveLikeCpp {
            tutorials: [0; 8],
            already_persisted: false,
        }),
        instance_lock_times: vec![PlayerInstanceLockTimeSaveLikeCpp {
            instance_id: 500,
            release_time: 1_700_000_100,
        }],
        played_time: PlayerPlayedTimeSaveLikeCpp {
            total_time: 11,
            level_time: 5,
        },
        reputations: vec![PlayerReputationSaveLikeCpp {
            faction_id: 600,
            standing: 1_000,
            flags: 1,
        }],
        cuf_profiles: Some(vec![
            PlayerCufProfileSlotSaveLikeCpp {
                profile_id: 0,
                profile: Some(PlayerCufProfileSaveLikeCpp {
                    profile_name: "raid".to_owned(),
                    frame_height: 40,
                    frame_width: 80,
                    sort_by: 0,
                    health_text: 0,
                    bool_options: 0,
                    top_point: 0,
                    bottom_point: 0,
                    left_point: 0,
                    top_offset: 0,
                    bottom_offset: 0,
                    left_offset: 0,
                }),
            },
            PlayerCufProfileSlotSaveLikeCpp {
                profile_id: 1,
                profile: None,
            },
        ]),
    };
    let mut runs: Vec<(String, usize)> = Vec::new();
    for statement in player_character_save_statements_like_cpp(&request) {
        let sql = statement.sql().to_owned();
        match runs.last_mut() {
            Some((previous, count)) if *previous == sql => *count += 1,
            _ => runs.push((sql, 1)),
        }
    }
    let golden: Vec<(String, usize)> = serde_json::from_str(include_str!(
        "../../../../../wow-world/tests/fixtures/player-save-plan-order.json"
    ))
    .expect("frozen order fixture parses");
    assert_eq!(
        runs,
        golden,
        "replace player-save-plan-order.json with:\n{}",
        serde_json::to_string_pretty(&runs).unwrap_or_default()
    );
}

#[test]
fn fallback_spells_expand_inside_the_spell_group_without_a_statement_port_like_cpp() {
    let mut request = minimal_character_request();
    request.spells = Some(PlayerSpellSaveGroupLikeCpp::Fallback {
        rows: vec![
            PlayerFallbackSpellSaveLikeCpp {
                spell_id: 20,
                active: true,
                dependent: false,
            },
            PlayerFallbackSpellSaveLikeCpp {
                spell_id: 30,
                active: false,
                dependent: true,
            },
        ],
    });
    let statements = player_character_save_statements_like_cpp(&request);
    let sql = statements
        .iter()
        .map(PreparedStatement::sql)
        .collect::<Vec<_>>();
    let fallback = sql
        .iter()
        .position(|sql| *sql == CharStatements::UPSERT_CHAR_SPELL_LEARN_FALLBACK.sql())
        .expect("non-dependent fallback spell is upserted");
    assert_eq!(
        sql.get(fallback + 1),
        Some(&CharStatements::DEL_CHAR_SPELL_BY_SPELL.sql()),
        "dependent fallback removal retains its position in the spell group"
    );
}

#[test]
fn cooldown_and_charge_groups_drop_expired_rows_but_keep_the_group_replace_like_cpp() {
    let mut request = minimal_character_request();
    request.spell_cooldowns = Some(vec![
        PlayerSpellCooldownSaveLikeCpp {
            spell_id: 1,
            item_id: 0,
            cooldown_end_unix_secs: 1_699_999_999,
            category_id: 0,
            category_end_unix_secs: 1_699_999_999,
        },
        PlayerSpellCooldownSaveLikeCpp {
            spell_id: 2,
            item_id: 0,
            cooldown_end_unix_secs: 1_700_000_001,
            category_id: 0,
            category_end_unix_secs: 1_699_999_999,
        },
    ]);
    request.spell_charges = Some(vec![
        PlayerSpellChargeSaveLikeCpp {
            category_id: 3,
            recharge_start_unix_secs: 1_699_999_990,
            recharge_end_unix_secs: 1_699_999_999,
        },
        PlayerSpellChargeSaveLikeCpp {
            category_id: 4,
            recharge_start_unix_secs: 1_700_000_000,
            recharge_end_unix_secs: 1_700_000_001,
        },
    ]);
    let statements = player_character_save_statements_like_cpp(&request);
    let sql = statements
        .iter()
        .map(PreparedStatement::sql)
        .collect::<Vec<_>>();
    assert_eq!(
        sql.iter()
            .filter(|sql| **sql == CharStatements::INS_CHAR_SPELL_COOLDOWN.sql())
            .count(),
        1
    );
    assert_eq!(
        sql.iter()
            .filter(|sql| **sql == CharStatements::INS_CHAR_SPELL_CHARGES.sql())
            .count(),
        1
    );
    assert!(sql.contains(&CharStatements::DEL_CHAR_SPELL_COOLDOWNS.sql()));
    assert!(sql.contains(&CharStatements::DEL_CHAR_SPELL_CHARGES.sql()));
}

#[test]
fn the_full_save_persists_the_absolute_at_login_flags_like_cpp() {
    // C++ `Player::RemoveAtLoginFlag(AT_LOGIN_FIRST)` on the first login does
    // not write: it clears `m_atLoginFlags` and lets the next
    // `Player::SaveToDB` bind it into `CHAR_UPD_CHARACTER`
    // (`Player.cpp:19849`). Without this statement the flag survives in the row
    // and every later login is treated as the first one.
    let mut request = minimal_character_request();
    request.character.at_login_flags = 0;
    let statements = player_character_save_statements_like_cpp(&request);
    let cleared = statements
        .iter()
        .find(|statement| statement.sql() == CharStatements::UPD_CHAR_AT_LOGIN_FLAGS.sql())
        .expect("the full save writes at_login");
    assert_eq!(
        cleared.params(),
        &[
            crate::params::SqlParam::U16(0),
            crate::params::SqlParam::U64(request.player_guid),
        ],
        "the save binds the absolute flag value and the guid"
    );

    // The value is whatever the snapshot carries, not a mask: C++ writes
    // `at_login = ?`, while `at_login | ?` / `at_login & ~ ?` belong to the
    // immediate SetAtLoginFlag/RemoveAtLoginFlag(.., true) writes.
    request.character.at_login_flags = 0x021;
    let statements = player_character_save_statements_like_cpp(&request);
    let retained = statements
        .iter()
        .find(|statement| statement.sql() == CharStatements::UPD_CHAR_AT_LOGIN_FLAGS.sql())
        .expect("the full save writes at_login");
    assert_eq!(
        retained.params().first(),
        Some(&crate::params::SqlParam::U16(0x021))
    );
    assert!(
        !CharStatements::UPD_CHAR_AT_LOGIN_FLAGS.sql().contains('|'),
        "the full save is an absolute assignment, not an OR mask"
    );
}

#[test]
fn the_aura_group_writes_both_deletes_then_each_aura_with_its_own_effects_like_cpp() {
    // C++ `Player::_SaveAuras` (`Player.cpp:20089-20146`) appends
    // `CHAR_DEL_CHAR_AURA_EFFECT`, then `CHAR_DEL_CHAR_AURA`, then for each
    // saveable aura one `CHAR_INS_AURA` immediately followed by that aura's own
    // `CHAR_INS_AURA_EFFECT` rows. The group sits between the action buttons and
    // the skills (`Player.cpp:19947-19949`).
    let mut request = minimal_character_request();
    request.auras = Some(vec![
        PlayerAuraSaveLikeCpp {
            caster_guid_binary: vec![1; 16],
            item_guid_binary: vec![0; 16],
            spell_id: 101,
            effect_mask: 0b101,
            recalculate_mask: 0b100,
            difficulty: 0,
            stack_count: 1,
            max_duration_ms: 60_000,
            remain_time_ms: 42_000,
            remain_charges: 0,
            cast_item_id: 0,
            cast_item_level: 0,
            effects: vec![
                PlayerAuraEffectSaveRowLikeCpp {
                    effect_index: 0,
                    amount: 7,
                    base_amount: 5,
                },
                PlayerAuraEffectSaveRowLikeCpp {
                    effect_index: 2,
                    amount: -3,
                    base_amount: -3,
                },
            ],
        },
        PlayerAuraSaveLikeCpp {
            caster_guid_binary: vec![2; 16],
            item_guid_binary: vec![0; 16],
            spell_id: 202,
            effect_mask: 0b1,
            recalculate_mask: 0,
            difficulty: 0,
            stack_count: 3,
            max_duration_ms: -1,
            remain_time_ms: -1,
            remain_charges: 0,
            cast_item_id: 0,
            cast_item_level: 0,
            effects: Vec::new(),
        },
    ]);
    let statements = player_character_save_statements_like_cpp(&request);
    let sql = statements
        .iter()
        .map(PreparedStatement::sql)
        .collect::<Vec<_>>();
    let position = |needle: &str| {
        sql.iter()
            .position(|candidate| *candidate == needle)
            .unwrap_or_else(|| panic!("missing statement: {needle}"))
    };

    let delete_effects = position(CharStatements::DEL_CHAR_AURA_EFFECT.sql());
    let delete_auras = position(CharStatements::DEL_CHAR_AURA.sql());
    let first_aura = position(CharStatements::INS_AURA.sql());
    let first_effect = position(CharStatements::INS_AURA_EFFECT.sql());
    assert!(delete_effects < delete_auras, "effects are deleted first");
    assert!(delete_auras < first_aura, "both deletes precede any insert");
    assert!(
        first_aura < first_effect,
        "an aura precedes its own effects"
    );
    assert_eq!(
        sql.iter()
            .filter(|candidate| **candidate == CharStatements::INS_AURA.sql())
            .count(),
        2
    );
    assert_eq!(
        sql.iter()
            .filter(|candidate| **candidate == CharStatements::INS_AURA_EFFECT.sql())
            .count(),
        2,
        "only the first aura carries effect rows"
    );
    // The second aura's row follows the first aura's effects, not before them.
    assert!(
        sql.iter()
            .skip(first_effect)
            .any(|candidate| *candidate == CharStatements::INS_AURA.sql())
    );

    // Both GUID columns are bound as the raw binary C++ writes with
    // `setBinary(ObjectGuid::GetRawValue())`, not as text.
    let aura_params = statements[first_aura].params();
    assert_eq!(aura_params.len(), 13);
    assert_eq!(aura_params[1], crate::params::SqlParam::Bytes(vec![1; 16]));
    assert_eq!(aura_params[2], crate::params::SqlParam::Bytes(vec![0; 16]));
    let effect_params = statements[first_effect].params();
    assert_eq!(effect_params.len(), 8);
    assert_eq!(
        effect_params[1],
        crate::params::SqlParam::Bytes(vec![1; 16])
    );
}

#[test]
fn an_empty_aura_group_still_clears_the_stored_rows_like_cpp() {
    // C++ appends both deletes before it reads `m_ownedAuras` at all, so a
    // Player who dropped every aura clears the table.
    let mut request = minimal_character_request();
    request.auras = Some(Vec::new());
    let statements = player_character_save_statements_like_cpp(&request);
    let sql = statements
        .iter()
        .map(PreparedStatement::sql)
        .collect::<Vec<_>>();
    assert!(sql.contains(&CharStatements::DEL_CHAR_AURA_EFFECT.sql()));
    assert!(sql.contains(&CharStatements::DEL_CHAR_AURA.sql()));
    assert!(!sql.contains(&CharStatements::INS_AURA.sql()));
}

#[test]
fn a_missing_aura_group_touches_neither_aura_table_like_cpp() {
    let request = minimal_character_request();
    let statements = player_character_save_statements_like_cpp(&request);
    let sql = statements
        .iter()
        .map(PreparedStatement::sql)
        .collect::<Vec<_>>();
    assert!(!sql.contains(&CharStatements::DEL_CHAR_AURA.sql()));
    assert!(!sql.contains(&CharStatements::DEL_CHAR_AURA_EFFECT.sql()));
}
