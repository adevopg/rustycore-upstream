// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! The death exit, end to end: release the spirit, run back, reclaim the corpse.
//!
//! The scenario is C++ `WorldSession::HandleRepopRequest` →
//! `Player::BuildPlayerRepop` + `Player::RepopAtGraveyard`
//! (`Handlers/MiscHandler.cpp:60-85`), then
//! `WorldSession::HandleReclaimCorpse` (`:435-464`).
//!
//! The acceptance signal is the `corpse` row, not a decoded update block. C++
//! `Corpse::SaveToDB` writes it inside `CreateCorpse` and
//! `Map::ConvertCorpseToBones` deletes it inside the reclaim
//! (`Maps/Map.cpp:3748-3750`), both committed immediately rather than at the next
//! player save, so the row appearing and then disappearing is exactly the
//! server's own record of the two transitions.

use super::*;

/// C++ `PLAYER_FLAGS_GHOST` (`Entities/Unit/UnitDefines.h`), persisted in
/// `characters.playerFlags`.
const PLAYER_FLAGS_GHOST_LIKE_CPP: u32 = 0x0000_0010;

/// The corpse run plus the C++ 30-second reclaim delay need a wider budget than
/// a melee pass.
pub(crate) const DEFAULT_DEATH_SMOKE_TIMEOUT_SECS: u64 = 180;

/// SMSG_MOVE_TELEPORT.
const SMSG_MOVE_TELEPORT: u16 = 0x2E04;
/// CMSG_MOVE_TELEPORT_ACK.
const CMSG_MOVE_TELEPORT_ACK: u16 = 0x39FA;
/// CMSG_REPOP_REQUEST.
const CMSG_REPOP_REQUEST: u16 = 0x3526;
/// CMSG_RECLAIM_CORPSE.
const CMSG_RECLAIM_CORPSE: u16 = 0x34DB;

/// How often the mode retries the reclaim while the C++ delay is still running.
const DEATH_SMOKE_RECLAIM_RETRY: Duration = Duration::from_secs(5);

#[derive(Debug, Default, Clone, serde::Serialize)]
pub(crate) struct DeathSmokeOutcome {
    pub killed_by_fixture: bool,
    pub corpse_row_after_release: bool,
    pub graveyard_teleport_seen: bool,
    pub graveyard_position: Option<(f32, f32, f32)>,
    pub walk_steps: u32,
    pub reclaim_requests: u32,
    pub corpse_row_after_reclaim: bool,
    pub health_after_logout: Option<u32>,
}

/// Where the server itself put the corpse, from the row `Corpse::SaveToDB` wrote.
/// Walking to this instead of to a remembered player position means the reclaim
/// radius is compared against the same authority on both sides.
fn corpse_row_position_like_cpp(owner_guid: u64) -> Result<Option<(f32, f32, f32)>> {
    use mysql::prelude::Queryable;
    let url = characters_db_url()?;
    let mut conn = mysql::Conn::new(qa_mysql_opts(&url, "characters")?)
        .map_err(|error| anyhow!("Connect to characters DB failed: {error}"))?;
    let row: Option<(f32, f32, f32)> = conn
        .exec_first(
            "SELECT posX, posY, posZ FROM corpse WHERE guid = ?",
            (owner_guid,),
        )
        .map_err(|error| anyhow!("Read corpse position for {owner_guid}: {error}"))?;
    Ok(row)
}

fn corpse_row_count_like_cpp(owner_guid: u64) -> Result<u32> {
    use mysql::prelude::Queryable;
    let url = characters_db_url()?;
    let mut conn = mysql::Conn::new(qa_mysql_opts(&url, "characters")?)
        .map_err(|error| anyhow!("Connect to characters DB failed: {error}"))?;
    let count: Option<u32> = conn
        .exec_first("SELECT COUNT(*) FROM corpse WHERE guid = ?", (owner_guid,))
        .map_err(|error| anyhow!("Count corpse rows for {owner_guid}: {error}"))?;
    Ok(count.unwrap_or(0))
}

/// C++ `MoveTeleport::Write` (`Server/Packets/MovementPackets.cpp`): the mover
/// guid as sixteen raw bytes, the sequence index, then the destination.
fn parse_move_teleport_like_cpp(payload: &[u8]) -> Option<(u32, (f32, f32, f32), f32)> {
    let sequence = payload.get(16..20)?;
    let sequence = u32::from_le_bytes([sequence[0], sequence[1], sequence[2], sequence[3]]);
    let floats = payload.get(20..36)?;
    let read = |index: usize| {
        f32::from_le_bytes([
            floats[index],
            floats[index + 1],
            floats[index + 2],
            floats[index + 3],
        ])
    };
    Some((sequence, (read(0), read(4), read(8)), read(12)))
}

/// C++ `CMSG_MOVE_TELEPORT_ACK`: the packed mover guid, the ack index the server
/// sent as the teleport's sequence index, and a client move time.
fn build_move_teleport_ack_payload(
    player_low: u64,
    player_high: u64,
    ack_index: u32,
    move_time: u32,
) -> Vec<u8> {
    let mut payload = build_packed_guid(player_low, player_high);
    payload.extend_from_slice(&ack_index.to_le_bytes());
    payload.extend_from_slice(&move_time.to_le_bytes());
    payload
}

pub(crate) async fn run_death_smoke_mode(
    cli: &CliOptions,
    mut bots: Vec<config::BotConfig>,
) -> Result<()> {
    if bots.len() != 1 {
        bail!(
            "--death-smoke needs exactly one enabled bot; select it with --single (got {})",
            bots.len()
        );
    }
    let bot = bots.remove(0);
    if bot.password.trim().is_empty() {
        bail!(
            "No password for {}; export {}",
            bot.account,
            password_env_name(&bot.account)
        );
    }
    let outcome = run_death_smoke(&bot, cli.death_smoke_timeout_secs).await?;
    if let Some(path) = &cli.report_path {
        std::fs::write(path, serde_json::to_string_pretty(&outcome)?)
            .with_context(|| format!("cannot write death report {path}"))?;
        info!("Death report written to {path}");
    }
    Ok(())
}

async fn run_death_smoke(bot: &config::BotConfig, timeout_secs: u64) -> Result<DeathSmokeOutcome> {
    let bot_index = bot.account_id as usize;
    let mut outcome = DeathSmokeOutcome::default();
    let character_guid = bot.character_guid;

    // The scenario needs a dead character, which is what a creature leaves
    // behind. Making it dead here is a fixture write to the column C++
    // `Player::LoadFromDB` reads as the death state (`Player.cpp:18119-18121`),
    // the exact mirror of the revive fixture `--melee-smoke` uses; it exercises
    // no server death path and is reported either way.
    let (already_dead, start_position) = tokio::task::spawn_blocking(move || -> Result<(bool, (f32, f32, f32))> {
        use mysql::prelude::Queryable;
        let url = characters_db_url()?;
        let mut conn = mysql::Conn::new(qa_mysql_opts(&url, "characters")?)
            .map_err(|error| anyhow!("Connect to characters DB failed: {error}"))?;
        let row: Option<(u32, u8, f32, f32, f32)> = conn
            .exec_first(
                "SELECT health, online, position_x, position_y, position_z FROM characters WHERE guid = ?",
                (character_guid,),
            )
            .map_err(|error| anyhow!("Read character state: {error}"))?;
        let (health, online, x, y, z) =
            row.ok_or_else(|| anyhow!("No characters row for guid {character_guid}"))?;
        if online != 0 {
            bail!("character {character_guid} is still online; log it out before this mode");
        }
        // A fresh death is a corpse, not a ghost: C++ `HandleRepopRequest`
        // (`Handlers/MiscHandler.cpp:62-63`) refuses to release a spirit that
        // already carries `PLAYER_FLAGS_GHOST`, and that flag is persisted in
        // `characters.playerFlags`, so a character left as a ghost by an earlier
        // run has to be reset with the health.
        conn.exec_drop(
            "UPDATE characters SET health = 0, playerFlags = playerFlags & ~? WHERE guid = ?",
            (PLAYER_FLAGS_GHOST_LIKE_CPP, character_guid),
        )
        .map_err(|error| anyhow!("Death fixture for {character_guid}: {error}"))?;
        // An interrupted run leaves the corpse row behind, and the scenario has to
        // start from a clean death. These are the same three owner-keyed deletes
        // C++ `Corpse::DeleteFromDB` issues, run as a fixture.
        for sql in [
            "DELETE FROM corpse WHERE guid = ?",
            "DELETE FROM corpse_phases WHERE OwnerGuid = ?",
            "DELETE FROM corpse_customizations WHERE OwnerGuid = ?",
        ] {
            conn.exec_drop(sql, (character_guid,))
                .map_err(|error| anyhow!("Corpse fixture cleanup for {character_guid}: {error}"))?;
        }
        Ok((health == 0, (x, y, z)))
    })
    .await
    .map_err(|error| anyhow!("Death fixture worker failed: {error}"))??;
    outcome.killed_by_fixture = !already_dead;
    info!(
        "[Bot {}] character {} starts dead ({}) at ({:.1}, {:.1}, {:.1})",
        bot_index,
        character_guid,
        if outcome.killed_by_fixture {
            "killed by the fixture"
        } else {
            "already dead"
        },
        start_position.0,
        start_position.1,
        start_position.2
    );

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
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
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
    let login_body = build_player_login(character_guid, realm_id(), 500.0);
    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_PLAYER_LOGIN,
        &login_body,
    )
    .await?;
    let mut realm_connection: Option<EncryptedWorldConnection> = None;
    expect_login_opcode_across_connect_to(
        bot_index,
        &mut connection,
        &mut realm_connection,
        &authenticated.derived_session_key,
        SMSG_LOGIN_VERIFY_WORLD,
        deadline,
        "SMSG_LOGIN_VERIFY_WORLD",
        None,
    )
    .await?;
    let active_mover_complete = build_move_init_active_mover_complete_payload(0);
    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_MOVE_INIT_ACTIVE_MOVER_COMPLETE,
        &active_mover_complete,
    )
    .await?;
    info!("[Bot {}] ✅ in the world as a corpse", bot_index);

    let (player_low, player_high) = create_player_guid_raw(character_guid, realm_id());
    let owner_guid = character_guid;

    // C++ `HandleRepopRequest` reads one bit and ignores its value for the
    // represented path.
    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_REPOP_REQUEST,
        &[0u8],
    )
    .await?;
    info!("[Bot {}] ✅ CMSG_REPOP_REQUEST sent", bot_index);

    // The release creates the corpse where the player died and then teleports the
    // ghost to its graveyard. Follow the teleport the way a client does.
    let mut position = start_position;
    let release_deadline = deadline.min(std::time::Instant::now() + Duration::from_secs(20));
    while std::time::Instant::now() < release_deadline && !outcome.graveyard_teleport_seen {
        // `read_encrypted_packet_if_ready` peeks before it reads. Wrapping the
        // frame read itself in a timeout cancels it mid-frame and desynchronizes
        // the stream, which surfaces as "Invalid encrypted packet size".
        match read_encrypted_packet_if_ready(
            &mut connection.stream,
            &mut connection.crypt,
            &mut connection.inflater,
            Duration::from_millis(500),
            Duration::from_secs(5),
            "death release",
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
                        "death release",
                    )
                    .await?;
                } else if opcode == SMSG_MOVE_TELEPORT {
                    let Some((sequence, destination, _facing)) =
                        parse_move_teleport_like_cpp(&payload)
                    else {
                        bail!("malformed SMSG_MOVE_TELEPORT ({} bytes)", payload.len());
                    };
                    let ack = build_move_teleport_ack_payload(
                        player_low,
                        player_high,
                        sequence,
                        clock_origin.elapsed().as_millis() as u32,
                    );
                    send_encrypted_packet(
                        &mut connection.stream,
                        &mut connection.crypt,
                        CMSG_MOVE_TELEPORT_ACK,
                        &ack,
                    )
                    .await?;
                    outcome.graveyard_teleport_seen = true;
                    outcome.graveyard_position = Some(destination);
                    position = destination;
                    info!(
                        "[Bot {}] ✅ SMSG_MOVE_TELEPORT to the graveyard at ({:.1}, {:.1}, {:.1}); acknowledged",
                        bot_index, destination.0, destination.1, destination.2
                    );
                }
            }
            Ok(None) => {}
            Err(error) => bail!("read error after the spirit release: {error}"),
        }
    }

    outcome.corpse_row_after_release = corpse_row_count_like_cpp(owner_guid)? != 0;
    if !outcome.corpse_row_after_release {
        bail!(
            "releasing the spirit left no corpse row: C++ `Player::CreateCorpse` saves one \
             (`Entities/Player/Player.cpp:4407-4408`) and the reclaim needs it"
        );
    }
    let corpse_position = corpse_row_position_like_cpp(owner_guid)?
        .ok_or_else(|| anyhow!("the corpse row disappeared between two reads"))?;
    info!(
        "[Bot {}] ✅ the release wrote a corpse row at ({:.1}, {:.1}, {:.1})",
        bot_index, corpse_position.0, corpse_position.1, corpse_position.2
    );

    // Run back. The reclaim radius is 39 yards (`Entities/Corpse/Corpse.h:37`),
    // and this stops well inside it.
    let mut steps = 0u32;
    let mut last_step = std::time::Instant::now() - MELEE_SMOKE_STEP_INTERVAL;
    let mut last_reclaim = std::time::Instant::now() - DEATH_SMOKE_RECLAIM_RETRY;
    while std::time::Instant::now() < deadline {
        match read_encrypted_packet_if_ready(
            &mut connection.stream,
            &mut connection.crypt,
            &mut connection.inflater,
            Duration::from_millis(200),
            Duration::from_secs(5),
            "corpse run",
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
                        "corpse run",
                    )
                    .await?;
                }
            }
            Ok(None) => {}
            Err(error) => bail!("read error during the corpse run: {error}"),
        }

        if last_step.elapsed() >= MELEE_SMOKE_STEP_INTERVAL {
            // A real client keeps publishing heartbeats after it stops walking.
            // Stopping at the last step means one dropped heartbeat leaves the
            // server's idea of the position behind for good, and the reclaim
            // radius is then refused forever with nothing to correct it.
            let (next, facing) =
                next_walk_step_like_cpp(position, corpse_position, 5.0).unwrap_or((
                    position,
                    (corpse_position.1 - position.1).atan2(corpse_position.0 - position.0),
                ));
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
            if next != position {
                steps += 1;
                outcome.walk_steps = steps;
            }
            position = next;
            last_step = std::time::Instant::now();
        }

        // Only ask once the run is over: the distance gate would refuse anyway,
        // and the delay gate is what the retries are waiting out.
        let home = distance_between(position, corpse_position) <= 6.0;
        if home && last_reclaim.elapsed() >= DEATH_SMOKE_RECLAIM_RETRY {
            // C++ reads a guid here and then ignores it: the handler resolves the
            // corpse through `Player::GetCorpse`, so the only valid request is
            // "reclaim mine".
            let mut body = vec![0u8; 16];
            body[..8].copy_from_slice(&player_low.to_le_bytes());
            body[8..].copy_from_slice(&player_high.to_le_bytes());
            send_encrypted_packet(
                &mut connection.stream,
                &mut connection.crypt,
                CMSG_RECLAIM_CORPSE,
                &body,
            )
            .await?;
            outcome.reclaim_requests += 1;
            last_reclaim = std::time::Instant::now();
            // The row is dropped by the server's own transaction, so give it a
            // moment before reading: counting immediately attributes a success to
            // the next request instead of this one.
            tokio::time::sleep(Duration::from_millis(750)).await;
            if corpse_row_count_like_cpp(owner_guid)? == 0 {
                info!(
                    "[Bot {}] ✅ the reclaim took on request {}: the corpse row is gone",
                    bot_index, outcome.reclaim_requests
                );
                break;
            }
            info!(
                "[Bot {}] reclaim {} refused so far; the corpse row is still there",
                bot_index, outcome.reclaim_requests
            );
        }
    }
    outcome.corpse_row_after_reclaim = corpse_row_count_like_cpp(owner_guid)? != 0;

    // The corpse row is the server's record of the transition, but it does not by
    // itself say the player came back. C++ `:460` resurrects at half health and
    // the ghost flag is cleared, both of which a clean logout persists, so read
    // them back from the columns `Player::SaveToDB` writes.
    if !outcome.corpse_row_after_reclaim {
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
        let (health, player_flags) = tokio::task::spawn_blocking(move || -> Result<(u32, u32)> {
            use mysql::prelude::Queryable;
            let url = characters_db_url()?;
            let mut conn = mysql::Conn::new(qa_mysql_opts(&url, "characters")?)
                .map_err(|error| anyhow!("Connect to characters DB failed: {error}"))?;
            let row: Option<(u32, u32)> = conn
                .exec_first(
                    "SELECT health, playerFlags FROM characters WHERE guid = ?",
                    (character_guid,),
                )
                .map_err(|error| anyhow!("Read resurrected state: {error}"))?;
            row.ok_or_else(|| anyhow!("No characters row for guid {character_guid}"))
        })
        .await
        .map_err(|error| anyhow!("Resurrected-state worker failed: {error}"))??;
        outcome.health_after_logout = Some(health);
        info!(
            "[Bot {}] ✅ logged out: health={} playerFlags=0x{:X}",
            bot_index, health, player_flags
        );
        if health == 0 {
            bail!("the corpse was retired but the character saved with zero health");
        }
        if player_flags & PLAYER_FLAGS_GHOST_LIKE_CPP != 0 {
            bail!("the corpse was retired but the character saved as a ghost");
        }
    }

    info!(
        "[Bot {}] death summary: fixture_kill={} corpse_after_release={} teleport={} steps={} \
         reclaims={} corpse_after_reclaim={} health_after_logout={:?}",
        bot_index,
        outcome.killed_by_fixture,
        outcome.corpse_row_after_release,
        outcome.graveyard_teleport_seen,
        outcome.walk_steps,
        outcome.reclaim_requests,
        outcome.corpse_row_after_reclaim,
        outcome.health_after_logout
    );

    if outcome.corpse_row_after_reclaim {
        bail!(
            "the corpse survived {} reclaim requests: C++ `HandleReclaimCorpse` ends in \
             `SpawnCorpseBones`, which deletes the row (`Maps/Map.cpp:3748-3750`)",
            outcome.reclaim_requests
        );
    }
    Ok(outcome)
}
