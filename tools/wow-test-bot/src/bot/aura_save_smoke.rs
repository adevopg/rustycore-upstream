//! Live `Player::_SaveAuras` check: do a Player's auras survive a logout?
//!
//! C++ `Player::SaveToDB` reaches `_SaveAuras` (`Player.cpp:19948`), which
//! clears both aura tables and rewrites them from the live `m_ownedAuras`
//! (`:20089-20146`). Observing only that a seeded row is still there after a
//! logout proves nothing — it also looks like that when nothing was written at
//! all. So the fixture seeds two rows and the assertions are about the
//! difference between them:
//!
//! * one aura for a real spell, with deliberately wrong stored values. The save
//!   must rewrite them from the live aura.
//! * one aura for a spell id no store knows. `_LoadAuras` drops it, so it is not
//!   in the live aura map and the save must not write it back.
//!
//! Without a write side the first keeps its wrong values and the second survives.

use super::*;

/// The harness's own patience for two logins, one logout and the row read-back.
pub(crate) const DEFAULT_AURA_SAVE_SMOKE_TIMEOUT_SECS: u64 = 180;

/// A spell id no `Spell.db2` row can carry, so `_LoadAuras` is certain to drop
/// it: C++ skips a stored aura whose `SpellInfo` is missing.
const UNKNOWN_SPELL_ID: u32 = 90_000_001;

/// Values the save cannot reproduce, so finding them afterwards means the write
/// never happened.
const SEEDED_BASE_AMOUNT: i32 = 777_777;
const SEEDED_REMAIN_CHARGES: u8 = 5;
const SEEDED_MAX_DURATION_MS: i32 = 120_000;
const SEEDED_REMAIN_TIME_MS: i32 = 90_000;

/// One `character_aura` row as read back, with nothing inferred.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub(crate) struct PersistedAuraRow {
    pub spell: u32,
    pub effect_mask: u32,
    pub recalculate_mask: u32,
    pub stack_count: u8,
    pub max_duration_ms: i32,
    pub remain_time_ms: i32,
    pub remain_charges: u8,
    pub caster_guid_len: usize,
    pub effects: Vec<(u8, i32, i32)>,
}

#[derive(Debug, Default, serde::Serialize)]
pub(crate) struct AuraSaveSmokeOutcome {
    pub revived_by_fixture: bool,
    pub seeded_spell: u32,
    pub aura_updates_first_login: u32,
    pub aura_updates_after_relog: u32,
    pub rows_after_logout: Vec<PersistedAuraRow>,
    pub unknown_spell_row_survived: bool,
}

pub(crate) async fn run_aura_save_smoke_mode(
    cli: &CliOptions,
    mut bots: Vec<config::BotConfig>,
) -> Result<()> {
    if bots.len() != 1 {
        bail!(
            "--aura-save needs exactly one enabled bot; select it with --single (got {})",
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
    let spell_id = cli
        .aura_save_spell_id
        .ok_or_else(|| anyhow!("--aura-save wants the spell id to seed"))?;

    let outcome = run_aura_save_smoke(&bot, spell_id, cli.aura_save_timeout_secs).await?;
    if let Some(path) = &cli.report_path {
        std::fs::write(path, serde_json::to_string_pretty(&outcome)?)
            .with_context(|| format!("cannot write aura-save report {path}"))?;
        info!("Aura-save report written to {path}");
    }

    if outcome.unknown_spell_row_survived {
        bail!(
            "the row for unknown spell {UNKNOWN_SPELL_ID} survived the logout, so _SaveAuras \
             never cleared the table"
        );
    }
    let saved = outcome
        .rows_after_logout
        .iter()
        .find(|row| row.spell == spell_id)
        .ok_or_else(|| anyhow!("spell {spell_id} is not in character_aura after the logout"))?;
    if saved.remain_charges == SEEDED_REMAIN_CHARGES {
        bail!("the stored remainCharges is still the seeded value; the row was not rewritten");
    }
    if saved
        .effects
        .iter()
        .any(|(_, _, base_amount)| *base_amount == SEEDED_BASE_AMOUNT)
    {
        bail!("an effect row still carries the seeded baseAmount; the row was not rewritten");
    }
    if outcome.aura_updates_after_relog == 0 {
        bail!("the relog published no SMSG_AURA_UPDATE, so the saved aura did not come back");
    }
    Ok(())
}

/// Seed both aura rows. These are writes to columns `Player::_SaveAuras` owns; no
/// server path is exercised by them.
fn seed_aura_rows_like_cpp(bot: &config::BotConfig, spell_id: u32) -> Result<bool> {
    use mysql::prelude::Queryable;
    let character_guid = bot.character_guid;
    let url = characters_db_url()?;
    let mut conn = mysql::Conn::new(qa_mysql_opts(&url, "characters")?)
        .map_err(|error| anyhow!("Connect to characters DB failed: {error}"))?;
    let online: Option<u8> = conn
        .exec_first(
            "SELECT online FROM characters WHERE guid = ?",
            (character_guid,),
        )
        .map_err(|error| anyhow!("Read character online flag: {error}"))?;
    match online {
        None => bail!("No characters row for guid {character_guid}"),
        Some(flag) if flag != 0 => {
            bail!("character {character_guid} is still online; log it out before this mode")
        }
        Some(_) => {}
    }
    // A dead character is not what this mode is about, and a previous run can
    // leave one behind; the same fixture the other live modes use.
    let revived = revive_dead_bot_character_fixture(&mut conn, bot)?;

    conn.exec_drop(
        "DELETE FROM character_aura WHERE guid = ?",
        (character_guid,),
    )
    .map_err(|error| anyhow!("Aura fixture cleanup: {error}"))?;
    conn.exec_drop(
        "DELETE FROM character_aura_effect WHERE guid = ?",
        (character_guid,),
    )
    .map_err(|error| anyhow!("Aura effect fixture cleanup: {error}"))?;

    // The caster is the character itself, written the way C++ writes it:
    // `setBinary(ObjectGuid::GetRawValue())`. A player GUID's raw value is the
    // low 64 bits with the high word carrying the type and realm, which the
    // server's own encoding owns — so seed the low counter and let the loader
    // resolve an all-zero high word to "no caster", which C++ `_LoadAuras`
    // treats as the player itself.
    let empty_guid = vec![0u8; 16];
    for (spell, effect_mask) in [(spell_id, 1u32), (UNKNOWN_SPELL_ID, 1u32)] {
        conn.exec_drop(
            "INSERT INTO character_aura (guid, casterGuid, itemGuid, spell, effectMask, \
             recalculateMask, difficulty, stackCount, maxDuration, remainTime, remainCharges, \
             castItemId, castItemLevel) \
             VALUES (?, ?, ?, ?, ?, 0, 0, 1, ?, ?, ?, 0, 0)",
            (
                character_guid,
                &empty_guid,
                &empty_guid,
                spell,
                effect_mask,
                SEEDED_MAX_DURATION_MS,
                SEEDED_REMAIN_TIME_MS,
                SEEDED_REMAIN_CHARGES,
            ),
        )
        .map_err(|error| anyhow!("Aura fixture for spell {spell}: {error}"))?;
        conn.exec_drop(
            "INSERT INTO character_aura_effect (guid, casterGuid, itemGuid, spell, effectMask, \
             effectIndex, amount, baseAmount) VALUES (?, ?, ?, ?, ?, 0, 11, ?)",
            (
                character_guid,
                &empty_guid,
                &empty_guid,
                spell,
                effect_mask,
                SEEDED_BASE_AMOUNT,
            ),
        )
        .map_err(|error| anyhow!("Aura effect fixture for spell {spell}: {error}"))?;
    }
    Ok(revived)
}

fn read_persisted_aura_rows(character_guid: u64) -> Result<Vec<PersistedAuraRow>> {
    use mysql::prelude::Queryable;
    let url = characters_db_url()?;
    let mut conn = mysql::Conn::new(qa_mysql_opts(&url, "characters")?)
        .map_err(|error| anyhow!("Connect to characters DB failed: {error}"))?;
    let rows: Vec<(u32, u32, u32, u8, i32, i32, u8, Vec<u8>)> = conn
        .exec(
            "SELECT spell, effectMask, recalculateMask, stackCount, maxDuration, remainTime, \
             remainCharges, casterGuid FROM character_aura WHERE guid = ? ORDER BY spell",
            (character_guid,),
        )
        .map_err(|error| anyhow!("Read character_aura: {error}"))?;
    let mut out = Vec::new();
    for (
        spell,
        effect_mask,
        recalculate_mask,
        stack_count,
        max_duration,
        remain_time,
        charges,
        caster,
    ) in rows
    {
        let effects: Vec<(u8, i32, i32)> = conn
            .exec(
                "SELECT effectIndex, amount, baseAmount FROM character_aura_effect \
                 WHERE guid = ? AND spell = ? ORDER BY effectIndex",
                (character_guid, spell),
            )
            .map_err(|error| anyhow!("Read character_aura_effect: {error}"))?;
        out.push(PersistedAuraRow {
            spell,
            effect_mask,
            recalculate_mask,
            stack_count,
            max_duration_ms: max_duration,
            remain_time_ms: remain_time,
            remain_charges: charges,
            caster_guid_len: caster.len(),
            effects,
        });
    }
    Ok(out)
}

async fn run_aura_save_smoke(
    bot: &config::BotConfig,
    spell_id: u32,
    timeout_secs: u64,
) -> Result<AuraSaveSmokeOutcome> {
    let bot_index = bot.account_id as usize;
    let character_guid = bot.character_guid;
    let mut outcome = AuraSaveSmokeOutcome {
        seeded_spell: spell_id,
        ..Default::default()
    };

    let fixture_bot = bot.clone();
    outcome.revived_by_fixture =
        tokio::task::spawn_blocking(move || seed_aura_rows_like_cpp(&fixture_bot, spell_id))
            .await
            .map_err(|error| anyhow!("Aura fixture worker failed: {error}"))??;
    info!(
        "[Bot {}] fixture: character {} seeded with auras {} and {} (unknown){}",
        bot_index,
        character_guid,
        spell_id,
        UNKNOWN_SPELL_ID,
        if outcome.revived_by_fixture {
            ", restored from a death a previous run left behind"
        } else {
            ""
        }
    );

    // First session: the load installs the seeded aura, the logout saves it.
    outcome.aura_updates_first_login =
        login_and_logout_counting_aura_updates(bot, timeout_secs, "first login").await?;
    outcome.rows_after_logout =
        tokio::task::spawn_blocking(move || read_persisted_aura_rows(character_guid))
            .await
            .map_err(|error| anyhow!("Aura read-back worker failed: {error}"))??;
    outcome.unknown_spell_row_survived = outcome
        .rows_after_logout
        .iter()
        .any(|row| row.spell == UNKNOWN_SPELL_ID);

    // Second session: what the save wrote has to come back as a live aura.
    outcome.aura_updates_after_relog =
        login_and_logout_counting_aura_updates(bot, timeout_secs, "relog").await?;

    info!(
        "[Bot {}] aura-save summary: revived={} first_login_aura_updates={} \
         rows_after_logout={} unknown_row_survived={} relog_aura_updates={}",
        bot_index,
        outcome.revived_by_fixture,
        outcome.aura_updates_first_login,
        outcome.rows_after_logout.len(),
        outcome.unknown_spell_row_survived,
        outcome.aura_updates_after_relog
    );
    for row in &outcome.rows_after_logout {
        info!(
            "[Bot {}] persisted aura: spell={} effectMask={} recalculateMask={} stack={} \
             maxDuration={} remainTime={} charges={} casterGuidBytes={} effects={:?}",
            bot_index,
            row.spell,
            row.effect_mask,
            row.recalculate_mask,
            row.stack_count,
            row.max_duration_ms,
            row.remain_time_ms,
            row.remain_charges,
            row.caster_guid_len,
            row.effects
        );
    }
    Ok(outcome)
}

/// One complete session: log in, count the `SMSG_AURA_UPDATE` packets the server
/// sends for the initial aura state, then log out cleanly so the save runs.
async fn login_and_logout_counting_aura_updates(
    bot: &config::BotConfig,
    timeout_secs: u64,
    label: &str,
) -> Result<u32> {
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
    let active_mover_complete = build_move_init_active_mover_complete_payload(0);
    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_MOVE_INIT_ACTIVE_MOVER_COMPLETE,
        &active_mover_complete,
    )
    .await?;
    info!("[Bot {}] ✅ in the world ({})", bot_index, label);

    let mut aura_updates = 0u32;
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
                "aura save",
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
                            "aura save",
                        )
                        .await?;
                    } else if opcode == SMSG_AURA_UPDATE {
                        aura_updates += 1;
                        info!(
                            "[Bot {}] ✅ SMSG_AURA_UPDATE ({} bytes, {})",
                            bot_index,
                            payload.len(),
                            label
                        );
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
    info!("[Bot {}] ✅ clean logout ({})", bot_index, label);
    Ok(aura_updates)
}
