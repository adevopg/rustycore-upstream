//! The represented spell resist: resolving the facts C++ rolls a resist bucket
//! from, and taking that share out of the hit.
//!
//! C++ `Unit::CalcAbsorbResist` runs `CalcSpellResistedDamage` first for every
//! spell hit (`Entities/Unit/Unit.cpp:2080-2111`), after the critical arm and the
//! armour reduction and before the absorb shields. The rules are in
//! `session_rules::rules_6`; this module only reads the state they need.

use super::*;

/// One rolled spell resist for a direct-damage hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::session) struct RepresentedSpellResistLikeCpp {
    /// The damage left after the resist, which is what the victim takes.
    pub damage: u32,
    /// C++ `SpellNonMeleeDamage::resist`, published on the combat log.
    pub resisted: u32,
    /// C++ `HITINFO_FULL_RESIST` or `HITINFO_PARTIAL_RESIST`, or zero.
    pub hit_info: i32,
}

impl WorldSession {
    /// Roll C++'s resist for one spell hit on a creature target.
    ///
    /// Boundaries, each a fact this port does not carry rather than a choice:
    ///
    /// * `SPELL_ATTR0_CU_BINARY_SPELL` is taken as unset, so the level-based
    ///   resistance always applies. That is right for a plain direct-damage spell,
    ///   which is all this path serves, but the attribute's own rule
    ///   (`Spells/SpellMgr.cpp:3470-3520` plus the trigger pass at `:3608-3640`)
    ///   is not ported, so a binary spell would wrongly take the term.
    /// * the two ignore-resistance aura families
    ///   (`SPELL_AURA_MOD_ABILITY_IGNORE_TARGET_RESIST` and
    ///   `SPELL_AURA_MOD_IGNORE_TARGET_RESIST`) are zero, as is the Chaos Bolt
    ///   family exception.
    /// * a school mask carrying both normal and magic does not get C++'s
    ///   `min(resisted, armourReduction)` comparison (`Unit.cpp:2021-2028`),
    ///   because this port does not run the load-time pass that strips the normal
    ///   school and records `SPELL_ATTR0_CU_SCHOOLMASK_NORMAL_WITH_MAGIC`. Such a
    ///   spell resists by the magic rule alone.
    pub(in crate::session) fn represented_spell_resist_for_damage_like_cpp(
        &mut self,
        spell_id: Option<i32>,
        caster_guid: ObjectGuid,
        target_guid: ObjectGuid,
        school_mask: u32,
        damage: u32,
    ) -> RepresentedSpellResistLikeCpp {
        let unchanged = RepresentedSpellResistLikeCpp {
            damage,
            resisted: 0,
            hit_info: 0,
        };
        // C++ `CalcAbsorbResist` returns before the resist for a hit with no
        // damage, and the resist itself needs the spell's school.
        if damage == 0 || spell_id.is_none() {
            return unchanged;
        }
        let school_mask = u8::try_from(school_mask).unwrap_or(u8::MAX);
        if school_mask & crate::session_rules::SPELL_SCHOOL_MASK_MAGIC_LIKE_CPP == 0 {
            return unchanged;
        }

        let Some((victim_resistance, victim_level)) =
            self.mutate_world_creature(target_guid, |creature| {
                (
                    creature
                        .creature
                        .resistance_for_school_mask_like_cpp(school_mask),
                    creature.creature.unit().data().level,
                )
            })
        else {
            return unchanged;
        };
        let caster_is_player = caster_guid.is_player();
        let caster_terms = caster_is_player
            .then(|| {
                self.canonical_player_snapshot_like_cpp(|player| {
                    let stats = player.effective_combat_stats_like_cpp();
                    (stats.mod_target_resistance, stats.spell_penetration)
                })
            })
            .flatten()
            .unwrap_or((0, 0));

        let average_resist = crate::session_rules::represented_average_resist_reduction_like_cpp(
            &crate::session_rules::RepresentedAverageResistFactsLikeCpp {
                victim_resistance,
                caster_mod_target_resistance: caster_terms.0,
                caster_spell_penetration: caster_terms.1,
                school_mask,
                is_binary_spell: false,
                has_caster: true,
                victim_level: u8::try_from(victim_level).unwrap_or(u8::MAX),
                caster_level: self.player_level_like_cpp(),
            },
        );
        let resisted = crate::session_rules::represented_spell_resisted_damage_like_cpp(
            damage,
            school_mask,
            true,
            average_resist,
            0,
            represented_resist_roll_like_cpp(),
            None,
        )
        .min(damage);
        RepresentedSpellResistLikeCpp {
            damage: damage - resisted,
            resisted,
            hit_info: crate::session_rules::represented_resist_hit_info_like_cpp(damage, resisted),
        }
    }
}

/// C++ `rand_norm()` (`Unit.cpp:1994`), pinnable so a scenario can assert an exact
/// resisted share.
fn represented_resist_roll_like_cpp() -> f32 {
    #[cfg(test)]
    if let Some(pinned) = PINNED_RESIST_ROLL_LIKE_CPP.with(|cell| cell.get()) {
        return pinned;
    }
    wow_core::rand_norm_like_cpp()
}

#[cfg(test)]
thread_local! {
    static PINNED_RESIST_ROLL_LIKE_CPP: std::cell::Cell<Option<f32>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(in crate::session) struct PinnedResistRollLikeCpp;

#[cfg(test)]
impl PinnedResistRollLikeCpp {
    /// Pin C++'s `rand_norm()` draw for this thread.
    pub(in crate::session) fn pin(draw: f32) -> Self {
        PINNED_RESIST_ROLL_LIKE_CPP.with(|cell| cell.set(Some(draw)));
        Self
    }
}

#[cfg(test)]
impl Drop for PinnedResistRollLikeCpp {
    fn drop(&mut self) {
        PINNED_RESIST_ROLL_LIKE_CPP.with(|cell| cell.set(None));
    }
}
