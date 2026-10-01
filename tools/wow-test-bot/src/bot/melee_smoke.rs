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
/// One heartbeat every this often, so the walk looks like a client moving rather
/// than a burst of teleports.
pub(crate) const MELEE_SMOKE_STEP_INTERVAL: Duration = Duration::from_millis(200);
/// How long the approach may keep listening for the target's CREATE block.
const MELEE_SMOKE_APPROACH_BUDGET: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub(crate) struct MeleeSmokeOptions {
    pub(crate) creature_entry: u32,
    pub(crate) creature_spawn_guid: Option<u64>,
    pub(crate) timeout_secs: u64,
    /// Loot the corpse the kill leaves behind and verify what it granted.
    pub(crate) loot_after_kill: bool,
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
    pub(crate) loot_response_seen: bool,
    pub(crate) loot_coins: u32,
    pub(crate) loot_items_offered: u32,
    pub(crate) loot_item_requests: u32,
    pub(crate) money_before: Option<u64>,
    pub(crate) money_after: Option<u64>,
    pub(crate) inventory_items_before: Option<u32>,
    pub(crate) inventory_items_after: Option<u32>,
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

pub(crate) fn distance_between(from: (f32, f32, f32), to: (f32, f32, f32)) -> f32 {
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
    let mut last_step = std::time::Instant::now() - MELEE_SMOKE_STEP_INTERVAL;
    loop {
        // The frame read must not be cancellable: a timeout that fires mid-frame
        // leaves the stream and the cipher out of step, which the next read
        // reports as "Invalid encrypted packet size". `read_encrypted_packet_if_ready`
        // peeks for readiness first, which is cancellation-safe.
        let read = read_encrypted_packet_if_ready(
            &mut connection.stream,
            &mut connection.crypt,
            &mut connection.inflater,
            Duration::from_millis(250),
            Duration::from_secs(5),
            "melee approach",
        )
        .await;
        match read {
            Ok(Some((opcode, payload))) => match opcode {
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
                SMSG_UPDATE_OBJECT => {
                    // Keep reading after the first sighting: the walk must close
                    // on where the creature *is*, not on its SQL spawn row. A
                    // wandering spawn can stand tens of yards away from that
                    // row, and then the swings resolve out of melee range —
                    // C++ `Unit::DoMeleeAttackIfReady` only swings inside
                    // `IsWithinMeleeRange`, so the server published nothing and
                    // the run reported an unexplained `player_landed=0`.
                    if let Some(sighting) = find_creature_guid_near_position_in_update_object(
                        &payload,
                        target.map_id,
                        target.entry,
                        target.x,
                        target.y,
                        target.z,
                        MELEE_SMOKE_DISCOVERY_RADIUS_YARDS,
                        discovered.map(|found| found.low),
                    ) {
                        discovered = Some(sighting);
                    }
                }
                _ => {}
            },
            Ok(None) => {}
            Err(error) => bail!("read error while approaching the target: {error}"),
        }

        // Fall through on both arms. The stream is busy — time sync plus every
        // creature's movement — so a `continue` here would keep reading packets
        // and never walk; the step cadence is its own timer instead.
        if last_step.elapsed() < MELEE_SMOKE_STEP_INTERVAL {
            continue;
        }
        let aim = discovered
            .map(|found| (found.x, found.y, found.z))
            .unwrap_or((target.x, target.y, target.z));
        let remaining = distance_between(position, aim);
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
        let Some((next, facing)) =
            next_walk_step_like_cpp(position, aim, NOMINAL_MELEE_RANGE_LIKE_CPP - 1.0)
        else {
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
        last_step = std::time::Instant::now();
        outcome.walk_steps = steps;
    }

    let runtime = discovered.expect("the approach loop only exits with a discovered target");
    let distance = distance_between(position, (runtime.x, runtime.y, runtime.z));
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
    //
    // The walk continues here: a wandering spawn steps out of melee reach while
    // the swing timer runs, and C++ `Unit::DoMeleeAttackIfReady` then publishes
    // `SMSG_ATTACKSWING_ERROR` instead of a swing — the server is right and a
    // standing bot simply never connects. Following the target is what a player
    // does, and it is what the first landed swing needs: once damage lands the
    // creature engages and chases by itself.
    let mut last_swing_seen = tokio::time::Instant::now();
    let mut target_position = (runtime.x, runtime.y, runtime.z);
    let mut last_follow_step = std::time::Instant::now();
    while std::time::Instant::now() < deadline {
        if outcome.target_death_seen && outcome.xp_gain_seen {
            break;
        }
        // In reach the walk stops but the turn does not: C++
        // `Unit::DoMeleeAttackIfReady` also needs `HasInArc(2*pi/3, victim)`, and
        // a target circling a standing bot leaves the last heartbeat's facing
        // behind, which the server answers with `SMSG_ATTACKSWING_ERROR` and no
        // swing. Keep publishing the facing at the current position.
        let follow_step = (!outcome.target_death_seen
            && last_follow_step.elapsed() >= MELEE_SMOKE_STEP_INTERVAL)
            .then(|| {
                next_walk_step_like_cpp(
                    position,
                    target_position,
                    NOMINAL_MELEE_RANGE_LIKE_CPP - 1.0,
                )
                .unwrap_or((
                    position,
                    (target_position.1 - position.1).atan2(target_position.0 - position.0),
                ))
            });
        if let Some((next, facing)) = follow_step {
            let heartbeat = build_move_heartbeat_payload(
                player_low,
                player_high,
                next.0,
                next.1,
                next.2,
                facing,
            );
            send_encrypted_packet(
                &mut connection.stream,
                &mut connection.crypt,
                CMSG_MOVE_HEARTBEAT,
                &heartbeat,
            )
            .await?;
            position = next;
            outcome.walk_steps += 1;
            last_follow_step = std::time::Instant::now();
        }
        // `SMSG_LOG_XP_GAIN` is `CONNECTION_TYPE_REALM` in C++
        // (`Server/Protocol/Opcodes.cpp:1662`), so the kill's XP never arrives on
        // the instance socket this mode attacks through. Reading only that socket
        // reported `xp=0` for kills the server had already granted and persisted.
        // `peek` waits for real bytes without consuming them, so losing the
        // select is safe — the same shape the stand-state drain uses.
        let ready = {
            let quiet = tokio::time::sleep(Duration::from_millis(500));
            tokio::pin!(quiet);
            let mut instance_peek = [0u8; 1];
            let mut realm_peek = [0u8; 1];
            match realm_connection.as_ref() {
                Some(realm) => tokio::select! {
                    result = connection.stream.peek(&mut instance_peek) => {
                        if result.context("instance melee engagement peek failed")? == 0 {
                            bail!("instance connection closed during the engagement");
                        }
                        MeleeReadSource::Instance
                    }
                    result = realm.stream.peek(&mut realm_peek) => {
                        if result.context("realm melee engagement peek failed")? == 0 {
                            bail!("realm connection closed during the engagement");
                        }
                        MeleeReadSource::Realm
                    }
                    _ = &mut quiet => MeleeReadSource::Quiet,
                },
                None => tokio::select! {
                    result = connection.stream.peek(&mut instance_peek) => {
                        if result.context("instance melee engagement peek failed")? == 0 {
                            bail!("instance connection closed during the engagement");
                        }
                        MeleeReadSource::Instance
                    }
                    _ = &mut quiet => MeleeReadSource::Quiet,
                },
            }
        };
        let read = match ready {
            MeleeReadSource::Instance => Ok(read_encrypted_packet(
                &mut connection.stream,
                &mut connection.crypt,
                &mut connection.inflater,
            )
            .await),
            MeleeReadSource::Realm => {
                let realm = realm_connection
                    .as_mut()
                    .expect("the realm branch only runs with a realm connection");
                Ok(
                    read_encrypted_packet(&mut realm.stream, &mut realm.crypt, &mut realm.inflater)
                        .await,
                )
            }
            MeleeReadSource::Quiet => Err(()),
        };
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
                SMSG_ON_MONSTER_MOVE => {
                    if let Ok((mover, moved_to)) =
                        monster_move_mover_and_position_like_cpp(&payload)
                    {
                        if mover == target_guid {
                            target_position = moved_to;
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

    if options.loot_after_kill && outcome.target_death_seen {
        let (money, items) = character_money_and_item_count_like_cpp(bot.character_guid)?;
        outcome.money_before = Some(money);
        outcome.inventory_items_before = Some(items);
        loot_the_corpse_like_cpp(
            bot_index,
            bot.character_guid,
            &mut connection,
            target_guid,
            deadline,
            clock_origin,
            &mut outcome,
        )
        .await?;
        // The grants are persisted by the save a clean logout performs, so read
        // them back from the columns `Player::SaveToDB` writes rather than
        // decoding an update block.
        send_encrypted_packet(
            &mut connection.stream,
            &mut connection.crypt,
            CMSG_LOGOUT_REQUEST,
            &[0],
        )
        .await?;
        let logout_deadline =
            std::time::Instant::now() + Duration::from_secs(NORMAL_LOGOUT_COMPLETE_WAIT_SECS);
        let logout_route = realm_connection.as_mut().unwrap_or(&mut connection);
        expect_encrypted_opcode(
            logout_route,
            SMSG_LOGOUT_COMPLETE,
            logout_deadline,
            "SMSG_LOGOUT_COMPLETE",
        )
        .await?;
        let (money, items) = character_money_and_item_count_like_cpp(bot.character_guid)?;
        outcome.money_after = Some(money);
        outcome.inventory_items_after = Some(items);
        info!(
            "[Bot {}] ✅ logged out: money {} -> {}, inventory rows {} -> {}",
            bot_index,
            outcome.money_before.unwrap_or(0),
            money,
            outcome.inventory_items_before.unwrap_or(0),
            items
        );
    }

    info!(
        "[Bot {}] melee summary: attack_start={} player_landed={} ({} damage) avoided={} \
         creature_landed={} ({} damage) avoided={} resent={} death={} xp={} \
         loot_coins={} loot_items={} money={:?}->{:?} inv={:?}->{:?}",
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
        outcome.xp_gained,
        outcome.loot_coins,
        outcome.loot_items_offered,
        outcome.money_before,
        outcome.money_after,
        outcome.inventory_items_before,
        outcome.inventory_items_after
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

/// Which socket had bytes ready, or neither within the quiet window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeleeReadSource {
    Instance,
    Realm,
    Quiet,
}

/// CMSG_LOOT_UNIT.
const CMSG_LOOT_UNIT: u16 = 0x320F;
/// CMSG_LOOT_MONEY.
const CMSG_LOOT_MONEY: u16 = 0x3210;
/// CMSG_LOOT_ITEM.
const CMSG_LOOT_ITEM: u16 = 0x3211;
/// CMSG_LOOT_RELEASE.
const CMSG_LOOT_RELEASE: u16 = 0x3213;
/// SMSG_LOOT_RESPONSE.
const SMSG_LOOT_RESPONSE: u16 = 0x2614;

/// One entry of a loot window: the id the client must quote back, and what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LootWindowItem {
    pub(crate) loot_list_id: u8,
    pub(crate) item_id: i32,
    pub(crate) quantity: u32,
}

/// Decode C++ `LootResponse::Write`.
///
/// The prefix is two packed guids, four `uint8` reasons, the coins and the two
/// list counts, then a bit byte. Each `LootItemData` block is its own packed bits
/// (`item_type`, `ui_type`, `can_trade_to_tap_list`), an `ItemInstance`, the
/// quantity, the loot item type and finally the loot list id.
///
/// The id matters: `handlers/loot/authority.rs:127` resolves the client's request
/// against `loot.items` by that stored id, and
/// `handlers/loot/generation.rs:172` assigns it over **every** generated entry,
/// including ones this player never sees. Assuming `0..count` instead of reading
/// it back asks for the wrong slot, and the server declines in silence.
///
/// A block carrying item bonuses or modifications is refused rather than guessed:
/// their lengths are variable and nothing in a white-loot scenario produces them.
fn parse_loot_response_like_cpp(payload: &[u8]) -> Result<((u64, u64), u32, Vec<LootWindowItem>)> {
    let mut offset = 0usize;
    let (owner_len, _, _) = parse_packed_guid(payload.get(offset..).unwrap_or_default())
        .context("SMSG_LOOT_RESPONSE is missing its owner guid")?;
    offset += owner_len;
    // The second guid is the LootObject, which is what `CMSG_LOOT_ITEM` must
    // quote back: the handler resolves the request through
    // `active_loot_owner_for_loot_object_like_cpp`, so sending the creature's own
    // guid there finds nothing and is answered with a bare SMSG_LOOT_RELEASE.
    let (loot_obj_len, loot_obj_low, loot_obj_high) =
        parse_packed_guid(payload.get(offset..).unwrap_or_default())
            .context("SMSG_LOOT_RESPONSE is missing its loot object guid")?;
    offset += loot_obj_len;
    offset += 4; // failure_reason, acquire_reason, loot_method, threshold
    let read_u32 = |at: usize| -> Result<u32> {
        let bytes = payload
            .get(at..at + 4)
            .context("SMSG_LOOT_RESPONSE ended inside a u32")?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    let coins = read_u32(offset)?;
    let item_count = read_u32(offset + 4)?;
    let currency_count = read_u32(offset + 8)?;
    offset += 12;
    offset += 1; // the `acquired`/`ae_looting` bit byte

    let mut items = Vec::with_capacity(item_count as usize);
    for index in 0..item_count {
        offset += 1; // item_type, ui_type and can_trade_to_tap_list, flushed
        let item_id = {
            let bytes = payload
                .get(offset..offset + 4)
                .with_context(|| format!("loot item {index} has no item id"))?;
            i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        };
        offset += 12; // item id, random properties seed and id
        let bonus_byte = *payload
            .get(offset)
            .with_context(|| format!("loot item {index} has no bonus flag"))?;
        offset += 1;
        if bonus_byte & 0x01 != 0 {
            bail!("loot item {index} carries item bonuses, which this mode does not decode");
        }
        let mod_byte = *payload
            .get(offset)
            .with_context(|| format!("loot item {index} has no modification count"))?;
        offset += 1;
        if mod_byte & 0x3F != 0 {
            bail!("loot item {index} carries item modifications, which this mode does not decode");
        }
        let quantity = read_u32(offset)?;
        offset += 4;
        offset += 1; // loot_item_type
        let loot_list_id = *payload
            .get(offset)
            .with_context(|| format!("loot item {index} has no loot list id"))?;
        offset += 1;
        items.push(LootWindowItem {
            loot_list_id,
            item_id,
            quantity,
        });
    }
    if currency_count != 0 {
        bail!(
            "the loot window offered {currency_count} currencies, which this mode does not decode"
        );
    }
    Ok(((loot_obj_low, loot_obj_high), coins, items))
}

fn character_money_and_item_count_like_cpp(character_guid: u64) -> Result<(u64, u32)> {
    use mysql::prelude::Queryable;
    let url = characters_db_url()?;
    let mut conn = mysql::Conn::new(qa_mysql_opts(&url, "characters")?)
        .map_err(|error| anyhow!("Connect to characters DB failed: {error}"))?;
    let money: Option<u64> = conn
        .exec_first(
            "SELECT money FROM characters WHERE guid = ?",
            (character_guid,),
        )
        .map_err(|error| anyhow!("Read money for {character_guid}: {error}"))?;
    let items: Option<u32> = conn
        .exec_first(
            "SELECT COUNT(*) FROM character_inventory WHERE guid = ?",
            (character_guid,),
        )
        .map_err(|error| anyhow!("Count inventory for {character_guid}: {error}"))?;
    Ok((money.unwrap_or(0), items.unwrap_or(0)))
}

/// Open the corpse, take the money and every item, then close the window.
///
/// C++ order: `CMSG_LOOT_UNIT` answers with `SMSG_LOOT_RESPONSE`, then
/// `CMSG_LOOT_MONEY` and one `CMSG_LOOT_ITEM` per entry, and `CMSG_LOOT_RELEASE`
/// closes it. Nothing here is asserted beyond the response arriving: the grants
/// are checked against the database the caller reads after a clean logout.
#[allow(clippy::too_many_arguments)]
async fn loot_the_corpse_like_cpp(
    bot_index: usize,
    character_guid: u64,
    connection: &mut EncryptedWorldConnection,
    target_guid: (u64, u64),
    deadline: std::time::Instant,
    clock_origin: tokio::time::Instant,
    outcome: &mut MeleeSmokeOutcome,
) -> Result<()> {
    let packed_target = build_packed_guid(target_guid.0, target_guid.1);
    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_LOOT_UNIT,
        &packed_target,
    )
    .await?;
    info!("[Bot {}] ✅ CMSG_LOOT_UNIT sent", bot_index);

    let mut window_items: Vec<LootWindowItem> = Vec::new();
    let mut loot_object: Option<(u64, u64)> = None;
    let loot_deadline = deadline.min(std::time::Instant::now() + Duration::from_secs(10));
    while std::time::Instant::now() < loot_deadline && !outcome.loot_response_seen {
        match read_encrypted_packet_if_ready(
            &mut connection.stream,
            &mut connection.crypt,
            &mut connection.inflater,
            Duration::from_millis(500),
            Duration::from_secs(5),
            "loot window",
        )
        .await
        {
            Ok(Some((opcode, payload))) => {
                if opcode == SMSG_TIME_SYNC_REQUEST {
                    respond_to_detour_time_sync_like_cpp(
                        bot_index,
                        &mut connection.stream,
                        &mut connection.crypt,
                        &payload,
                        clock_origin,
                        "loot window",
                    )
                    .await?;
                } else if opcode == SMSG_LOOT_RESPONSE {
                    let (loot_obj, coins, items) = parse_loot_response_like_cpp(&payload)
                        .with_context(|| format!("SMSG_LOOT_RESPONSE ({} bytes)", payload.len()))?;
                    loot_object = Some(loot_obj);
                    outcome.loot_response_seen = true;
                    outcome.loot_coins = coins;
                    outcome.loot_items_offered = items.len() as u32;
                    info!(
                        "[Bot {}] ✅ SMSG_LOOT_RESPONSE coins={} items={:?}",
                        bot_index, coins, items
                    );
                    window_items = items;
                }
            }
            Ok(None) => {}
            Err(error) => bail!("read error while opening the loot: {error}"),
        }
    }
    if !outcome.loot_response_seen {
        bail!("the server never answered CMSG_LOOT_UNIT with SMSG_LOOT_RESPONSE");
    }

    if outcome.loot_coins > 0 {
        // C++ reads one bit (`is_soft_interact`).
        send_encrypted_packet(
            &mut connection.stream,
            &mut connection.crypt,
            CMSG_LOOT_MONEY,
            &[0u8],
        )
        .await?;
        info!("[Bot {}] ✅ CMSG_LOOT_MONEY sent", bot_index);
    }

    let packed_loot_object = loot_object
        .map(|(low, high)| build_packed_guid(low, high))
        .unwrap_or_else(|| packed_target.clone());
    for item in &window_items {
        let mut body = 1u32.to_le_bytes().to_vec();
        body.extend_from_slice(&packed_loot_object);
        body.push(item.loot_list_id);
        body.push(0); // is_soft_interact
        send_encrypted_packet(
            &mut connection.stream,
            &mut connection.crypt,
            CMSG_LOOT_ITEM,
            &body,
        )
        .await?;
        outcome.loot_item_requests += 1;
    }
    if outcome.loot_item_requests > 0 {
        info!(
            "[Bot {}] ✅ {} CMSG_LOOT_ITEM sent",
            bot_index, outcome.loot_item_requests
        );
    }

    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_LOOT_RELEASE,
        &packed_target,
    )
    .await?;
    info!("[Bot {}] ✅ CMSG_LOOT_RELEASE sent", bot_index);

    // Drain briefly so the grants are processed before the caller logs out.
    let settle = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < settle {
        match read_encrypted_packet_if_ready(
            &mut connection.stream,
            &mut connection.crypt,
            &mut connection.inflater,
            Duration::from_millis(300),
            Duration::from_secs(5),
            "loot settle",
        )
        .await
        {
            Ok(Some((opcode, payload))) if opcode == SMSG_TIME_SYNC_REQUEST => {
                respond_to_detour_time_sync_like_cpp(
                    bot_index,
                    &mut connection.stream,
                    &mut connection.crypt,
                    &payload,
                    clock_origin,
                    "loot settle",
                )
                .await?;
            }
            Ok(_) => {}
            Err(error) => bail!("read error while the loot settled: {error}"),
        }
    }
    let _ = character_guid;
    Ok(())
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
        loot_after_kill: cli.loot_after_kill,
    };
    // A creature that kills the QA character leaves it unable to swing, and the
    // rejection is invisible on the wire, so this is checked and reported before
    // the run rather than surfacing as a missing SMSG_ATTACK_START.
    {
        let characters_url = characters_db_url()?;
        let opts = mysql::Opts::from_url(&characters_url)
            .map_err(|error| anyhow!("Bad characters DB URL: {error}"))?;
        let mut conn = mysql::Conn::new(opts)
            .map_err(|error| anyhow!("Connect to characters DB failed: {error}"))?;
        validate_local_bot_character_owner(&mut conn, &bot)?;
        if revive_dead_bot_character_fixture(&mut conn, &bot)? {
            info!(
                "character {} ({}) was dead; restored its stored health as a fixture before the run",
                bot.character_guid, bot.account
            );
        }
    }
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
