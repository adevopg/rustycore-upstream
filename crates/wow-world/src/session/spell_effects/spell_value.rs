//! The represented `SpellEffectInfo::CalcValue`: what one effect of a cast is
//! worth before any bonus chain touches it.
//!
//! C++ never hands an effect handler `SpellEffectEntry::EffectBasePoints`. Every
//! handler reads `damage`, which `Spell::EffectHandler` filled from
//! `SpellEffectInfo::CalcValue(caster)` (`Spells/SpellInfo.cpp:496-597`), so the
//! `DieSides` roll and the per-level term are part of the value by the time any
//! effect sees it. The arithmetic is in `wow_data`; this module reads the state
//! it needs and owns the roll.

use super::*;

impl WorldSession {
    /// C++ `SpellEffectInfo::CalcValue(caster)` for one effect of a cast by the
    /// session's own player.
    ///
    /// Boundaries, each a fact this port does not carry rather than a choice:
    ///
    /// * combo points are passed as zero because no represented owner tracks
    ///   `Unit::GetComboPoints`, so the `PointsPerResource` term is inert. Only
    ///   66 of the 69,504 effects in the installed `SpellEffect.db2` carry a
    ///   non-zero one.
    /// * `WorldObject::ApplyEffectModifiers`'s spellmods (`SpellInfo.cpp:538-539`)
    ///   have no represented owner either.
    ///
    /// No `NpcManaCostScaler` table is passed, and that is not a gap: C++ gates
    /// the creature-level multiplication on `!IsControlledByPlayer()`
    /// (`SpellInfo.cpp:544`), so a player caster can never reach it.
    pub(in crate::session) fn represented_spell_effect_calc_value_like_cpp(
        &self,
        spell_id: i32,
        effect: &wow_data::SpellEffectInfo,
    ) -> i32 {
        let levels = self
            .spell_store()
            .map(|store| {
                store.spell_levels_for_difficulty_like_cpp(
                    spell_id,
                    self.current_map_difficulty_id_like_cpp(),
                    self.difficulty_store().map(std::sync::Arc::as_ref),
                )
            })
            .unwrap_or_default();
        let value = effect.calc_value_with_caster_and_die_roll_like_cpp(
            levels,
            Some(wow_data::spell::CalcValueCasterLikeCpp {
                level: u32::from(self.player_level_like_cpp()),
                combo_points: 0,
                is_controlled_by_player: true,
                scales_with_creature_level: false,
            }),
            None,
            represented_calc_value_die_roll_like_cpp,
        );
        // Kept at debug: it is the only place the inputs of a spell's value are
        // all visible at once, and the live run that proved D-H26 read exactly
        // these lines to separate the level term from the die roll.
        tracing::debug!(
            target: "RUSTYCORE_CALCVALUE_TRACE",
            spell_id,
            effect_index = effect.effect_index,
            base_points = effect.effect_base_points,
            die_sides = effect.effect_die_sides,
            real_points_per_level = effect.effect_real_points_per_level,
            base_level = levels.base_level,
            max_level = levels.max_level,
            spell_level = levels.spell_level,
            caster_level = self.player_level_like_cpp(),
            value,
            "CalcValue trace"
        );
        value
    }
}

/// C++ `irand(min, max)` (`SpellInfo.cpp:525`), pinnable so a scenario can assert
/// an exact value out of a spell whose range is real.
fn represented_calc_value_die_roll_like_cpp(min: i32, max: i32) -> i32 {
    #[cfg(test)]
    if let Some(pinned) = PINNED_CALC_VALUE_DIE_ROLL_LIKE_CPP.with(|cell| cell.get()) {
        return pinned.clamp(min.min(max), max.max(min));
    }
    wow_core::irand_like_cpp(min, max)
}

#[cfg(test)]
thread_local! {
    static PINNED_CALC_VALUE_DIE_ROLL_LIKE_CPP: std::cell::Cell<Option<i32>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(in crate::session) struct PinnedCalcValueDieRollLikeCpp;

#[cfg(test)]
impl PinnedCalcValueDieRollLikeCpp {
    /// Pin C++'s `irand` draw for this thread.
    pub(in crate::session) fn pin(draw: i32) -> Self {
        PINNED_CALC_VALUE_DIE_ROLL_LIKE_CPP.with(|cell| cell.set(Some(draw)));
        Self
    }
}

#[cfg(test)]
impl Drop for PinnedCalcValueDieRollLikeCpp {
    fn drop(&mut self) {
        PINNED_CALC_VALUE_DIE_ROLL_LIKE_CPP.with(|cell| cell.set(None));
    }
}
