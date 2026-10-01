// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Represented spell resistance: the magic school's counterpart to armour.
//!
//! C++ `Unit::CalculateAverageResistReduction` (`Entities/Unit/Unit.cpp:2035-2077`)
//! and `Unit::CalcSpellResistedDamage` (`:1970-2003`), which `Unit::CalcAbsorbResist`
//! calls first for every spell hit (`:2080-2111`). Separated from the critical rules
//! in `rules_5` because this is the mitigation stage, not the amplification one.

/// C++ `SPELL_SCHOOL_MASK_NORMAL` (`SharedDefines.h:351`): the physical school,
/// mitigated by armour rather than by resistance.
pub(crate) const SPELL_SCHOOL_MASK_NORMAL_RESIST_LIKE_CPP: u8 = 0x01;
/// C++ `SPELL_SCHOOL_MASK_HOLY` (`:352`).
pub(crate) const SPELL_SCHOOL_MASK_HOLY_LIKE_CPP: u8 = 0x02;
/// C++ `SPELL_SCHOOL_MASK_MAGIC` (`:366`): holy plus the five spell schools.
pub(crate) const SPELL_SCHOOL_MASK_MAGIC_LIKE_CPP: u8 = 0x7E;

/// C++ `HITINFO_FULL_RESIST` / `HITINFO_PARTIAL_RESIST`
/// (`Entities/Unit/UnitDefines.h:454-455`), which `CalculateSpellDamageTaken`
/// adds to `SpellNonMeleeDamage::HitInfo` after the resist is known
/// (`Unit.cpp:1356-1357`).
pub(crate) const HITINFO_FULL_RESIST_LIKE_CPP: i32 = 0x0000_0080;
pub(crate) const HITINFO_PARTIAL_RESIST_LIKE_CPP: i32 = 0x0000_0100;

/// What C++ `Unit::CalculateAverageResistReduction` reads (`Unit.cpp:2035-2077`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RepresentedAverageResistFactsLikeCpp {
    /// C++ `victim->GetResistance(schoolMask)`: the smallest resistance among the
    /// schools in the mask.
    pub victim_resistance: i32,
    /// The caster's `SPELL_AURA_MOD_TARGET_RESISTANCE` sum for this school.
    pub caster_mod_target_resistance: i32,
    /// C++ `Player::GetSpellPenetrationItemMod`, subtracted for a player caster.
    pub caster_spell_penetration: i32,
    pub school_mask: u8,
    /// C++ `SPELL_ATTR0_CU_BINARY_SPELL`, which exempts the level-based term.
    pub is_binary_spell: bool,
    /// Whether there is a caster at all: C++ skips both the caster terms and the
    /// level-based term without one.
    pub has_caster: bool,
    /// C++ `victim->GetLevelForTarget(caster)`.
    pub victim_level: u8,
    /// C++ `caster->GetLevelForTarget(victim)`.
    pub caster_level: u8,
}

/// C++ `Unit::CalculateAverageResistReduction` (`Entities/Unit/Unit.cpp:2035-2077`),
/// returning the average fraction of the damage the victim resists.
///
/// Omitted deliberately and named: the Chaos Bolt exception at `:2054-2055`, which
/// zeroes the resistance for one spell id by family; it needs
/// `SpellInfo::SpellFamilyName`, which this port's spell metadata does not carry.
pub(crate) fn represented_average_resist_reduction_like_cpp(
    facts: &RepresentedAverageResistFactsLikeCpp,
) -> f32 {
    let mut victim_resistance = facts.victim_resistance as f32;
    if facts.has_caster {
        victim_resistance += facts.caster_mod_target_resistance as f32;
        victim_resistance -= facts.caster_spell_penetration as f32;
    }
    // Holy resistance exists in PvE and comes from the level difference, so the
    // template values are ignored for it.
    if facts.school_mask & SPELL_SCHOOL_MASK_HOLY_LIKE_CPP != 0 {
        victim_resistance = 0.0;
    }
    victim_resistance = victim_resistance.max(0.0);

    // The level-based term does not apply to binary spells and cannot be overcome
    // by spell penetration. Anyone below level 20 counts as level 20.
    if facts.has_caster && !facts.is_binary_spell {
        let victim_level = (facts.victim_level as f32).max(20.0);
        let caster_level = (facts.caster_level as f32).max(20.0);
        victim_resistance += ((victim_level - caster_level) * 5.0).max(0.0);
    }

    // C++ `bossLevel` 83 uses a flat constant instead of the level scaling.
    let level = u32::from(facts.victim_level).max(20);
    let resistance_constant = if level == 83 {
        510.0
    } else {
        level as f32 * 5.0
    };

    victim_resistance / (victim_resistance + resistance_constant)
}

/// The eleven discrete resist buckets C++ `CalcSpellResistedDamage` builds from
/// the average (`Entities/Unit/Unit.cpp:1982-1993`), in tenths of the damage.
pub(crate) fn represented_discrete_resist_probability_like_cpp(average_resist: f32) -> [f32; 11] {
    let mut probability = [0.0f32; 11];
    if average_resist <= 0.1 {
        probability[0] = 1.0 - 7.5 * average_resist;
        probability[1] = 5.0 * average_resist;
        probability[2] = 2.5 * average_resist;
    } else {
        for (bucket, value) in probability.iter_mut().enumerate() {
            *value = (0.5 - 2.5 * (0.1 * bucket as f32 - average_resist).abs()).max(0.0);
        }
    }
    probability
}

/// Which bucket C++'s `rand_norm()` draw selects (`Unit.cpp:1995-2002`). The draw
/// is a parameter so the owner keeps the only randomness.
pub(crate) fn represented_resist_bucket_for_roll_like_cpp(
    probability: &[f32; 11],
    roll: f32,
) -> u32 {
    let mut probability_sum = 0.0f32;
    for bucket in 0..11u32 {
        probability_sum += probability[bucket as usize];
        if roll < probability_sum {
            return bucket;
        }
    }
    // C++ leaves the loop variable at 11 when no bucket matched; the resisted
    // damage below then exceeds the damage and is clamped by the caller's use.
    11
}

/// C++ `Unit::CalcSpellResistedDamage` (`Entities/Unit/Unit.cpp:1970-2003`).
///
/// `armour_reduction` is C++'s `damage - CalcArmorReducedDamage(...)`, which only
/// the `SPELL_ATTR0_CU_SCHOOLMASK_NORMAL_WITH_MAGIC` comparison at `:2021-2028`
/// needs; the owner resolves it only for a mask that carries both normal and
/// magic, and passes `None` otherwise.
pub(crate) fn represented_spell_resisted_damage_like_cpp(
    damage: u32,
    school_mask: u8,
    victim_is_creature: bool,
    average_resist: f32,
    ignored_resistance_pct: i32,
    roll: f32,
    armour_reduction: Option<u32>,
) -> u32 {
    // Magic damage only: physical mitigation is armour.
    if school_mask & SPELL_SCHOOL_MASK_MAGIC_LIKE_CPP == 0 {
        return 0;
    }
    // NPCs can have holy resistance; a player victim cannot.
    if school_mask & SPELL_SCHOOL_MASK_HOLY_LIKE_CPP != 0 && !victim_is_creature {
        return 0;
    }

    let probability = represented_discrete_resist_probability_like_cpp(average_resist);
    let resistance = represented_resist_bucket_for_roll_like_cpp(&probability, roll);
    let mut damage_resisted = damage as f32 * resistance as f32 / 10.0;
    if damage_resisted > 0.0 {
        let ignored = ignored_resistance_pct.clamp(0, 100);
        damage_resisted = damage_resisted * (100 - ignored) as f32 / 100.0;
        if let Some(armour_reduction) = armour_reduction {
            // The weakest mitigation counts for a spell carrying both schools.
            damage_resisted = damage_resisted.min(armour_reduction as f32);
        }
    }
    damage_resisted.max(0.0) as u32
}

/// C++ `CalculateSpellDamageTaken`'s resist reporting (`Unit.cpp:1356-1357`): the
/// `HitInfo` bit depends on whether the resist took the whole hit.
pub(crate) fn represented_resist_hit_info_like_cpp(damage: u32, resisted: u32) -> i32 {
    if resisted == 0 {
        return 0;
    }
    if damage.saturating_sub(resisted) == 0 {
        HITINFO_FULL_RESIST_LIKE_CPP
    } else {
        HITINFO_PARTIAL_RESIST_LIKE_CPP
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> RepresentedAverageResistFactsLikeCpp {
        RepresentedAverageResistFactsLikeCpp {
            victim_resistance: 100,
            caster_mod_target_resistance: 0,
            caster_spell_penetration: 0,
            school_mask: 0x04, // fire
            is_binary_spell: false,
            has_caster: true,
            victim_level: 20,
            caster_level: 20,
        }
    }

    /// The average is `resistance / (resistance + level * 5)`, with level 20 as the
    /// floor for both sides.
    #[test]
    fn average_resist_reduction_follows_the_cpp_formula() {
        // 100 / (100 + 100)
        assert!((represented_average_resist_reduction_like_cpp(&facts()) - 0.5).abs() < 1e-6);

        // A level-60 victim against a level-20 caster adds (60 - 20) * 5 = 200.
        let mut higher = facts();
        higher.victim_level = 60;
        // 300 / (300 + 300)
        assert!((represented_average_resist_reduction_like_cpp(&higher) - 0.5).abs() < 1e-6);

        // A binary spell skips that term: 100 / (100 + 300).
        let mut binary = higher;
        binary.is_binary_spell = true;
        assert!((represented_average_resist_reduction_like_cpp(&binary) - 0.25).abs() < 1e-6);

        // Spell penetration lowers the resistance but not the level-based term.
        let mut penetrated = higher;
        penetrated.caster_spell_penetration = 100;
        // (100 - 100 + 200) / (200 + 300)
        assert!((represented_average_resist_reduction_like_cpp(&penetrated) - 0.4).abs() < 1e-6);

        // Level 83 uses the flat boss constant instead of level * 5.
        let mut boss = facts();
        boss.victim_level = 83;
        boss.caster_level = 83;
        // 100 / (100 + 510)
        assert!(
            (represented_average_resist_reduction_like_cpp(&boss) - 100.0 / 610.0).abs() < 1e-6
        );

        // Holy ignores the template resistance entirely, leaving only the level term.
        let mut holy = higher;
        holy.school_mask = SPELL_SCHOOL_MASK_HOLY_LIKE_CPP;
        // 200 / (200 + 300)
        assert!((represented_average_resist_reduction_like_cpp(&holy) - 0.4).abs() < 1e-6);

        // With no caster there are no caster terms and no level term.
        let mut no_caster = higher;
        no_caster.has_caster = false;
        no_caster.caster_spell_penetration = 500;
        // 100 / (100 + 300)
        assert!((represented_average_resist_reduction_like_cpp(&no_caster) - 0.25).abs() < 1e-6);
    }

    /// Below a tenth the table is the three-bucket form; above it, the triangular
    /// one. Both sum to one.
    #[test]
    fn the_discrete_resist_table_matches_cpp_in_both_forms() {
        let low = represented_discrete_resist_probability_like_cpp(0.04);
        assert!((low[0] - (1.0 - 7.5 * 0.04)).abs() < 1e-6);
        assert!((low[1] - 0.2).abs() < 1e-6);
        assert!((low[2] - 0.1).abs() < 1e-6);
        assert_eq!(low[3..].iter().copied().fold(0.0f32, f32::max), 0.0);
        assert!((low.iter().sum::<f32>() - 1.0).abs() < 1e-6);

        let high = represented_discrete_resist_probability_like_cpp(0.5);
        assert!((high[5] - 0.5).abs() < 1e-6);
        assert!((high[4] - 0.25).abs() < 1e-6);
        assert!((high[6] - 0.25).abs() < 1e-6);
        assert_eq!(high[0], 0.0);
        assert!((high.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    /// The bucket is the first whose cumulative probability exceeds the draw, and
    /// the resisted damage is that many tenths.
    #[test]
    fn the_resist_bucket_and_damage_follow_the_roll_like_cpp() {
        // At an average of 0.5 the table is 0.25 at bucket 4, 0.5 at 5 and 0.25 at
        // 6, so the cumulative edges are 0.25 and 0.75.
        let probability = represented_discrete_resist_probability_like_cpp(0.5);
        assert_eq!(
            represented_resist_bucket_for_roll_like_cpp(&probability, 0.1),
            4
        );
        assert_eq!(
            represented_resist_bucket_for_roll_like_cpp(&probability, 0.5),
            5
        );
        assert_eq!(
            represented_resist_bucket_for_roll_like_cpp(&probability, 0.99),
            6
        );

        // 0.5 average with a draw of 0.5 lands in bucket 5, which is half the damage.
        assert_eq!(
            represented_spell_resisted_damage_like_cpp(200, 0x04, true, 0.5, 0, 0.5, None),
            100
        );
        // A draw in the first bucket resists nothing.
        assert_eq!(
            represented_spell_resisted_damage_like_cpp(200, 0x04, true, 0.02, 0, 0.0, None),
            0
        );
    }

    /// The school gates come before the roll, and the ignore and armour terms
    /// after it.
    #[test]
    fn the_resist_school_gates_and_reductions_follow_cpp_order() {
        // A physical spell is never resisted.
        assert_eq!(
            represented_spell_resisted_damage_like_cpp(
                200,
                SPELL_SCHOOL_MASK_NORMAL_RESIST_LIKE_CPP,
                true,
                0.5,
                0,
                0.5,
                None
            ),
            0
        );
        // Holy resists only on an NPC victim.
        assert_eq!(
            represented_spell_resisted_damage_like_cpp(
                200,
                SPELL_SCHOOL_MASK_HOLY_LIKE_CPP,
                false,
                0.5,
                0,
                0.5,
                None
            ),
            0
        );
        assert_eq!(
            represented_spell_resisted_damage_like_cpp(
                200,
                SPELL_SCHOOL_MASK_HOLY_LIKE_CPP,
                true,
                0.5,
                0,
                0.5,
                None
            ),
            100
        );
        // `MOD_IGNORE_TARGET_RESIST` removes a percentage of the resisted amount
        // and is capped at 100.
        assert_eq!(
            represented_spell_resisted_damage_like_cpp(200, 0x04, true, 0.5, 25, 0.5, None),
            75
        );
        assert_eq!(
            represented_spell_resisted_damage_like_cpp(200, 0x04, true, 0.5, 400, 0.5, None),
            0
        );
        // With both schools, the weakest mitigation counts.
        assert_eq!(
            represented_spell_resisted_damage_like_cpp(200, 0x04, true, 0.5, 0, 0.5, Some(30)),
            30
        );
        assert_eq!(
            represented_spell_resisted_damage_like_cpp(200, 0x04, true, 0.5, 0, 0.5, Some(150)),
            100
        );
    }

    /// C++ distinguishes a resist that took everything from one that took part.
    #[test]
    fn the_resist_hit_info_bit_follows_cpp() {
        assert_eq!(represented_resist_hit_info_like_cpp(100, 0), 0);
        assert_eq!(
            represented_resist_hit_info_like_cpp(100, 40),
            HITINFO_PARTIAL_RESIST_LIKE_CPP
        );
        assert_eq!(
            represented_resist_hit_info_like_cpp(100, 100),
            HITINFO_FULL_RESIST_LIKE_CPP
        );
    }
}
