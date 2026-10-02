//! The durable creature-runtime publishers of [`PlayerRegistry`].
//!
//! Every map-owned transition a session has to publish arrives through one of
//! these: the attack start and stop, the melee hit, the creature spell hit, the
//! player's own melee result, the visibility sends and the PvP combat expiry.
//! They share one generation check — a command resolved for a session that has
//! since reconnected is dropped here rather than delivered to the new one — and
//! that check is the reason they belong together rather than beside the entry
//! storage they do not touch.
//!
//! Extracted from `directory.rs` under the #584 C4 physical boundary when the
//! creature spell hit became the family's tenth member. Behaviour is unchanged;
//! every method keeps its original body.

use super::*;

impl PlayerRegistry {
    fn with_current_durable_runtime(
        &self,
        registration: PlayerRegistration,
    ) -> Option<Arc<Mutex<DurableCreatureRuntimeCommandsLikeCpp>>> {
        let entry = self.entries.get(&registration.guid)?;
        (entry.generation == registration.generation)
            .then(|| Arc::clone(&entry.durable_creature_runtime_commands_like_cpp))
    }

    pub fn publish_current_attack_start(
        &self,
        registration: PlayerRegistration,
        command: CreatureAttackStartLikeCppCommand,
    ) -> bool {
        self.with_current_durable_runtime(registration)
            .and_then(|durable| {
                durable
                    .lock()
                    .ok()
                    .map(|mut durable| durable.publish_attack_start_like_cpp(command))
            })
            .unwrap_or(false)
    }

    pub fn publish_current_attack_stop(
        &self,
        registration: PlayerRegistration,
        command: CreatureAttackStopLikeCppCommand,
    ) -> bool {
        self.with_current_durable_runtime(registration)
            .and_then(|durable| {
                durable
                    .lock()
                    .ok()
                    .map(|mut durable| durable.publish_attack_stop_like_cpp(command))
            })
            .unwrap_or(false)
    }

    pub fn publish_current_melee_damage(
        &self,
        registration: PlayerRegistration,
        command: ApplyCreatureMeleeDamageLikeCppCommand,
    ) -> bool {
        self.with_current_durable_runtime(registration)
            .and_then(|durable| {
                durable
                    .lock()
                    .ok()
                    .map(|mut durable| durable.publish_melee_damage_like_cpp(command))
            })
            .unwrap_or(false)
    }

    /// Publish one map-owned creature spell hit to its player victim.
    ///
    /// Generation-checked like every other current-incarnation publish: a hit
    /// resolved for a session that has since reconnected is dropped here rather
    /// than delivered to the new one.
    pub fn publish_current_creature_spell_damage(
        &self,
        registration: PlayerRegistration,
        command: ApplyCreatureSpellDamageLikeCppCommand,
    ) -> bool {
        self.with_current_durable_runtime(registration)
            .and_then(|durable| {
                durable
                    .lock()
                    .ok()
                    .map(|mut durable| durable.publish_creature_spell_damage_like_cpp(command))
            })
            .unwrap_or(false)
    }

    /// Publish one map-owned player auto-attack resolution to its attacker.
    ///
    /// Generation-checked like every other current-incarnation publish: a
    /// result resolved for a session that has since reconnected is dropped
    /// here rather than delivered to the new one.
    pub fn publish_current_player_melee_result(
        &self,
        registration: PlayerRegistration,
        command: ApplyPlayerMeleeResultLikeCppCommand,
    ) -> bool {
        self.with_current_durable_runtime(registration)
            .and_then(|durable| {
                durable
                    .lock()
                    .ok()
                    .map(|mut durable| durable.publish_player_melee_result_like_cpp(command))
            })
            .unwrap_or(false)
    }

    pub fn publish_current_send_if_visible(
        &self,
        registration: PlayerRegistration,
        command: SendIfVisibleLikeCppCommand,
    ) -> bool {
        self.with_current_durable_runtime(registration)
            .and_then(|durable| {
                durable
                    .lock()
                    .ok()
                    .map(|mut durable| durable.publish_send_if_visible_like_cpp(command))
            })
            .unwrap_or(false)
    }

    pub fn publish_current_destroy_visible_object(
        &self,
        registration: PlayerRegistration,
        command: DestroyVisibleObjectLikeCppCommand,
    ) -> bool {
        self.with_current_durable_runtime(registration)
            .and_then(|durable| {
                durable
                    .lock()
                    .ok()
                    .map(|mut durable| durable.publish_destroy_visible_object_like_cpp(command))
            })
            .unwrap_or(false)
    }

    pub fn publish_current_player_spell_if_visible(
        &self,
        registration: PlayerRegistration,
        command: SendPlayerSpellIfVisibleLikeCppCommand,
    ) -> bool {
        self.with_current_durable_runtime(registration)
            .and_then(|durable| {
                durable
                    .lock()
                    .ok()
                    .map(|mut queue| queue.publish_player_spell_if_visible_like_cpp(command))
            })
            .unwrap_or(false)
    }

    pub fn publish_current_creature_spell_cast_if_visible(
        &self,
        registration: PlayerRegistration,
        command: SendCreatureSpellCastIfVisibleLikeCppCommand,
    ) -> bool {
        self.with_current_durable_runtime(registration)
            .and_then(|durable| {
                durable.lock().ok().map(|mut durable| {
                    durable.publish_creature_spell_cast_if_visible_like_cpp(command)
                })
            })
            .unwrap_or(false)
    }

    pub fn publish_current_pvp_combat_expiry(
        &self,
        registration: PlayerRegistration,
        command: ReconcilePvpCombatExpiryLikeCppCommand,
    ) -> bool {
        self.with_current_durable_runtime(registration)
            .and_then(|durable| {
                durable.lock().ok().map(|mut durable| {
                    durable.publish_pvp_combat_expiry_like_cpp(command);
                    true
                })
            })
            .unwrap_or(false)
    }
}
