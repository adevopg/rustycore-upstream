//! Represented death, corpse and resurrection handling.
//!
//! Moved out of the Session root under #617. Behaviour is preserved; the
//! canonical owner of this state is unchanged.

use super::*;

impl WorldSession {
    fn apply_represented_player_environmental_death_like_cpp(&mut self) {
        // C++ `Player::EnvironmentalDamage` routes lethal damage through
        // `Unit::Kill` -> `Player::setDeathState(JUST_DIED)` before the client
        // proceeds into release/cemetery flows.
        let _ = self.with_owned_player_mut_like_cpp(|player| {
            player
                .unit_mut()
                .set_death_state(wow_constants::DeathState::JustDied);
            player.unit_mut().set_health(0);
        });
        #[cfg(test)]
        {
            self.player_health_like_cpp = 0;
            self.player_alive_like_cpp = false;
        }
        self.sync_player_registry_state_like_cpp();
    }
    /// C++ `CONFIG_DEATH_*` as `HandleReclaimCorpse` and
    /// `Map::ConvertCorpseToBones` read them.
    pub fn set_death_corpse_config_like_cpp(&mut self, config: DeathCorpseConfigLikeCpp) {
        self.death_corpse_config_like_cpp = config;
    }

    pub(crate) const fn death_corpse_config_like_cpp(&self) -> DeathCorpseConfigLikeCpp {
        self.death_corpse_config_like_cpp
    }

    pub(crate) fn apply_represented_resurrection_health_like_cpp(&mut self, health: u32) {
        let Some((_, max_health, _)) = self.resolved_player_vitals_like_cpp() else {
            return;
        };
        let _ = self.sync_canonical_player_health_like_cpp(health, max_health);
    }
    pub(crate) fn apply_represented_resurrection_percent_like_cpp(&mut self, restore_percent: f32) {
        let Some((_, max_health, _)) = self.resolved_player_vitals_like_cpp() else {
            return;
        };
        let health =
            ((f64::from(max_health) * f64::from(restore_percent)).floor() as u32).min(max_health);
        self.apply_represented_resurrection_health_like_cpp(health);
    }
    pub(in crate::session) fn player_resurrection_state_snapshot_like_cpp(
        &self,
    ) -> Option<PlayerResurrectionStateLikeCpp> {
        let canonical =
            self.with_owned_player_like_cpp(|player| player.resurrection_state_like_cpp().clone());
        #[cfg(test)]
        if canonical.is_none() && self.player_handle_like_cpp.is_none() {
            return Some(PlayerResurrectionStateLikeCpp {
                request: self.represented_resurrection_request_like_cpp,
                delayed_after_teleport: self
                    .represented_delayed_resurrection_after_teleport_like_cpp,
                self_res_spells: self.represented_self_res_spells_like_cpp.clone(),
                death_timer_active: self.represented_death_timer_active_like_cpp,
                area_spirit_healer_guid: self.area_spirit_healer_guid_like_cpp,
            });
        }
        canonical
    }
}

/// The four C++ `Death.*` world configuration values the corpse-reclaim and
/// bones paths read (`server/game/World/World.cpp:1307-1310`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeathCorpseConfigLikeCpp {
    /// C++ `CONFIG_DEATH_CORPSE_RECLAIM_DELAY_PVP` / `Death.CorpseReclaimDelay.PvP`.
    pub corpse_reclaim_delay_pvp: bool,
    /// C++ `CONFIG_DEATH_CORPSE_RECLAIM_DELAY_PVE` / `Death.CorpseReclaimDelay.PvE`.
    pub corpse_reclaim_delay_pve: bool,
    /// C++ `CONFIG_DEATH_BONES_WORLD` / `Death.Bones.World`.
    pub bones_world: bool,
    /// C++ `CONFIG_DEATH_BONES_BG_OR_ARENA` / `Death.Bones.BattlegroundOrArena`.
    pub bones_battleground_or_arena: bool,
}

impl Default for DeathCorpseConfigLikeCpp {
    /// The C++ code defaults, which are what `World::LoadConfigSettings` applies
    /// when the key is absent from the configuration file.
    fn default() -> Self {
        Self {
            corpse_reclaim_delay_pvp: true,
            corpse_reclaim_delay_pve: true,
            bones_world: true,
            bones_battleground_or_arena: true,
        }
    }
}
