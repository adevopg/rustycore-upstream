//! Live melee engagement check: player attacks a real creature spawn.
//!
//! The MVP gameplay loop — visibility, movement, a landed white swing, the
//! creature's AI answering, its death and the XP that follows — had no live
//! evidence at all: every existing workflow either needs a pinned fixture with
//! an empty character or checks something other than combat. This mode drives
//! the loop over the wire against a spawn chosen by `creature_template.entry`
//! and reports what the server actually published, with nothing inferred.

use super::*;

pub(crate) const DEFAULT_MELEE_SMOKE_TIMEOUT_SECS: u64 = 60;
/// How close each heartbeat step may carry the player. C++ accepts client
/// movement, so the walk is deliberately made of ordinary-looking steps rather
/// than one teleport across the zone.
const MELEE_SMOKE_STEP_YARDS: f32 = 20.0;
/// Bound on the walk so a wrong target position cannot loop forever.
const MELEE_SMOKE_MAX_STEPS: u32 = 64;
/// How far from its SQL position a runtime spawn is still the same creature.
const MELEE_SMOKE_DISCOVERY_RADIUS_YARDS: f32 = 60.0;
/// How long the approach may keep listening for the target's CREATE block.
const MELEE_SMOKE_APPROACH_BUDGET: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub(crate) struct MeleeSmokeOptions {
    pub(crate) creature_entry: u32,
    pub(crate) creature_spawn_guid: Option<u64>,
    pub(crate) timeout_secs: u64,
}

/// What the server published during the engagement. Every field is an
/// observation, not a conclusion.
#[derive(Debug, Default, Clone, Serialize)]
pub(crate) struct MeleeSmokeOutcome {
    pub(crate) creature_entry: u32,
    pub(crate) creature_spawn_guid: u64,
    pub(crate) creature_runtime_counter: u64,
    pub(crate) walk_steps: u32,
    pub(crate) attack_start_seen: bool,
    pub(crate) player_swings_landed: u32,
    pub(crate) player_swings_avoided: u32,
    pub(crate) player_damage_dealt: i64,
    pub(crate) creature_swings_landed: u32,
    pub(crate) creature_damage_dealt: i64,
    pub(crate) creature_swings_avoided: u32,
    pub(crate) swings_resent: u32,
    pub(crate) target_death_seen: bool,
    pub(crate) xp_gain_seen: bool,
    pub(crate) xp_gained: u32,
}

/// One `world.creature` spawn plus the template facts the report needs.
#[derive(Debug, Clone)]
pub(crate) struct MeleeSmokeTarget {
    pub(crate) entry: u32,
    pub(crate) spawn_guid: u64,
    pub(crate) name: String,
    pub(crate) faction: u32,
    pub(crate) map_id: u16,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
}

pub(crate) fn resolve_melee_smoke_target(options: &MeleeSmokeOptions) -> Result<MeleeSmokeTarget> {
    use mysql::prelude::Queryable;

    let world_url = world_db_url()?;
    let opts = qa_mysql_opts(&world_url, "world")?;
    let mut world =
        mysql::Conn::new(opts).map_err(|error| anyhow!("Connect to world DB failed: {error}"))?;

    let row: Option<(u64, u32, String, u32, u32, f64, f64, f64)> = match options.creature_spawn_guid
    {
        Some(spawn_guid) => world
            .exec_first(
                "SELECT c.guid, c.id, ct.name, ct.faction, c.map, c.position_x, c.position_y, \
                 c.position_z FROM creature c JOIN creature_template ct ON ct.entry = c.id \
                 WHERE c.guid = ? AND c.id = ?",
                (spawn_guid, options.creature_entry),
            )
            .map_err(|error| anyhow!("Resolve melee target spawn {spawn_guid}: {error}"))?,
        None => world
            .exec_first(
                "SELECT c.guid, c.id, ct.name, ct.faction, c.map, c.position_x, c.position_y, \
                 c.position_z FROM creature c JOIN creature_template ct ON ct.entry = c.id \
                 WHERE c.id = ? AND c.phaseid = 0 AND c.phasegroup = 0 \
                 ORDER BY c.guid LIMIT 1",
                (options.creature_entry,),
            )
            .map_err(|error| {
                anyhow!(
                    "Resolve a melee target spawn for entry {}: {error}",
                    options.creature_entry
                )
            })?,
    };
    let Some((spawn_guid, entry, name, faction, map, x, y, z)) = row else {
        bail!(
            "no world.creature spawn matches entry {}{}",
            options.creature_entry,
            options
                .creature_spawn_guid
                .map(|guid| format!(" and guid {guid}"))
                .unwrap_or_default()
        );
    };
    let map_id =
        u16::try_from(map).map_err(|_| anyhow!("target map id does not fit protocol: {map}"))?;
    Ok(MeleeSmokeTarget {
        entry,
        spawn_guid,
        name,
        faction,
        map_id,
        x: x as f32,
        y: y as f32,
        z: z as f32,
    })
}

fn distance_between(from: (f32, f32, f32), to: (f32, f32, f32)) -> f32 {
    ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2) + (to.2 - from.2).powi(2)).sqrt()
}

/// One heartbeat step of at most [`MELEE_SMOKE_STEP_YARDS`] toward `to`,
/// facing it. The last step stops at melee reach instead of inside the target.
pub(crate) fn next_walk_step_like_cpp(
    from: (f32, f32, f32),
    to: (f32, f32, f32),
    stop_short: f32,
) -> Option<((f32, f32, f32), f32)> {
    let remaining = distance_between(from, to);
    let facing = (to.1 - from.1).atan2(to.0 - from.0);
    if remaining <= stop_short {
        return None;
    }
    let travel = (remaining - stop_short).min(MELEE_SMOKE_STEP_YARDS);
    let scale = travel / remaining;
    Some((
        (
            from.0 + (to.0 - from.0) * scale,
            from.1 + (to.1 - from.1) * scale,
            from.2 + (to.2 - from.2) * scale,
        ),
        facing,
    ))
}

/// Drive the whole engagement and report what the server published.
pub(crate) async fn run_melee_smoke(
    bot: &config::BotConfig,
    options: &MeleeSmokeOptions,
) -> Result<MeleeSmokeOutcome> {
    let bot_index = bot.account_id as usize;
    let target = tokio::task::spawn_blocking({
        let options = options.clone();
        move || resolve_melee_smoke_target(&options)
    })
    .await
    .map_err(|error| anyhow!("Melee target resolution worker failed: {error}"))??;
    info!(
        "[Bot {}] melee target: {:?} entry={} spawn={} faction={} map={} at ({:.1}, {:.1}, {:.1})",
        bot_index,
        target.name,
        target.entry,
        target.spawn_guid,
        target.faction,
        target.map_id,
        target.x,
        target.y,
        target.z
    );

    let mut outcome = MeleeSmokeOutcome {
        creature_entry: target.entry,
        creature_spawn_guid: target.spawn_guid,
        ..MeleeSmokeOutcome::default()
    };

    let (session_key, world_auth_context) = prepare_live_world_session_key_like_cpp(bot).await?;
    let authenticated = establish_encrypted_world_session_like_cpp(
        bot_index,
        &session_key,
        &world_auth_context.username,
        &world_auth_context.win64_auth_seed,
    )
    .await?;
    let mut connection = EncryptedWorldConnection {
        stream: authenticated.stream,
        crypt: authenticated.crypt,
        inflater: ServerPacketInflater::default(),
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(options.timeout_secs);
    let clock_origin = tokio::time::Instant::now();

    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_ENUM_CHARACTERS,
        &[],
    )
    .await?;
    expect_encrypted_opcode(
        &mut connection,
        SMSG_ENUM_CHARACTERS_RESULT,
        deadline,
        "SMSG_ENUM_CHARACTERS_RESULT",
    )
    .await?;

    let login_body = build_player_login(bot.character_guid, realm_id(), 500.0);
    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_PLAYER_LOGIN,
        &login_body,
    )
    .await?;
    let mut realm_connection: Option<EncryptedWorldConnection> = None;
    let mut login_update_objects: Vec<Vec<u8>> = Vec::new();
    expect_login_opcode_across_connect_to(
        bot_index,
        &mut connection,
        &mut realm_connection,
        &authenticated.derived_session_key,
        SMSG_LOGIN_VERIFY_WORLD,
        deadline,
        "SMSG_LOGIN_VERIFY_WORLD",
        Some(&mut login_update_objects),
    )
    .await?;
    info!(
        "[Bot {}] ✅ in the world ({} SMSG_UPDATE_OBJECT in the login burst)",
        bot_index,
        login_update_objects.len()
    );

    // C++ `Player::CanNeverSee` (`Entities/Player/Player.cpp:23214-23218`) hides
    // *every* object from a player that has not set
    // PLAYER_LOCAL_FLAG_OVERRIDE_TRANSPORT_SERVER_TIME, which the server does
    // when the client acknowledges its active mover. Without this packet the
    // visibility scan finds candidates and then rejects all of them, so no
    // creature is ever published.
    let active_mover_complete = build_move_init_active_mover_complete_payload(0);
    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_MOVE_INIT_ACTIVE_MOVER_COMPLETE,
        &active_mover_complete,
    )
    .await?;
    info!(
        "[Bot {}] ✅ CMSG_MOVE_INIT_ACTIVE_MOVER_COMPLETE sent",
        bot_index
    );

    let (player_low, player_high) = create_player_guid_raw(bot.character_guid, realm_id());

    // The walk starts where the character actually is. Reading the stored
    // position is read-only and avoids guessing from the login stream.
    let character_guid = bot.character_guid;
    let start_position = tokio::task::spawn_blocking(move || -> Result<(f32, f32, f32)> {
        use mysql::prelude::Queryable;
        let url = characters_db_url()?;
        let mut conn = mysql::Conn::new(qa_mysql_opts(&url, "characters")?)
            .map_err(|error| anyhow!("Connect to characters DB failed: {error}"))?;
        let row: Option<(f32, f32, f32, u16)> = conn
            .exec_first(
                "SELECT position_x, position_y, position_z, map FROM characters WHERE guid = ?",
                (character_guid,),
            )
            .map_err(|error| anyhow!("Read character position: {error}"))?;
        let (x, y, z, map) =
            row.ok_or_else(|| anyhow!("No characters row for guid {character_guid}"))?;
        Ok((x, y, z, map)).map(|(x, y, z, _)| (x, y, z))
    })
    .await
    .map_err(|error| anyhow!("Character position worker failed: {error}"))??;

    // Walk to the target with ordinary heartbeats, reading the stream as we go:
    // the runtime ObjectGuid only appears once the creature enters visibility,
    // which is itself the visibility row of the matrix.
    let mut position = start_position;
    let mut discovered: Option<DiscoveredCreatureGuid> = None;
    for payload in &login_update_objects {
        if discovered.is_some() {
            break;
        }
        discovered = find_creature_guid_near_position_in_update_object(
            payload,
            target.map_id,
            target.entry,
            target.x,
            target.y,
            target.z,
            MELEE_SMOKE_DISCOVERY_RADIUS_YARDS,
            None,
        );
    }
    // Walk toward the target and keep reading: C++ publishes the CREATE block of
    // everything in range from `SendInitialPacketsAfterAddToMap` onward, so the
    // spawn's live ObjectGuid can arrive after SMSG_LOGIN_VERIFY_WORLD and again
    // as the walk brings new cells into visibility.
    let approach_deadline = deadline.min(std::time::Instant::now() + MELEE_SMOKE_APPROACH_BUDGET);
    let mut steps = 0u32;
    loop {
        let read = tokio::time::timeout(
            Duration::from_millis(250),
            read_encrypted_packet(
                &mut connection.stream,
                &mut connection.crypt,
                &mut connection.inflater,
            ),
        )
        .await;
        match read {
            Ok(Ok((opcode, payload))) => {
                match opcode {
                    SMSG_TIME_SYNC_REQUEST => {
                        respond_to_detour_time_sync_like_cpp(
                            bot_index,
                            &mut connection.stream,
                            &mut connection.crypt,
                            &payload,
                            clock_origin,
                            "melee approach",
                        )
                        .await?;
                    }
                    SMSG_UPDATE_OBJECT if discovered.is_none() => {
                        discovered = find_creature_guid_near_position_in_update_object(
                            &payload,
                            target.map_id,
                            target.entry,
                            target.x,
                            target.y,
                            target.z,
                            MELEE_SMOKE_DISCOVERY_RADIUS_YARDS,
                            None,
                        );
                    }
                    _ => {}
                }
                continue;
            }
            Ok(Err(error)) => bail!("read error while approaching the target: {error}"),
            Err(_) => {}
        }

        let remaining = distance_between(position, (target.x, target.y, target.z));
        if discovered.is_some() && remaining <= NOMINAL_MELEE_RANGE_LIKE_CPP {
            break;
        }
        if std::time::Instant::now() >= approach_deadline {
            if discovered.is_none() {
                bail!(
                    "the target never appeared in SMSG_UPDATE_OBJECT within {:.0} yards of its \
                     SQL position after {steps} steps and {:.0}s of listening; without its live \
                     ObjectGuid there is nothing to attack",
                    MELEE_SMOKE_DISCOVERY_RADIUS_YARDS,
                    MELEE_SMOKE_APPROACH_BUDGET.as_secs_f32()
                );
            }
            bail!(
                "the approach stopped {remaining:.2} yards from the target; C++ nominal melee \
                 range is {NOMINAL_MELEE_RANGE_LIKE_CPP:.2}"
            );
        }
        let Some((next, facing)) = next_walk_step_like_cpp(
            position,
            (target.x, target.y, target.z),
            NOMINAL_MELEE_RANGE_LIKE_CPP - 1.0,
        ) else {
            // In reach already: keep listening for the spawn's CREATE block.
            continue;
        };
        if steps >= MELEE_SMOKE_MAX_STEPS {
            bail!("the walk did not reach the target in {MELEE_SMOKE_MAX_STEPS} steps");
        }
        let heartbeat =
            build_move_heartbeat_payload(player_low, player_high, next.0, next.1, next.2, facing);
        send_encrypted_packet(
            &mut connection.stream,
            &mut connection.crypt,
            CMSG_MOVE_HEARTBEAT,
            &heartbeat,
        )
        .await?;
        position = next;
        steps += 1;
        outcome.walk_steps = steps;
    }

    let distance = distance_between(position, (target.x, target.y, target.z));
    let runtime = discovered.expect("the approach loop only exits with a discovered target");
    info!(
        "[Bot {}] ✅ approached in {} steps, {:.2} yards from the target",
        bot_index, outcome.walk_steps, distance
    );
    outcome.creature_runtime_counter = runtime.low & OBJECT_GUID_COUNTER_MASK;
    info!(
        "[Bot {}] ✅ target discovered at runtime counter {}",
        bot_index, outcome.creature_runtime_counter
    );
    let target_guid = (runtime.low, runtime.high);
    let packed_target = build_packed_guid(runtime.low, runtime.high);
    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_ATTACK_SWING,
        &packed_target,
    )
    .await?;
    info!("[Bot {}] ✅ CMSG_ATTACK_SWING sent", bot_index);

    // Observe the engagement. Nothing here is asserted: the report says what the
    // server published, and only the mandatory observations below fail the run.
    let mut last_swing_seen = tokio::time::Instant::now();
    while std::time::Instant::now() < deadline {
        if outcome.target_death_seen && outcome.xp_gain_seen {
            break;
        }
        let read = tokio::time::timeout(
            Duration::from_millis(500),
            read_encrypted_packet(
                &mut connection.stream,
                &mut connection.crypt,
                &mut connection.inflater,
            ),
        )
        .await;
        match read {
            Ok(Ok((opcode, payload))) => match opcode {
                SMSG_TIME_SYNC_REQUEST => {
                    respond_to_detour_time_sync_like_cpp(
                        bot_index,
                        &mut connection.stream,
                        &mut connection.crypt,
                        &payload,
                        clock_origin,
                        "melee engagement",
                    )
                    .await?;
                }
                SMSG_ATTACK_START => {
                    let (attacker, victim) = parse_attack_start_guids_like_cpp(&payload)?;
                    if attacker == (player_low, player_high) && victim == target_guid {
                        outcome.attack_start_seen = true;
                        info!("[Bot {}] ✅ SMSG_ATTACK_START player → target", bot_index);
                    }
                }
                SMSG_ATTACKER_STATE_UPDATE => {
                    let update = parse_attacker_state_update_summary(&payload)
                        .context("malformed SMSG_ATTACKER_STATE_UPDATE")?;
                    let attacker = (update.attacker_guid_low, update.attacker_guid_high);
                    let victim = (update.victim_guid_low, update.victim_guid_high);
                    if attacker == (player_low, player_high) && victim == target_guid {
                        last_swing_seen = tokio::time::Instant::now();
                        if update.damage > 0 {
                            outcome.player_swings_landed += 1;
                            outcome.player_damage_dealt += i64::from(update.damage);
                        } else {
                            outcome.player_swings_avoided += 1;
                        }
                    } else if attacker == target_guid && victim == (player_low, player_high) {
                        if update.damage > 0 {
                            outcome.creature_swings_landed += 1;
                            outcome.creature_damage_dealt += i64::from(update.damage);
                        } else {
                            outcome.creature_swings_avoided += 1;
                        }
                    }
                }
                SMSG_ATTACK_STOP => {
                    if let Some(stop) = parse_attack_stop_summary(&payload) {
                        let attacker = (stop.attacker_guid_low, stop.attacker_guid_high);
                        let victim = (stop.victim_guid_low, stop.victim_guid_high);
                        if attacker == (player_low, player_high)
                            && victim == target_guid
                            && stop.now_dead
                        {
                            // C++ `AttackStop::NowDead` is the server saying the
                            // victim died; the health republished in
                            // SMSG_UPDATE_OBJECT is not decoded here, so this is
                            // the single witness this mode reports.
                            outcome.target_death_seen = true;
                            info!(
                                "[Bot {}] ✅ SMSG_ATTACK_STOP reports the target dead",
                                bot_index
                            );
                        }
                    }
                }
                SMSG_LOG_XP_GAIN => {
                    outcome.xp_gain_seen = true;
                    outcome.xp_gained = parse_log_xp_gain_amount(&payload).unwrap_or(0);
                    info!(
                        "[Bot {}] ✅ SMSG_LOG_XP_GAIN {} XP",
                        bot_index, outcome.xp_gained
                    );
                }
                _ => {}
            },
            Ok(Err(error)) => bail!("read error during the engagement: {error}"),
            Err(_) => {
                // C++ keeps swinging on the attack timer once `Unit::Attack`
                // took. If nothing lands for two seconds the repeat is not
                // happening, so resend the request and record that it was
                // needed instead of silently masking it.
                if outcome.attack_start_seen
                    && !outcome.target_death_seen
                    && last_swing_seen.elapsed() >= Duration::from_secs(2)
                {
                    send_encrypted_packet(
                        &mut connection.stream,
                        &mut connection.crypt,
                        CMSG_ATTACK_SWING,
                        &packed_target,
                    )
                    .await?;
                    outcome.swings_resent += 1;
                    last_swing_seen = tokio::time::Instant::now();
                }
            }
        }
    }

    info!(
        "[Bot {}] melee summary: attack_start={} player_landed={} ({} damage) avoided={} \
         creature_landed={} ({} damage) avoided={} resent={} death={} xp={}",
        bot_index,
        outcome.attack_start_seen,
        outcome.player_swings_landed,
        outcome.player_damage_dealt,
        outcome.player_swings_avoided,
        outcome.creature_swings_landed,
        outcome.creature_damage_dealt,
        outcome.creature_swings_avoided,
        outcome.swings_resent,
        outcome.target_death_seen,
        outcome.xp_gained
    );

    if !outcome.attack_start_seen {
        bail!("the server never published SMSG_ATTACK_START for the requested target");
    }
    if outcome.player_swings_landed == 0 {
        bail!(
            "no player swing landed: {} avoided, {} damage total",
            outcome.player_swings_avoided,
            outcome.player_damage_dealt
        );
    }
    Ok(outcome)
}

fn parse_log_xp_gain_amount(payload: &[u8]) -> Option<u32> {
    // C++ `WorldPackets::Combat::LogXPGain::Write`: packed victim ObjectGuid,
    // then int32 Original, uint8 Reason, int32 Amount.
    let (victim_len, _, _) = parse_packed_guid(payload)?;
    let start = victim_len + 4 + 1;
    let bytes: [u8; 4] = payload.get(start..start + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

/// The `--melee-smoke` entry point: one exclusive mode, one bot, one target.
pub(crate) async fn run_melee_smoke_mode(
    cli: &CliOptions,
    mut bots: Vec<config::BotConfig>,
) -> Result<()> {
    if cli.create_character {
        bail!("--melee-smoke needs an existing character; it never creates one");
    }
    if bots.len() != 1 {
        bail!(
            "--melee-smoke needs exactly one enabled bot; select it with --single (got {})",
            bots.len()
        );
    }
    let creature_entry = cli
        .melee_creature_entry
        .context("--melee-smoke requires --melee-creature-entry")?;
    let bot = bots.remove(0);
    if bot.password.trim().is_empty() {
        bail!(
            "No password for {}; export {}",
            bot.account,
            password_env_name(&bot.account)
        );
    }
    let options = MeleeSmokeOptions {
        creature_entry,
        creature_spawn_guid: cli.melee_creature_spawn_guid,
        timeout_secs: cli.melee_timeout_secs,
    };
    let outcome = run_melee_smoke(&bot, &options).await?;
    if let Some(path) = &cli.report_path {
        std::fs::write(path, serde_json::to_string_pretty(&outcome)?)
            .with_context(|| format!("cannot write melee report {path}"))?;
        info!("Melee report written to {path}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_walk_step_closes_at_most_one_stride_and_faces_the_target() {
        let from = (0.0, 0.0, 0.0);
        let to = (100.0, 0.0, 0.0);
        let (next, facing) = next_walk_step_like_cpp(from, to, 4.0).expect("a step is needed");
        assert!((next.0 - MELEE_SMOKE_STEP_YARDS).abs() < 0.001, "{next:?}");
        assert_eq!(next.1, 0.0);
        assert!(facing.abs() < 0.001, "facing east is orientation 0");
    }

    #[test]
    fn the_last_step_stops_short_instead_of_landing_inside_the_target() {
        // 6 yards out with a 4-yard stop distance: the step closes 2 yards, not 6.
        let (next, _) =
            next_walk_step_like_cpp((0.0, 0.0, 0.0), (6.0, 0.0, 0.0), 4.0).expect("a step");
        assert!((next.0 - 2.0).abs() < 0.001, "{next:?}");

        // Already inside the stop distance: no step at all.
        assert!(next_walk_step_like_cpp((0.0, 0.0, 0.0), (3.0, 0.0, 0.0), 4.0).is_none());
    }

    #[test]
    fn a_walk_step_faces_the_target_in_every_quadrant() {
        for (to, expected) in [
            ((0.0_f32, 50.0_f32, 0.0_f32), std::f32::consts::FRAC_PI_2),
            ((-50.0, 0.0, 0.0), std::f32::consts::PI),
            ((0.0, -50.0, 0.0), -std::f32::consts::FRAC_PI_2),
        ] {
            let (_, facing) = next_walk_step_like_cpp((0.0, 0.0, 0.0), to, 1.0).expect("a step");
            assert!((facing - expected).abs() < 0.001, "{to:?} -> {facing}");
        }
    }

    #[test]
    fn the_xp_gain_amount_is_read_at_the_cpp_offset() {
        // C++ `LogXPGain::Write` (`CharacterPackets.cpp:615`): packed victim
        // guid, int32 Original, uint8 Reason, int32 Amount, float GroupBonus.
        let mut payload = build_packed_guid(42, 0x0800_0400_0000_0000);
        payload.extend_from_slice(&120i32.to_le_bytes()); // Original
        payload.push(0); // Reason = Kill
        payload.extend_from_slice(&95i32.to_le_bytes()); // Amount
        payload.extend_from_slice(&1.0f32.to_le_bytes()); // GroupBonus
        assert_eq!(parse_log_xp_gain_amount(&payload), Some(95));

        // A truncated packet reports nothing rather than a wrong number.
        assert_eq!(
            parse_log_xp_gain_amount(&payload[..payload.len() - 6]),
            None
        );
        assert_eq!(parse_log_xp_gain_amount(&[]), None);
    }
}
