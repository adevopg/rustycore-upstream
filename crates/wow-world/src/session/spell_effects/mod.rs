//! Represented spell-effect execution and the ticks that drive it.
//!
//! Separated from the Session root under #621.

use super::*;
mod checks;
mod combat;
mod destinations;
mod effect_apply;
mod effect_combat;
mod effect_summon;
mod effects;
mod effects_player;
mod effects_power;
mod effects_progress;
mod execution;
pub(in crate::session) use execution::weapon_damage_effect_amount_like_cpp;
mod execution_overloads;
mod spell_absorb;
mod spell_crit;
mod spell_resist;
mod spell_value;
#[cfg(test)]
pub(in crate::session) use spell_crit::PinnedSpellCritRollLikeCpp;
#[cfg(test)]
pub(in crate::session) use spell_resist::PinnedResistRollLikeCpp;
#[cfg(test)]
pub(in crate::session) use spell_value::PinnedCalcValueDieRollLikeCpp;
mod threat;
mod ticks;
