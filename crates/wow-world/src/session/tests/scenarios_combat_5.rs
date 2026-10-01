//! Session scenarios exercising represented melee mitigation stages (#29).
//!
//! Split out of `scenarios_combat_4.rs` before that file reached the
//! 2,000-line test-file budget; the shared fixtures stay in the parent module.

use super::*;

#[test]
fn melee_crushing_band_preserves_cpp_expression_and_gates() {
    use crate::session_rules::{
        RepresentedMeleeAttackerFactsLikeCpp as Attacker,
        RepresentedMeleeOutcomeLikeCpp as Outcome, RepresentedMeleeVictimFactsLikeCpp as Victim,
        melee_outcome_inputs_like_cpp, melee_outcome_like_cpp,
    };

    // C++ `Unit.cpp:2364-2378` admits the band for a creature attacker four
    // levels above its victim, then evaluates `attackerLevel - victimLevel *
    // 1000 - 1500` verbatim. At 80 versus 76 that is -77,420, so the outcome
    // remains Hit even at the top of the roll range.
    let mut attacker = Attacker {
        level: 80,
        is_controlled_by_player: false,
        no_crushing_blows: false,
        ..Default::default()
    };
    let victim = Victim {
        level: 76,
        is_creature: true,
        ..Default::default()
    };
    let inputs = melee_outcome_inputs_like_cpp(&attacker, &victim);
    assert_eq!(inputs[0].crushing_chance_units, -77_420);
    assert_eq!(melee_outcome_like_cpp(&inputs[0], 9_999), Outcome::Hit);

    // Player-controlled creatures and NO_CRUSHING_BLOWS skip the band before
    // the source expression is evaluated.
    attacker.is_controlled_by_player = true;
    assert_eq!(
        melee_outcome_inputs_like_cpp(&attacker, &victim)[0].crushing_chance_units,
        0
    );
    attacker.is_controlled_by_player = false;
    attacker.no_crushing_blows = true;
    assert_eq!(
        melee_outcome_inputs_like_cpp(&attacker, &victim)[0].crushing_chance_units,
        0
    );
}

/// C++ `Unit::CalcAbsorbResist`'s school-absorb loop
/// (`Unit.cpp:2114-2178`).
///
/// The loop offers the damage to each shield in
/// `Trinity::AbsorbAuraOrderPred` order, clamps a negative (infinite) shield
/// amount to zero, clamps the consumed amount to the damage left and reports
/// the removal an amount-counting shield reaches at zero. Every case here is
/// the pure rule the map-owned swing and the session's aura transition share.
#[test]
fn represented_melee_absorb_matches_calc_absorb_resist_like_cpp() {
    use crate::session_rules::{
        RepresentedAbsorbShieldLikeCpp as Shield, represented_school_absorb_like_cpp,
    };

    let shield = |slot: u8, spell_id: i32, amount: i32| Shield {
        slot,
        effect_index: 0,
        spell_id,
        category_id: 0,
        amount,
    };

    // No shield: the damage passes through untouched and nothing is consumed.
    let none = represented_school_absorb_like_cpp(&[], 100);
    assert_eq!(none.absorbed, 0);
    assert_eq!(none.damage, 100);
    assert!(none.consumed.is_empty());

    // Zero damage returns before the loop (`if (!damageInfo.GetDamage()) return;`).
    let zero = represented_school_absorb_like_cpp(&[shield(0, 91_200, 500)], 0);
    assert_eq!(zero.absorbed, 0);
    assert_eq!(zero.damage, 0);
    assert!(zero.consumed.is_empty());

    // A shield larger than the hit absorbs it whole and survives with the
    // remainder.
    let partial = represented_school_absorb_like_cpp(&[shield(3, 91_200, 30)], 10);
    assert_eq!(partial.absorbed, 10);
    assert_eq!(partial.damage, 0);
    assert_eq!(partial.consumed.len(), 1);
    assert_eq!(partial.consumed[0].slot, 3);
    assert_eq!(partial.consumed[0].consumed, 10);
    assert_eq!(partial.consumed[0].remaining, 20);
    assert!(!partial.consumed[0].removed);

    // A shield exactly the size of the hit is spent and removed.
    let exact = represented_school_absorb_like_cpp(&[shield(4, 91_200, 10)], 10);
    assert_eq!(exact.absorbed, 10);
    assert_eq!(exact.damage, 0);
    assert_eq!(exact.consumed[0].remaining, 0);
    assert!(exact.consumed[0].removed);

    // A small shield absorbs what it can and the rest lands; the spent shield
    // is removed and C++ carries on with the reduced damage.
    let spill = represented_school_absorb_like_cpp(&[shield(5, 91_200, 4)], 10);
    assert_eq!(spill.absorbed, 4);
    assert_eq!(spill.damage, 6);
    assert_eq!(spill.consumed[0].consumed, 4);
    assert!(spill.consumed[0].removed);

    // A negative amount is an infinite-absorb script shield: without the
    // scripts C++ clamps it to zero, so it absorbs nothing and is never
    // removed here.
    let infinite = represented_school_absorb_like_cpp(&[shield(6, 91_200, -1)], 10);
    assert_eq!(infinite.absorbed, 0);
    assert_eq!(infinite.damage, 10);
    assert_eq!(infinite.consumed[0].consumed, 0);
    assert_eq!(infinite.consumed[0].remaining, -1);
    assert!(!infinite.consumed[0].removed);

    // `AbsorbAuraOrderPred` (`SpellAuraEffects.h:333-374`): Fel Blossom, then
    // the Ice Barrier category, then Sacrifice, then any other shield, with
    // Cauterize and Spirit of Redemption always last. The first shield in that
    // order spends first, and once the damage is gone the remaining shields are
    // not visited.
    let ordered = represented_school_absorb_like_cpp(
        &[
            shield(7, 91_201, 10), // plain shield, rank 3
            Shield {
                slot: 8,
                effect_index: 0,
                spell_id: 91_202,
                category_id: 471, // Ice Barrier, rank 1
                amount: 10,
            },
            shield(9, 86949, 10), // Cauterize, rank 4
        ],
        10,
    );
    assert_eq!(ordered.absorbed, 10);
    assert_eq!(ordered.damage, 0);
    assert_eq!(
        ordered.consumed.len(),
        1,
        "the loop stops once the damage is absorbed"
    );
    assert_eq!(
        ordered.consumed[0].slot, 8,
        "the Ice Barrier rank spends first"
    );
    assert!(ordered.consumed[0].removed);
}

/// `Trinity::AbsorbAuraOrderPred`'s named ranks, ranked for the stable sort the
/// represented loop uses (`SpellAuraEffects.h:333-374`).
#[test]
fn represented_absorb_priority_matches_absorb_aura_order_pred_like_cpp() {
    use crate::session_rules::{
        RepresentedAbsorbShieldLikeCpp as Shield, represented_absorb_priority_like_cpp,
    };

    let shield = |spell_id: i32, category_id: u32| Shield {
        slot: 0,
        effect_index: 0,
        spell_id,
        category_id,
        amount: 0,
    };
    let fel_blossom = represented_absorb_priority_like_cpp(&shield(28527, 0));
    let ice_barrier = represented_absorb_priority_like_cpp(&shield(11426, 471));
    let sacrifice = represented_absorb_priority_like_cpp(&shield(7812, 0));
    let plain = represented_absorb_priority_like_cpp(&shield(91_203, 0));
    let cauterize = represented_absorb_priority_like_cpp(&shield(86949, 0));
    let redemption = represented_absorb_priority_like_cpp(&shield(20711, 0));
    assert!(fel_blossom < ice_barrier);
    assert!(ice_barrier < sacrifice);
    assert!(sacrifice < plain);
    assert!(plain < cauterize);
    assert!(cauterize < redemption);
}

/// C++ `Unit::CalcAbsorbResist`'s mana-shield loop (`Unit.cpp:2179-2248`).
///
/// The shield's amount caps the damage it may take, the drain is that amount
/// scaled by `SpellEffectInfo::CalcValueMultiplier` (`Amplitude`), and the
/// absorbed damage scales down by the fraction of the drain the victim's mana
/// could pay.
#[test]
fn represented_melee_mana_absorb_matches_calc_absorb_resist_like_cpp() {
    use crate::session_rules::{
        RepresentedManaShieldLikeCpp as Shield, represented_mana_shield_absorb_like_cpp,
    };

    let shield = |slot: u8, amount: i32, mana_multiplier: f32| Shield {
        slot,
        effect_index: 0,
        spell_id: 91_520,
        amount,
        mana_multiplier,
    };

    // No shield and zero damage both return before the loop.
    let none = represented_mana_shield_absorb_like_cpp(&[], 10, 100);
    assert_eq!((none.absorbed, none.damage, none.mana_spent), (0, 10, 0));
    let zero = represented_mana_shield_absorb_like_cpp(&[shield(0, 30, 1.0)], 0, 100);
    assert_eq!((zero.absorbed, zero.damage, zero.mana_spent), (0, 0, 0));

    // Plenty of mana: the whole hit is absorbed, one point of mana per point of
    // damage, and the shield keeps the remainder.
    let full = represented_mana_shield_absorb_like_cpp(&[shield(1, 30, 1.0)], 10, 100);
    assert_eq!((full.absorbed, full.damage, full.mana_spent), (10, 0, 10));
    assert_eq!(full.consumed[0].remaining, 20);
    assert!(!full.consumed[0].removed);

    // The victim can only pay part of the drain, so only that fraction is
    // absorbed (`currentAbsorb * manaTaken / manaReduction`).
    let limited = represented_mana_shield_absorb_like_cpp(&[shield(1, 30, 1.0)], 10, 3);
    assert_eq!(
        (limited.absorbed, limited.damage, limited.mana_spent),
        (3, 7, 3)
    );
    assert_eq!(limited.consumed[0].remaining, 27);

    // `Amplitude` 2 drains two mana per absorbed point.
    let doubled = represented_mana_shield_absorb_like_cpp(&[shield(1, 30, 2.0)], 10, 100);
    assert_eq!(
        (doubled.absorbed, doubled.damage, doubled.mana_spent),
        (10, 0, 20)
    );

    // The shield's own amount caps the hit and a fully spent shield is removed.
    let capped = represented_mana_shield_absorb_like_cpp(&[shield(1, 4, 1.0)], 10, 100);
    assert_eq!(
        (capped.absorbed, capped.damage, capped.mana_spent),
        (4, 6, 4)
    );
    assert_eq!(capped.consumed[0].remaining, 0);
    assert!(capped.consumed[0].removed);

    // A negative amount is an infinite shield C++ clamps to zero for safety: it
    // absorbs nothing and is never removed by this loop.
    let negative = represented_mana_shield_absorb_like_cpp(&[shield(1, -1, 1.0)], 10, 100);
    assert_eq!(
        (negative.absorbed, negative.damage, negative.mana_spent),
        (0, 10, 0)
    );
    assert_eq!(negative.consumed[0].remaining, -1);
    assert!(!negative.consumed[0].removed);

    // No mana at all: nothing is absorbed and nothing is spent.
    let dry = represented_mana_shield_absorb_like_cpp(&[shield(1, 30, 1.0)], 10, 0);
    assert_eq!((dry.absorbed, dry.damage, dry.mana_spent), (0, 10, 0));
    assert_eq!(dry.consumed[0].remaining, 30);
    assert!(!dry.consumed[0].removed);
}

/// C++ `Unit::CalcHealAbsorb`'s heal-absorb loop (`Unit.cpp:2365-2425`) for one
/// heal.
///
/// Each shield's amount is clamped to the heal left, an amount-counting shield
/// is depleted and removed at zero, and a negative (infinite) shield is clamped
/// to zero and never removed. C++ has no priority sort and no ignore-absorb
/// term in this loop.
#[test]
fn represented_heal_absorb_matches_calc_heal_absorb_like_cpp() {
    use crate::session_rules::{
        RepresentedHealAbsorbShieldLikeCpp as Shield, represented_heal_absorb_like_cpp,
    };

    let shield = |slot: u8, amount: i32| Shield {
        slot,
        effect_index: 0,
        amount,
    };

    let none = represented_heal_absorb_like_cpp(&[], 20);
    assert_eq!((none.absorbed, none.heal), (0, 20));
    assert!(none.consumed.is_empty());

    let zero = represented_heal_absorb_like_cpp(&[shield(0, 30)], 0);
    assert_eq!((zero.absorbed, zero.heal), (0, 0));

    // A shield larger than the heal consumes it whole and keeps the remainder.
    let partial = represented_heal_absorb_like_cpp(&[shield(2, 30)], 20);
    assert_eq!((partial.absorbed, partial.heal), (20, 0));
    assert_eq!(partial.consumed[0].consumed, 20);
    assert_eq!(partial.consumed[0].remaining, 10);
    assert!(!partial.consumed[0].removed);

    // A shield exactly the size of the heal is spent and removed.
    let exact = represented_heal_absorb_like_cpp(&[shield(3, 20)], 20);
    assert_eq!((exact.absorbed, exact.heal), (20, 0));
    assert_eq!(exact.consumed[0].remaining, 0);
    assert!(exact.consumed[0].removed);

    // A small shield absorbs what it can; the rest of the heal lands.
    let spill = represented_heal_absorb_like_cpp(&[shield(4, 7)], 20);
    assert_eq!((spill.absorbed, spill.heal), (7, 13));
    assert!(spill.consumed[0].removed);

    // A negative amount is an infinite shield C++ clamps to zero and never
    // removes.
    let infinite = represented_heal_absorb_like_cpp(&[shield(5, -1)], 20);
    assert_eq!((infinite.absorbed, infinite.heal), (0, 20));
    assert_eq!(infinite.consumed[0].remaining, -1);
    assert!(!infinite.consumed[0].removed);

    // Shields are spent in aura order and the loop stops once the heal is gone.
    let ordered = represented_heal_absorb_like_cpp(&[shield(6, 8), shield(7, 8)], 20);
    assert_eq!((ordered.absorbed, ordered.heal), (16, 4));
    assert_eq!(ordered.consumed.len(), 2);
    assert_eq!(ordered.consumed[0].slot, 6);
    assert_eq!(ordered.consumed[1].slot, 7);
}

/// C++ `Unit::CalcAbsorbResist`'s `auraAbsorbMod` term (`Unit.cpp:2086-2112`,
/// `:2250`): an attacker's `SPELL_AURA_MOD_TARGET_ABSORB_SCHOOL` holds a share
/// of the hit out of both shield loops, and that share is added back to the
/// damage once the loops are done.
///
/// The share is *not* a per-shield term. In the 3.4.3 reference the loops only
/// ever read `damageInfo.GetDamage()`, and
/// `SPELL_ATTR6_ABSORB_CANNOT_BE_IGNORE` is declared in `SharedDefines.h:697`
/// but read nowhere in the server, so no shield is exempt.
#[test]
fn represented_ignore_absorb_matches_calc_absorb_resist_like_cpp() {
    use crate::session_rules::{
        AppliedAuraEffectLikeCpp, RepresentedAbsorbShieldLikeCpp as Shield,
        RepresentedManaShieldLikeCpp as ManaShield, represented_absorb_stages_like_cpp,
        represented_ignore_absorb_pct_like_cpp, represented_ignored_absorb_amount_like_cpp,
    };

    let effect = |misc_value: i32, amount: i32| AppliedAuraEffectLikeCpp {
        slot: 0,
        spell_id: 91_300,
        caster_guid: ObjectGuid::create_null(),
        aura_type: wow_data::spell::aura_types::SPELL_AURA_MOD_TARGET_ABSORB_SCHOOL,
        misc_value,
        misc_value_b: 0,
        amount,
    };

    // `GetMaxPositiveAuraModifierByMiscMask` then `RoundToInterval(0, 100)`.
    assert_eq!(represented_ignore_absorb_pct_like_cpp(&[], 0x01), 0.0);
    assert_eq!(
        represented_ignore_absorb_pct_like_cpp(&[effect(0x04, 50)], 0x01),
        0.0,
        "a different school's modifier does not match"
    );
    assert_eq!(
        represented_ignore_absorb_pct_like_cpp(&[effect(0x01, 50)], 0x01),
        50.0
    );
    assert_eq!(
        represented_ignore_absorb_pct_like_cpp(&[effect(0x01, 25), effect(0x01, 40)], 0x01),
        40.0
    );
    assert_eq!(
        represented_ignore_absorb_pct_like_cpp(&[effect(0x01, -10)], 0x01),
        0.0,
        "the maximum starts at zero"
    );
    assert_eq!(
        represented_ignore_absorb_pct_like_cpp(&[effect(0x01, 150)], 0x01),
        100.0
    );

    // `CalculatePct(damage, pct)` truncates.
    assert_eq!(represented_ignored_absorb_amount_like_cpp(100, 50.0), 50);
    assert_eq!(represented_ignored_absorb_amount_like_cpp(7, 50.0), 3);
    assert_eq!(represented_ignored_absorb_amount_like_cpp(100, 0.0), 0);

    let shield = |amount: i32| Shield {
        slot: 1,
        effect_index: 0,
        spell_id: 91_200,
        category_id: 0,
        amount,
    };
    let mana_shield = |amount: i32| ManaShield {
        slot: 2,
        effect_index: 0,
        spell_id: 91_520,
        amount,
        mana_multiplier: 1.0,
    };

    // Without the modifier a big enough shield eats the whole hit.
    let whole = represented_absorb_stages_like_cpp(&[shield(100)], &[], 100, 100, 0, 0.0);
    assert_eq!((whole.absorbed, whole.damage), (100, 0));
    assert_eq!(whole.school_consumed[0].remaining, 0);

    // With 50% ignored, half the hit is held out of the loop and comes back as
    // damage, so the shield can only spend the other half.
    let ignored = represented_absorb_stages_like_cpp(&[shield(100)], &[], 100, 100, 0, 50.0);
    assert_eq!((ignored.absorbed, ignored.damage), (50, 50));
    assert_eq!(
        ignored.school_consumed[0].remaining, 50,
        "the shield keeps what the held-out share stopped it from spending"
    );

    // The share is taken from the damage *before* the resist (`:2106` runs
    // before `ResistDamage` at `:2111`): 50% of 100 is held out even though the
    // shields only see the 60 the resist left.
    let resisted = represented_absorb_stages_like_cpp(&[shield(100)], &[], 100, 60, 0, 50.0);
    assert_eq!((resisted.absorbed, resisted.damage), (10, 50));

    // 100% ignored leaves the loops nothing to do at all.
    let all = represented_absorb_stages_like_cpp(&[shield(100)], &[], 100, 100, 0, 100.0);
    assert_eq!((all.absorbed, all.damage), (0, 100));
    assert!(all.school_consumed.is_empty());

    // Both loops run over the held-out damage in order, and the mana shield
    // takes what the school shield left.
    let both = represented_absorb_stages_like_cpp(
        &[shield(20)],
        &[mana_shield(100)],
        100,
        100,
        1_000,
        0.0,
    );
    assert_eq!(
        (both.absorbed, both.damage, both.mana_spent),
        (100, 0, 80),
        "the school shield spends 20 and the mana shield pays for the other 80"
    );
    assert_eq!(both.school_consumed.len(), 1);
    assert_eq!(both.mana_consumed.len(), 1);

    // The ignored share reaches the mana shield as well, and is restored once.
    let both_ignored = represented_absorb_stages_like_cpp(
        &[shield(20)],
        &[mana_shield(100)],
        100,
        100,
        1_000,
        50.0,
    );
    assert_eq!(
        (
            both_ignored.absorbed,
            both_ignored.damage,
            both_ignored.mana_spent
        ),
        (50, 50, 30)
    );
}

fn weapon_effect(index: u32, effect: u32, base_points: i32) -> wow_data::SpellEffectInfo {
    wow_data::SpellEffectInfo {
        effect_index: index,
        effect,
        effect_base_points: base_points,
        ..Default::default()
    }
}

#[test]
fn weapon_damage_effect_adds_the_fixed_bonus_once_like_cpp() {
    use wow_data::spell::spell_effect_types::SPELL_EFFECT_WEAPON_DAMAGE;
    // C++ `Spell::EffectWeaponDmg` (`SpellEffects.cpp:3550-3628`): a single
    // SPELL_EFFECT_WEAPON_DAMAGE adds its base points to the rolled weapon
    // damage exactly once.
    let effects = [weapon_effect(0, SPELL_EFFECT_WEAPON_DAMAGE, 40)];
    let (damage, normalized) =
        crate::session::spell_effects::weapon_damage_effect_amount_like_cpp(&effects, 0, 100)
            .expect("last weapon effect must calculate");
    assert_eq!(damage, 140);
    assert!(!normalized);
}

#[test]
fn weapon_damage_effect_applies_percent_mod_from_one_hundred_base_like_cpp() {
    use wow_data::spell::spell_effect_types::SPELL_EFFECT_WEAPON_PERCENT_DAMAGE;
    // C++ starts `weaponDamagePercentMod` at `1.0f` (`:3553`) and folds the
    // effect in with `ApplyPct(base, pct) => base * pct / 100` (`:3567`), then
    // multiplies the weapon damage by it (`:3611`). 150 base points therefore
    // means 150% of the rolled damage, not 150x.
    let effects = [weapon_effect(0, SPELL_EFFECT_WEAPON_PERCENT_DAMAGE, 150)];
    let (damage, _) =
        crate::session::spell_effects::weapon_damage_effect_amount_like_cpp(&effects, 0, 200)
            .expect("last weapon effect must calculate");
    assert_eq!(damage, 300);
}

#[test]
fn weapon_damage_effect_only_the_last_weapon_effect_calculates_like_cpp() {
    use wow_data::spell::spell_effect_types::{
        SPELL_EFFECT_WEAPON_DAMAGE, SPELL_EFFECT_WEAPON_PERCENT_DAMAGE,
    };
    // C++ `:3355-3367`: an earlier weapon-damage effect returns without
    // computing, so the spell is not counted twice. The later one handles all of
    // them at once.
    let effects = [
        weapon_effect(0, SPELL_EFFECT_WEAPON_DAMAGE, 40),
        weapon_effect(1, SPELL_EFFECT_WEAPON_PERCENT_DAMAGE, 200),
    ];
    assert!(
        crate::session::spell_effects::weapon_damage_effect_amount_like_cpp(&effects, 0, 100,)
            .is_none(),
        "the earlier weapon effect must defer to the last one"
    );
    let (damage, _) =
        crate::session::spell_effects::weapon_damage_effect_amount_like_cpp(&effects, 1, 100)
            .expect("the last weapon effect calculates");
    // Sequence matters (`:3598`): +40 for the WEAPON_DAMAGE arm, then x2.00 for
    // the percent arm => (100 + 40) * 2 = 280.
    assert_eq!(damage, 280);
}

#[test]
fn weapon_damage_effect_reports_normalized_and_never_goes_negative_like_cpp() {
    use wow_data::spell::spell_effect_types::SPELL_EFFECT_NORMALIZED_WEAPON_DMG;
    // `normalized` is reported so the caller can log the documented departure,
    // and C++ clamps the result with `std::max(weaponDamage, 0)` (`:3628`).
    let effects = [weapon_effect(0, SPELL_EFFECT_NORMALIZED_WEAPON_DMG, -500)];
    let (damage, normalized) =
        crate::session::spell_effects::weapon_damage_effect_amount_like_cpp(&effects, 0, 100)
            .expect("last weapon effect must calculate");
    assert!(
        normalized,
        "NORMALIZED_WEAPON_DMG must report normalization"
    );
    assert_eq!(damage, 0, "negative weapon damage is clamped to zero");
}

#[test]
fn the_test_only_roll_pin_is_scoped_and_leaves_the_table_intact() {
    use crate::session_rules::{
        RepresentedMeleeAttackerFactsLikeCpp as Attacker,
        RepresentedMeleeOutcomeLikeCpp as Outcome, RepresentedMeleeVictimFactsLikeCpp as Victim,
        TEST_MELEE_OUTCOME_ROLL_LIKE_CPP, melee_outcome_inputs_like_cpp,
        pin_melee_outcome_roll_like_cpp, rolled_melee_outcome_like_cpp,
    };

    // A level-2 creature victim facing a level-2 attacker: a 5% miss band plus
    // dodge and parry. The default test pin is the top of the roll range, which
    // loses every partial band, so a whole-tick test gets a landed swing instead
    // of an intermittent avoid.
    let attacker = Attacker {
        level: 2,
        ..Default::default()
    };
    let victim = Victim {
        level: 2,
        is_creature: true,
        dodge_pct: 3.0,
        parry_pct: 6.0,
        ..Default::default()
    };
    let inputs = melee_outcome_inputs_like_cpp(&attacker, &victim);
    assert_eq!(TEST_MELEE_OUTCOME_ROLL_LIKE_CPP, 9_999);
    assert_eq!(rolled_melee_outcome_like_cpp(&inputs[0]), Outcome::Hit);

    // A pin is scoped to its guard, so one test cannot change what the next one
    // on the same thread draws. At equal levels the first band is the 5% miss,
    // so the bottom of the range resolves there.
    {
        let _miss = pin_melee_outcome_roll_like_cpp(0);
        assert_eq!(rolled_melee_outcome_like_cpp(&inputs[0]), Outcome::Miss);
        {
            let _hit = pin_melee_outcome_roll_like_cpp(TEST_MELEE_OUTCOME_ROLL_LIKE_CPP);
            assert_eq!(rolled_melee_outcome_like_cpp(&inputs[0]), Outcome::Hit);
        }
        assert_eq!(rolled_melee_outcome_like_cpp(&inputs[0]), Outcome::Miss);
    }
    assert_eq!(rolled_melee_outcome_like_cpp(&inputs[0]), Outcome::Hit);
}

#[test]
fn white_swing_publishes_an_evade_like_cpp() {
    use wow_packet::packets::combat::{
        HIT_INFO_MISS, HIT_INFO_SWING_NO_HIT_SOUND, VICTIM_STATE_EVADES,
    };

    let (mut session, _, _) = make_session();
    let manager = shared_map_manager();
    let canonical = shared_canonical_map_manager();
    let guid = test_creature_guid(18_042);
    let player = ObjectGuid::create_player(1, 100);

    canonical.lock().unwrap().create_world_map(0, 0);
    session.set_canonical_map_manager(Arc::clone(&canonical));
    session.set_map_store(Arc::new(wow_data::MapStore::from_entries([
        wow_data::MapEntry {
            id: 0,
            instance_type: wow_data::map::MAP_COMMON,
            expansion_id: 0,
            parent_map_id: -1,
            cosmetic_parent_map_id: -1,
            flags1: 0,
            flags2: 0,
        },
    ])));
    session.attach_player_controller_like_cpp(SessionPlayerController::new(
        player,
        "Evade".to_string(),
        Position::new(10.0, 10.0, 0.0, 0.0),
        0,
        1,
        1,
        80,
        0,
    ));
    let _ = session.ensure_canonical_world_map_for_current_player_like_cpp();
    session
        .mutate_canonical_player_like_cpp(|player| {
            let unit = player.unit_mut();
            unit.set_attacking(Some(guid));
            unit.set_target(guid);
            unit.add_unit_state(UnitState::MELEE_ATTACKING.bits());
            unit.set_base_attack_time_like_cpp(WeaponAttackType::BaseAttack, 2_000);
            unit.set_attack_timer(WeaponAttackType::BaseAttack, 0);
            unit.set_weapon_damage(WeaponAttackType::BaseAttack, 7.0, 7.0);
        })
        .unwrap();
    session.combat_target = Some(guid);
    session.in_combat = true;
    register_test_creature(&mut session, manager.clone(), guid, 40);
    let swing = |session: &mut WorldSession| {
        let melee_damage_bonus = session.represented_melee_damage_bonus_like_cpp();
        let armor_mitigation = session.represented_melee_armor_mitigation_like_cpp();
        let outcome_facts = session.represented_melee_outcome_facts_like_cpp();
        let damage_taken = session.represented_melee_damage_taken_like_cpp();
        session
            .mutate_canonical_player_like_cpp(|player| {
                player
                    .unit_mut()
                    .set_attack_timer(WeaponAttackType::BaseAttack, 0);
                take_canonical_player_attack_swings_like_cpp(
                    player,
                    0,
                    true,
                    true,
                    true,
                    melee_damage_bonus,
                    armor_mitigation,
                    outcome_facts,
                    damage_taken,
                )
            })
            .flatten()
            .map(|(swings, _)| swings)
    };

    // A free victim lands a normal hit.
    assert_eq!(swing(&mut session).map(|s| s[0].damage), Some(7));

    // C++ `IsEvadingAttacks()` returns `MELEE_HIT_EVADE` before any band.
    session
        .mutate_world_creature(guid, |creature| {
            creature.creature.set_in_evade_mode_like_cpp(true);
        })
        .unwrap();
    let facts = session.represented_melee_outcome_facts_like_cpp();
    assert!(facts.1.is_evading_attacks);
    let swings = swing(&mut session).expect("white swing resolves");
    assert_eq!(swings[0].damage, 0);
    assert_eq!(
        swings[0].hit_info,
        HIT_INFO_MISS | HIT_INFO_SWING_NO_HIT_SOUND
    );
    assert_eq!(swings[0].victim_state, VICTIM_STATE_EVADES);
}
