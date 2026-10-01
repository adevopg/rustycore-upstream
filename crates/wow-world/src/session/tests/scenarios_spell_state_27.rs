//! Session scenarios for the Player half of C++ `Player::_SaveAuras`.

use super::*;

/// One `SpellInfo` with the attribute words and single effect the save reads.
fn aura_save_spell_like_cpp(
    spell_id: i32,
    effect_base_points: i32,
    implicit_target_1: u32,
    effect: u32,
) -> SpellInfo {
    SpellInfo {
        spell_id,
        cast_time_ms: 0,
        cooldown_ms: 0,
        recovery_time_ms: 0,
        effect_type: effect,
        effect_base_points,
        effect_bonus_coefficient: 0.0,
        aura_type: Some(wow_data::spell::aura_types::SPELL_AURA_MOD_DAMAGE_DONE),
        display_flags: 0,
        requires_spell_focus: 0,
        power_costs: Vec::new(),
        effects: vec![wow_data::SpellEffectInfo {
            effect_index: 0,
            effect,
            effect_aura: wow_data::spell::aura_types::SPELL_AURA_MOD_DAMAGE_DONE,
            effect_base_points,
            implicit_target_1,
            ..Default::default()
        }],
    }
}

fn aura_save_session_like_cpp() -> (WorldSession, ObjectGuid) {
    let (mut session, _, _) = make_session();
    let player_guid = ObjectGuid::create_player(1, 77);
    session.set_player_guid(Some(player_guid));

    let mut spell_store = SpellStore::new();
    // 101: an ordinary single-target buff.
    spell_store.insert(
        101,
        aura_save_spell_like_cpp(
            101,
            44,
            0,
            wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
        ),
    );
    // 202: passive.
    spell_store.insert(
        202,
        aura_save_spell_like_cpp(
            202,
            0,
            0,
            wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
        ),
    );
    spell_store.insert_spell_misc_attributes_like_cpp(202, {
        let mut attributes = [0u32; 15];
        attributes[0] = wow_data::spell::attributes::SPELL_ATTR0_PASSIVE;
        attributes
    });
    // 303: channeled.
    spell_store.insert(
        303,
        aura_save_spell_like_cpp(
            303,
            0,
            0,
            wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
        ),
    );
    spell_store.insert_spell_misc_attributes_like_cpp(303, {
        let mut attributes = [0u32; 15];
        attributes[1] = wow_data::spell::attributes::SPELL_ATTR1_IS_CHANNELLED;
        attributes
    });
    // 404: an area-aura effect, saveable from its own caster only.
    spell_store.insert(
        404,
        aura_save_spell_like_cpp(
            404,
            0,
            0,
            wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AREA_AURA_PARTY,
        ),
    );
    // 505: a single-effect aura whose TargetA is an area target.
    spell_store.insert(
        505,
        aura_save_spell_like_cpp(
            505,
            0,
            33, // TARGET_UNIT_SRC_AREA_PARTY
            wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
        ),
    );
    // 606: SPELL_ATTR5_LIMIT_N, which C++ `SpellInfo::IsSingleTarget` reads.
    spell_store.insert(
        606,
        aura_save_spell_like_cpp(
            606,
            0,
            0,
            wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
        ),
    );
    spell_store.insert_spell_misc_attributes_like_cpp(606, {
        let mut attributes = [0u32; 15];
        attributes[5] = wow_data::spell::attributes::SPELL_ATTR5_LIMIT_N;
        attributes
    });
    // 707: an aura type `SpellMgr::LoadSpellInfoCustomAttributes` marks as one
    // that cannot be saved.
    let mut charm = aura_save_spell_like_cpp(
        707,
        0,
        0,
        wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
    );
    charm.effects[0].effect_aura = wow_data::spell::aura_types::SPELL_AURA_MOD_CHARM;
    spell_store.insert(707, charm);
    session.set_spell_store(Arc::new(spell_store));
    (session, player_guid)
}

/// The save row carries the live duration, stack and effect values, and the
/// effect's `BasePoints` as the base amount C++ `AuraEffect` holds for an aura
/// that was not restored with a stored one (`SpellAuraEffects.cpp:620`).
#[test]
fn player_aura_save_rows_carry_the_live_aura_values_like_cpp() {
    let (mut session, player_guid) = aura_save_session_like_cpp();
    let mut aura = test_visible_aura(0, 101);
    aura.caster_guid = player_guid;
    aura.duration_total = 60_000;
    aura.duration_remaining = 42_000;
    aura.stack_count = 3;
    aura.represented_effect_amounts = vec![RepresentedAuraEffectAmountLikeCpp {
        effect_index: 0,
        amount: 7,
    }];
    session.visible_auras.insert(0, aura);

    let rows = session
        .player_aura_save_rows_like_cpp()
        .expect("the live aura state is readable");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.spell_id, 101);
    assert_eq!(row.caster_guid_binary, player_guid.to_raw_bytes().to_vec());
    assert_eq!(
        row.item_guid_binary,
        ObjectGuid::EMPTY.to_raw_bytes().to_vec()
    );
    assert_eq!(row.stack_count, 3);
    assert_eq!(row.max_duration_ms, 60_000);
    assert_eq!(row.remain_time_ms, 42_000);
    // C++ `Aura::GenerateKey` derives both masks from the live effects, and a
    // runtime `AuraEffect` starts recalculable.
    assert_eq!(row.effect_mask, 0b1);
    assert_eq!(row.recalculate_mask, 0b1);
    assert_eq!(row.effects.len(), 1);
    assert_eq!(row.effects[0].effect_index, 0);
    assert_eq!(row.effects[0].amount, 7);
    assert_eq!(row.effects[0].base_amount, 44);
}

/// C++ writes `-1` for a permanent aura's duration, which is the marker
/// `_LoadAuras` reads back (`session/mod.rs:1586-1602`).
#[test]
fn a_permanent_player_aura_is_stored_with_the_cpp_minus_one_duration_like_cpp() {
    let (mut session, player_guid) = aura_save_session_like_cpp();
    let mut aura = test_visible_aura(0, 101);
    aura.caster_guid = player_guid;
    aura.duration_total = 0;
    aura.duration_remaining = 0;
    session.visible_auras.insert(0, aura);

    let rows = session
        .player_aura_save_rows_like_cpp()
        .expect("the live aura state is readable");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].max_duration_ms, -1);
    assert_eq!(rows[0].remain_time_ms, -1);
}

/// `Aura::CanBeSaved` (`SpellAuras.cpp:1172-1209`) refuses a passive or
/// channeled aura, and one whose spell carries the cannot-be-saved custom
/// attribute, whoever cast it.
#[test]
fn passive_channeled_and_unsaveable_player_auras_are_skipped_like_cpp() {
    let (mut session, player_guid) = aura_save_session_like_cpp();
    for (slot, spell_id) in [(0u8, 101i32), (1, 202), (2, 303), (3, 707)] {
        let mut aura = test_visible_aura(slot, spell_id);
        aura.caster_guid = player_guid;
        session.visible_auras.insert(slot, aura);
    }

    let rows = session
        .player_aura_save_rows_like_cpp()
        .expect("the live aura state is readable");
    assert_eq!(
        rows.iter().map(|row| row.spell_id).collect::<Vec<_>>(),
        vec![101]
    );
}

/// The area and single-target gates fire only for a foreign caster: C++ checks
/// `GetCasterGUID() != GetOwner()->GetGUID()` before it looks at the effects.
#[test]
fn a_foreign_casters_area_or_single_target_aura_is_skipped_like_cpp() {
    let stranger = ObjectGuid::create_player(1, 0xB2);
    for spell_id in [404i32, 505, 606] {
        let (mut session, player_guid) = aura_save_session_like_cpp();
        let mut own = test_visible_aura(0, spell_id);
        own.caster_guid = player_guid;
        session.visible_auras.insert(0, own);
        assert_eq!(
            session
                .player_aura_save_rows_like_cpp()
                .expect("the live aura state is readable")
                .len(),
            1,
            "the owner's own aura {spell_id} is saved"
        );

        let (mut session, _) = aura_save_session_like_cpp();
        let mut foreign = test_visible_aura(0, spell_id);
        foreign.caster_guid = stranger;
        session.visible_auras.insert(0, foreign);
        assert!(
            session
                .player_aura_save_rows_like_cpp()
                .expect("the live aura state is readable")
                .is_empty(),
            "another caster's aura {spell_id} is not"
        );
    }
}

/// A foreign caster's ordinary buff is saved: none of the gates apply to it.
#[test]
fn a_foreign_casters_ordinary_buff_is_saved_like_cpp() {
    let (mut session, _) = aura_save_session_like_cpp();
    let mut aura = test_visible_aura(0, 101);
    aura.caster_guid = ObjectGuid::create_player(1, 0xB2);
    session.visible_auras.insert(0, aura);

    let rows = session
        .player_aura_save_rows_like_cpp()
        .expect("the live aura state is readable");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].caster_guid_binary,
        ObjectGuid::create_player(1, 0xB2).to_raw_bytes().to_vec()
    );
}

/// With every aura gone the group is still present and empty, so the save
/// clears the stored rows instead of leaving the last login's auras behind.
#[test]
fn a_player_without_auras_still_reports_an_empty_group_like_cpp() {
    let (session, _) = aura_save_session_like_cpp();
    assert_eq!(
        session.player_aura_save_rows_like_cpp(),
        Some(Vec::new()),
        "an empty group is not a missing group"
    );
}
