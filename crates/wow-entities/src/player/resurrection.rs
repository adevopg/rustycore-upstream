use super::Player;
use crate::{PlayerResurrectionRequestLikeCpp, PlayerResurrectionStateLikeCpp};
use wow_core::ObjectGuid;

impl Player {
    pub fn resurrection_state_like_cpp(&self) -> &PlayerResurrectionStateLikeCpp {
        &self.gameplay_state.resurrection
    }

    pub fn resurrection_state_mut_like_cpp(&mut self) -> &mut PlayerResurrectionStateLikeCpp {
        &mut self.gameplay_state.resurrection
    }

    pub fn set_resurrection_request_like_cpp(&mut self, request: PlayerResurrectionRequestLikeCpp) {
        self.gameplay_state.resurrection.request = Some(request);
    }

    pub fn clear_resurrection_request_like_cpp(&mut self) {
        self.gameplay_state.resurrection.request = None;
    }

    pub fn take_resurrection_request_if_requested_by_like_cpp(
        &mut self,
        resurrecter: ObjectGuid,
    ) -> Option<PlayerResurrectionRequestLikeCpp> {
        if !self
            .gameplay_state
            .resurrection
            .request
            .is_some_and(|request| {
                !request.resurrecter.is_empty() && request.resurrecter == resurrecter
            })
        {
            return None;
        }
        self.gameplay_state.resurrection.request.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wow_core::Position;

    #[test]
    fn player_owns_resurrection_lifecycle_like_cpp() {
        let mut player = Player::new(Some(1), false);
        let resurrecter = ObjectGuid::create_player(1, 77);
        let request = PlayerResurrectionRequestLikeCpp {
            resurrecter,
            map_id: 571,
            position: Position::new(11.0, 22.0, 33.0, 1.5),
            health: 450,
            mana: 120,
            aura: 0,
        };

        player.set_resurrection_request_like_cpp(request);
        player
            .resurrection_state_mut_like_cpp()
            .self_res_spells
            .insert(21169);
        player
            .resurrection_state_mut_like_cpp()
            .delayed_after_teleport = Some(request);
        player.resurrection_state_mut_like_cpp().death_timer_active = true;
        player
            .resurrection_state_mut_like_cpp()
            .area_spirit_healer_guid = ObjectGuid::create_player(1, 88);

        assert_eq!(
            player.take_resurrection_request_if_requested_by_like_cpp(ObjectGuid::create_player(
                1, 78
            )),
            None
        );
        assert_eq!(
            player.take_resurrection_request_if_requested_by_like_cpp(resurrecter),
            Some(request)
        );
        assert!(player.resurrection_state_like_cpp().request.is_none());
        assert_eq!(
            player.resurrection_state_like_cpp().delayed_after_teleport,
            Some(request)
        );
        assert!(
            player
                .resurrection_state_like_cpp()
                .self_res_spells
                .contains(&21169)
        );
        assert!(player.resurrection_state_like_cpp().death_timer_active);
    }
}

/// C++ `copseReclaimDelay` (`Entities/Player/Player.cpp:141`) — the spelling is
/// the server's own.
pub const CORPSE_RECLAIM_DELAY_SECS_LIKE_CPP: [u32; MAX_DEATH_COUNT_LIKE_CPP] = [30, 60, 120];

/// C++ `MAX_DEATH_COUNT` (`Entities/Player/Player.cpp:139`).
pub const MAX_DEATH_COUNT_LIKE_CPP: usize = 3;

/// C++ `DEATH_EXPIRE_STEP` (`Entities/Player/Player.cpp:138`), five minutes.
pub const DEATH_EXPIRE_STEP_SECS_LIKE_CPP: i64 = 5 * 60;

/// C++ `Player::GetCorpseReclaimDelay` (`Entities/Player/Player.cpp:25297-25312`).
///
/// `pvp` selects which configuration flag decides whether the delay escalates at
/// all; with the flag off, a PvP death falls back to the first step and a PvE
/// death has no delay. `death_expire_time` is C++ `m_deathExpireTime`, and the
/// count is deliberately `ceil(x) - 1` rather than `floor(x)` — the `- 1` on the
/// expire time is the comment's own correction, kept here verbatim.
pub fn corpse_reclaim_delay_secs_like_cpp(
    pvp: bool,
    pvp_delay_enabled: bool,
    pve_delay_enabled: bool,
    now_secs: i64,
    death_expire_time_secs: i64,
) -> u32 {
    if pvp {
        if !pvp_delay_enabled {
            return CORPSE_RECLAIM_DELAY_SECS_LIKE_CPP[0];
        }
    } else if !pve_delay_enabled {
        return 0;
    }

    let count = if now_secs < death_expire_time_secs - 1 {
        (death_expire_time_secs - 1 - now_secs) / DEATH_EXPIRE_STEP_SECS_LIKE_CPP
    } else {
        0
    };
    let count = usize::try_from(count).unwrap_or(0);
    CORPSE_RECLAIM_DELAY_SECS_LIKE_CPP[count.min(MAX_DEATH_COUNT_LIKE_CPP - 1)]
}
