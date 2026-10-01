//! Live `CMSG_AREA_TRIGGER` quest-credit check.
//!
//! C++ `WorldSession::HandleAreaTriggerOpcode` credits area-trigger quests only
//! for a living player who is actually inside the trigger
//! (`Handlers/MiscHandler.cpp:497-574`), and the trigger's geometry is client
//! data: `AreaTrigger.db2`, not SQL. So this mode takes the position from the
//! server rather than guessing it — start the world server with
//! `RUSTYCORE_AREA_TRIGGER_TRACE=<id>` and pass the `RUST_AREA_TRIGGER geometry`
//! line's coordinates with `--area-trigger-at`.

use super::*;

/// C++ gives the client no deadline here; this is the harness's own patience for
/// one login, one packet and one clean logout.
pub(crate) const DEFAULT_AREA_TRIGGER_SMOKE_TIMEOUT_SECS: u64 = 120;

/// What the server published, with nothing inferred.
#[derive(Debug, Default, serde::Serialize)]
pub(crate) struct AreaTriggerSmokeOutcome {
    pub revived_by_fixture: bool,
    pub simple_credits: u32,
    pub quest_completes: u32,
    pub objective_data_after_logout: Option<i32>,
    pub explored_after_logout: Option<u8>,
    pub quest_status_after_logout: Option<u8>,
}

fn parse_position_argument(value: &str) -> Result<(f32, f32, f32)> {
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        bail!("--area-trigger-at wants x,y,z (got {value:?})");
    }
    let parse = |index: usize| -> Result<f32> {
        parts[index]
            .parse::<f32>()
            .map_err(|error| anyhow!("--area-trigger-at component {index}: {error}"))
    };
    Ok((parse(0)?, parse(1)?, parse(2)?))
}

pub(crate) async fn run_area_trigger_smoke_mode(
    cli: &CliOptions,
    mut bots: Vec<config::BotConfig>,
) -> Result<()> {
    if bots.len() != 1 {
        bail!(
            "--area-trigger needs exactly one enabled bot; select it with --single (got {})",
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
    let trigger_id = cli
        .area_trigger_id
        .ok_or_else(|| anyhow!("--area-trigger wants the AreaTrigger.db2 id"))?;
    let quest_id = cli
        .area_trigger_quest_id
        .ok_or_else(|| anyhow!("--area-trigger-quest wants the quest id to seed"))?;
    let at = cli
        .area_trigger_at
        .as_deref()
        .ok_or_else(|| anyhow!("--area-trigger-at wants the trigger's x,y,z"))?;
    let position = parse_position_argument(at)?;
    let map_id = cli.area_trigger_map_id;

    let outcome = run_area_trigger_smoke(
        &bot,
        trigger_id,
        quest_id,
        map_id,
        position,
        cli.area_trigger_timeout_secs,
    )
    .await?;
    if let Some(path) = &cli.report_path {
        std::fs::write(path, serde_json::to_string_pretty(&outcome)?)
            .with_context(|| format!("cannot write area-trigger report {path}"))?;
        info!("Area-trigger report written to {path}");
    }
    if outcome.simple_credits == 0 && outcome.quest_completes == 0 {
        bail!("the server published neither a simple credit nor a quest completion");
    }
    Ok(())
}

/// Seed the scenario and return the character's original position so the run can
/// put it back. Both writes are fixtures to columns `Player::SaveToDB` owns; no
/// server path is exercised by them.
fn seed_area_trigger_scenario_like_cpp(
    bot: &config::BotConfig,
    quest_id: u32,
    map_id: u16,
    position: (f32, f32, f32),
) -> Result<(bool, u16, (f32, f32, f32))> {
    use mysql::prelude::Queryable;
    let character_guid = bot.character_guid;
    let url = characters_db_url()?;
    let mut conn = mysql::Conn::new(qa_mysql_opts(&url, "characters")?)
        .map_err(|error| anyhow!("Connect to characters DB failed: {error}"))?;
    let row: Option<(u8, u16, f32, f32, f32)> = conn
        .exec_first(
            "SELECT online, map, position_x, position_y, position_z FROM characters WHERE guid = ?",
            (character_guid,),
        )
        .map_err(|error| anyhow!("Read character position: {error}"))?;
    let (online, original_map, x, y, z) =
        row.ok_or_else(|| anyhow!("No characters row for guid {character_guid}"))?;
    if online != 0 {
        bail!("character {character_guid} is still online; log it out before this mode");
    }
    // C++ gates the whole quest block on `player->IsAlive()`
    // (`Handlers/MiscHandler.cpp:530`), and the refusal is invisible on the wire:
    // a dead character simply produces zero credits, which reads exactly like a
    // broken server. The trigger positions are also where the quest's mobs live,
    // so a previous run can easily end with the character dead there. Restore it
    // first, and report it.
    let revived = revive_dead_bot_character_fixture(&mut conn, bot)?;
    conn.exec_drop(
        "UPDATE characters SET map = ?, position_x = ?, position_y = ?, position_z = ? \
         WHERE guid = ?",
        (map_id, position.0, position.1, position.2, character_guid),
    )
    .map_err(|error| anyhow!("Position fixture for {character_guid}: {error}"))?;
    // The quest has to be in the log and incomplete: C++ only credits a quest
    // whose status is `QUEST_STATUS_INCOMPLETE` (`MiscHandler.cpp:541`).
    conn.exec_drop(
        "INSERT INTO character_queststatus (guid, quest, status, explored, acceptTime, endTime) \
         VALUES (?, ?, 3, 0, UNIX_TIMESTAMP(), 0) \
         ON DUPLICATE KEY UPDATE status = 3, explored = 0",
        (character_guid, quest_id),
    )
    .map_err(|error| anyhow!("Quest fixture for {character_guid}: {error}"))?;
    conn.exec_drop(
        "DELETE FROM character_queststatus_objectives WHERE guid = ? AND quest = ?",
        (character_guid, quest_id),
    )
    .map_err(|error| anyhow!("Objective fixture cleanup for {character_guid}: {error}"))?;
    Ok((revived, original_map, (x, y, z)))
}

fn restore_character_position_like_cpp(
    character_guid: u64,
    map_id: u16,
    position: (f32, f32, f32),
) -> Result<()> {
    use mysql::prelude::Queryable;
    let url = characters_db_url()?;
    let mut conn = mysql::Conn::new(qa_mysql_opts(&url, "characters")?)
        .map_err(|error| anyhow!("Connect to characters DB failed: {error}"))?;
    conn.exec_drop(
        "UPDATE characters SET map = ?, position_x = ?, position_y = ?, position_z = ? \
         WHERE guid = ?",
        (map_id, position.0, position.1, position.2, character_guid),
    )
    .map_err(|error| anyhow!("Position restore for {character_guid}: {error}"))
}

/// C++ `WorldPackets::AreaTrigger::AreaTrigger::Read`: the id, then `Entered` and
/// `FromClient` as two bits.
///
/// Bits are written most-significant first — the server's `WorldPacket::write_bit`
/// sets `1 << (8 - position)` — so `Entered` is bit 7 and `FromClient` bit 6 of
/// the single flushed byte.
fn build_area_trigger_payload(trigger_id: u32, entered: bool) -> Vec<u8> {
    let mut payload = trigger_id.to_le_bytes().to_vec();
    let mut bits = 0b0100_0000u8; // FromClient: this is a client-sent trigger.
    if entered {
        bits |= 0b1000_0000;
    }
    payload.push(bits);
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_trigger_payload_puts_entered_in_the_high_bit() {
        assert_eq!(
            build_area_trigger_payload(87, true),
            vec![87, 0, 0, 0, 0b1100_0000]
        );
        assert_eq!(
            build_area_trigger_payload(87, false),
            vec![87, 0, 0, 0, 0b0100_0000]
        );
    }

    #[test]
    fn position_argument_wants_three_components() {
        assert_eq!(
            parse_position_argument(" -9100.5 , 40.25 , 83.0 ").unwrap(),
            (-9100.5, 40.25, 83.0)
        );
        assert!(parse_position_argument("1,2").is_err());
        assert!(parse_position_argument("1,2,x").is_err());
    }
}

async fn run_area_trigger_smoke(
    bot: &config::BotConfig,
    trigger_id: u32,
    quest_id: u32,
    map_id: u16,
    position: (f32, f32, f32),
    timeout_secs: u64,
) -> Result<AreaTriggerSmokeOutcome> {
    let bot_index = bot.account_id as usize;
    let character_guid = bot.character_guid;
    let mut outcome = AreaTriggerSmokeOutcome::default();

    let fixture_bot = bot.clone();
    let (revived, original_map, original_position) = tokio::task::spawn_blocking(move || {
        seed_area_trigger_scenario_like_cpp(&fixture_bot, quest_id, map_id, position)
    })
    .await
    .map_err(|error| anyhow!("Area-trigger fixture worker failed: {error}"))??;
    outcome.revived_by_fixture = revived;
    info!(
        "[Bot {}] fixture: quest {} incomplete, character {} at map {} ({:.1}, {:.1}, {:.1}){}",
        bot_index,
        quest_id,
        character_guid,
        map_id,
        position.0,
        position.1,
        position.2,
        if revived {
            ", restored from a death a previous run left behind"
        } else {
            ""
        }
    );

    let result =
        drive_area_trigger_scenario_like_cpp(bot, trigger_id, quest_id, timeout_secs, &mut outcome)
            .await;

    // Put the character back wherever it was, pass or fail.
    if let Err(error) = tokio::task::spawn_blocking(move || {
        restore_character_position_like_cpp(character_guid, original_map, original_position)
    })
    .await
    .map_err(|error| anyhow!("Position restore worker failed: {error}"))?
    {
        warn!(
            "[Bot {}] could not restore the position: {error}",
            bot_index
        );
    }
    result?;

    info!(
        "[Bot {}] area-trigger summary: revived={} simple_credits={} quest_completes={} \
         objective_data={:?} explored={:?} status={:?}",
        bot_index,
        outcome.revived_by_fixture,
        outcome.simple_credits,
        outcome.quest_completes,
        outcome.objective_data_after_logout,
        outcome.explored_after_logout,
        outcome.quest_status_after_logout
    );
    Ok(outcome)
}

async fn drive_area_trigger_scenario_like_cpp(
    bot: &config::BotConfig,
    trigger_id: u32,
    quest_id: u32,
    timeout_secs: u64,
    outcome: &mut AreaTriggerSmokeOutcome,
) -> Result<()> {
    let bot_index = bot.account_id as usize;
    let character_guid = bot.character_guid;

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
    // Without this the world looks empty to the player: C++ `Player::CanNeverSee`
    // hides every object until the client sets its active mover.
    let active_mover_complete = build_move_init_active_mover_complete_payload(0);
    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_MOVE_INIT_ACTIVE_MOVER_COMPLETE,
        &active_mover_complete,
    )
    .await?;
    info!(
        "[Bot {}] ✅ in the world inside trigger {}",
        bot_index, trigger_id
    );

    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_AREA_TRIGGER,
        &build_area_trigger_payload(trigger_id, true),
    )
    .await?;
    info!(
        "[Bot {}] ✅ CMSG_AREA_TRIGGER {} sent",
        bot_index, trigger_id
    );

    // Both sockets: the quest packets are realm-connection opcodes in C++, like
    // `SMSG_LOG_XP_GAIN` is, and reading only the instance socket reported zero
    // for credits the server had already granted.
    let listen_until = deadline.min(std::time::Instant::now() + Duration::from_secs(10));
    while std::time::Instant::now() < listen_until {
        let mut saw_any = false;
        for route in [false, true] {
            let target = if route {
                match realm_connection.as_mut() {
                    Some(realm) => realm,
                    None => continue,
                }
            } else {
                &mut connection
            };
            match read_encrypted_packet_if_ready(
                &mut target.stream,
                &mut target.crypt,
                &mut target.inflater,
                Duration::from_millis(200),
                Duration::from_secs(5),
                "area trigger",
            )
            .await
            {
                Ok(Some((opcode, payload))) => {
                    saw_any = true;
                    if opcode == SMSG_TIME_SYNC_REQUEST {
                        let (stream, crypt) = (&mut target.stream, &mut target.crypt);
                        respond_to_detour_time_sync_like_cpp(
                            bot_index,
                            stream,
                            crypt,
                            &payload,
                            clock_origin,
                            "area trigger",
                        )
                        .await?;
                    } else if opcode == SMSG_QUEST_UPDATE_ADD_CREDIT_SIMPLE {
                        outcome.simple_credits += 1;
                        info!(
                            "[Bot {}] ✅ SMSG_QUEST_UPDATE_ADD_CREDIT_SIMPLE ({} bytes)",
                            bot_index,
                            payload.len()
                        );
                    } else if opcode == SMSG_QUEST_UPDATE_COMPLETE {
                        outcome.quest_completes += 1;
                        info!("[Bot {}] ✅ SMSG_QUEST_UPDATE_COMPLETE", bot_index);
                    }
                }
                Ok(None) => {}
                Err(error) => return Err(error),
            }
        }
        if !saw_any {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    // The objective and the explored flag are persisted by the logout save, so
    // read them back from the columns `Player::SaveToDB` writes.
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

    let persisted =
        tokio::task::spawn_blocking(move || -> Result<(Option<i32>, Option<(u8, u8)>)> {
            use mysql::prelude::Queryable;
            let url = characters_db_url()?;
            let mut conn = mysql::Conn::new(qa_mysql_opts(&url, "characters")?)
                .map_err(|error| anyhow!("Connect to characters DB failed: {error}"))?;
            let objective: Option<i32> = conn
            .exec_first(
                "SELECT data FROM character_queststatus_objectives WHERE guid = ? AND quest = ? \
                 ORDER BY objective LIMIT 1",
                (character_guid, quest_id),
            )
            .map_err(|error| anyhow!("Read objective row: {error}"))?;
            let status: Option<(u8, u8)> = conn
            .exec_first(
                "SELECT status, explored FROM character_queststatus WHERE guid = ? AND quest = ?",
                (character_guid, quest_id),
            )
            .map_err(|error| anyhow!("Read quest status row: {error}"))?;
            Ok((objective, status))
        })
        .await
        .map_err(|error| anyhow!("Persisted-state worker failed: {error}"))??;
    outcome.objective_data_after_logout = persisted.0;
    if let Some((status, explored)) = persisted.1 {
        outcome.quest_status_after_logout = Some(status);
        outcome.explored_after_logout = Some(explored);
    }
    Ok(())
}
