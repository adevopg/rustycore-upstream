//! Live spell-damage check: what `SMSG_SPELL_NON_MELEE_DAMAGE_LOG` reports for a
//! player cast, cast after cast.
//!
//! This is the mode three closed entries were waiting on. A spell critical needs
//! a caster-side percentage (`Unit::SpellCritChanceDone`, `Unit.cpp:7706-7772`)
//! and a spell resist needs a magic school
//! (`Unit::CalcSpellResistedDamage`, `:1973-1975`), so neither can be reached by a
//! warrior however it is fixtured. The mode therefore drives a caster-class
//! character and reports every published field of every cast, with no conclusion
//! drawn in the harness.
//!
//! The character needs no spellbook fixture for a class spell. A freshly created
//! human mage is granted 43 spells at login, Fireball and Frostbolt among them,
//! and none of them is in `character_spell`: C++ `_SaveSpells`
//! (`Entities/Player/Player.cpp:20664-20666`) writes only **non-dependent** rows
//! and `LearnSkillRewardedSpells` learns dependent ones, so that table is the
//! wrong oracle for what a character knows. Seeding a row there for such a spell
//! would write one the target build never writes, so it is opt-in through
//! `--spell-damage-seed-spell` for the case where the spell genuinely is not
//! granted.

use super::*;

/// The harness's own patience for the login, the casts and the clean logout.
pub(crate) const DEFAULT_SPELL_DAMAGE_SMOKE_TIMEOUT_SECS: u64 = 240;
/// How far from the resolved spawn the character is placed. Inside C++'s spell
/// range for a nuke and outside melee reach, so the creature does not interrupt
/// the sequence by closing in on the first cast.
const SPELL_DAMAGE_STAND_OFF_YARDS: f32 = 12.0;
/// How wide a radius the creature's CREATE block is accepted within.
const SPELL_DAMAGE_DISCOVERY_RADIUS_YARDS: f32 = 60.0;

/// One cast's published combat-log row, with nothing inferred.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub(crate) struct SpellDamageCastRow {
    pub damage: i32,
    pub original_damage: i32,
    pub absorbed: i32,
    pub resisted: i32,
    pub overkill: i32,
    pub school_mask: u8,
    /// The seven `HitInfo` bits C++ writes; `0x02` is `SPELL_HIT_TYPE_CRIT`.
    pub flags: u32,
}

#[derive(Debug, Default, serde::Serialize)]
pub(crate) struct SpellDamageSmokeOutcome {
    pub spell_id: i32,
    pub creature_entry: u32,
    pub creature_runtime_counter: u64,
    pub revived_by_fixture: bool,
    pub spellbook_row_seeded: bool,
    pub casts_sent: u32,
    pub cast_failures: u32,
    /// The `SpellCastResult` values the server sent, in order.
    pub cast_failure_reasons: Vec<i32>,
    pub rows: Vec<SpellDamageCastRow>,
}

/// The `SpellCastResult` values this mode has actually seen, named so a run report
/// reads as a reason rather than a number. C++ `SharedDefines.h`'s enum is long;
/// anything unseen prints as its number.
fn spell_cast_result_name_like_cpp(reason: i32) -> &'static str {
    match reason {
        0 => "SPELL_CAST_OK — C++ never sends CastFailed with this",
        32 => "SPELL_FAILED_DONT_REPORT (`SharedDefines.h:1498`)",
        _ => "see SpellCastResult in SharedDefines.h",
    }
}

impl SpellDamageSmokeOutcome {
    fn criticals(&self) -> usize {
        self.rows.iter().filter(|row| row.flags & 0x02 != 0).count()
    }

    fn resisted_casts(&self) -> usize {
        self.rows.iter().filter(|row| row.resisted > 0).count()
    }
}

pub(crate) async fn run_spell_damage_smoke_mode(
    cli: &CliOptions,
    mut bots: Vec<config::BotConfig>,
) -> Result<()> {
    if bots.len() != 1 {
        bail!(
            "--spell-damage needs exactly one enabled bot; select it with --single (got {})",
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
        .spell_damage_spell_id
        .ok_or_else(|| anyhow!("--spell-damage wants the spell id to cast"))?;
    let creature_entry = cli
        .spell_damage_creature_entry
        .ok_or_else(|| anyhow!("--spell-damage-entry wants the creature entry to cast at"))?;
    let character_guid = cli
        .spell_damage_character_guid
        .unwrap_or(bot.character_guid);

    let outcome = run_spell_damage_smoke(
        &bot,
        character_guid,
        spell_id,
        creature_entry,
        cli.spell_damage_casts.max(1),
        cli.spell_damage_seed_spell,
        cli.spell_damage_timeout_secs,
    )
    .await?;
    if let Some(path) = &cli.report_path {
        std::fs::write(path, serde_json::to_string_pretty(&outcome)?)
            .with_context(|| format!("cannot write spell-damage report {path}"))?;
        info!("Spell-damage report written to {path}");
    }
    if outcome.rows.is_empty() {
        bail!(
            "the server published no SMSG_SPELL_NON_MELEE_DAMAGE_LOG for {} casts ({} refusals)",
            outcome.casts_sent,
            outcome.cast_failures
        );
    }
    Ok(())
}

/// Seed the spellbook row and stand the character off from the spawn. Both are
/// writes to columns `Player::SaveToDB` owns; no server path is exercised by them.
/// Returns `(revived, spellbook row seeded, original map, original position)`.
fn seed_spell_damage_scenario_like_cpp(
    bot: &config::BotConfig,
    character_guid: u64,
    spell_id: i32,
    seed_spellbook_row: bool,
    target: (u16, f32, f32, f32),
) -> Result<(bool, bool, u16, (f32, f32, f32))> {
    use mysql::prelude::Queryable;
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
    let mut revive_bot = bot.clone();
    revive_bot.character_guid = character_guid;
    let revived = revive_dead_bot_character_fixture(&mut conn, &revive_bot)?;

    // Only when asked. `character_spell` holds no dependent skill-rewarded spell,
    // so its emptiness says nothing about what the character knows, and inserting
    // a row for such a spell writes one C++ never writes.
    let mut seeded = false;
    if seed_spellbook_row {
        let known: Option<u32> = conn
            .exec_first(
                "SELECT spell FROM character_spell WHERE guid = ? AND spell = ?",
                (character_guid, spell_id),
            )
            .map_err(|error| anyhow!("Read character_spell: {error}"))?;
        if known.is_none() {
            conn.exec_drop(
                "INSERT INTO character_spell (guid, spell, active, disabled) VALUES (?, ?, 1, 0)",
                (character_guid, spell_id),
            )
            .map_err(|error| anyhow!("Spellbook fixture for {character_guid}: {error}"))?;
            seeded = true;
        }
    }

    // Stand off along +x so the cast starts outside melee reach.
    conn.exec_drop(
        "UPDATE characters SET map = ?, position_x = ?, position_y = ?, position_z = ? \
         WHERE guid = ?",
        (
            target.0,
            target.1 + SPELL_DAMAGE_STAND_OFF_YARDS,
            target.2,
            target.3,
            character_guid,
        ),
    )
    .map_err(|error| anyhow!("Position fixture for {character_guid}: {error}"))?;
    Ok((revived, seeded, original_map, (x, y, z)))
}

fn restore_spell_damage_character_like_cpp(
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

/// C++ `WorldPackets::CombatLog::SpellNonMeleeDamageLog::Write`
/// (`Server/Packets/CombatLogPackets.cpp:23-49`). `Flags` is written in seven
/// bits, so a `HitInfo` bit at `0x80` or above never reaches the client.
fn parse_spell_non_melee_damage_log(payload: &[u8]) -> Option<SpellDamageCastRow> {
    let mut cursor = 0usize;
    read_packed_guid_at(payload, &mut cursor)?; // Me
    read_packed_guid_at(payload, &mut cursor)?; // CasterGUID
    read_packed_guid_at(payload, &mut cursor)?; // CastID
    let _spell_id = read_i32_at(payload, &mut cursor)?;
    let _visual = read_i32_at(payload, &mut cursor)?;
    let damage = read_i32_at(payload, &mut cursor)?;
    let original_damage = read_i32_at(payload, &mut cursor)?;
    let overkill = read_i32_at(payload, &mut cursor)?;
    let school_mask = *payload.get(cursor)?;
    cursor += 1;
    let absorbed = read_i32_at(payload, &mut cursor)?;
    let resisted = read_i32_at(payload, &mut cursor)?;
    let _shield_block = read_i32_at(payload, &mut cursor)?;
    let _world_text_viewers = read_i32_at(payload, &mut cursor)?;
    let _supporters = read_i32_at(payload, &mut cursor)?;
    // One `Periodic` bit, then the seven `Flags` bits. `WriteBit` fills a byte
    // from bit 7 down and `WriteBits` writes most significant first, so those
    // eight bits are exactly one byte: `Periodic` in bit 7 and the flags in bits
    // 6..0 in their natural order.
    let bits = *payload.get(cursor)?;
    let flags = u32::from(bits & 0x7F);
    Some(SpellDamageCastRow {
        damage,
        original_damage,
        absorbed,
        resisted,
        overkill,
        school_mask,
        flags,
    })
}

/// C++ `WorldPackets::Spells::CastFailed::Write`: a packed `CastID`, the spell id,
/// the `SpellCastVisual`, then the `SpellCastResult` reason and its two arguments.
///
/// `SpellCastVisual` serialises **one** `uint32` here, not two: `ScriptVisualID`
/// is commented out in this C++ branch, and the port's writer matches
/// (`crates/wow-packet/src/packets/spell.rs:227-229`). Reading two made the reason
/// come back as `FailedArg1`, which is zero — a refusal that looked like success.
fn parse_cast_failed_reason(payload: &[u8]) -> Option<(i32, i32)> {
    let mut cursor = 0usize;
    read_packed_guid_at(payload, &mut cursor)?;
    let spell_id = read_i32_at(payload, &mut cursor)?;
    let _spell_visual_id = read_i32_at(payload, &mut cursor)?;
    let reason = read_i32_at(payload, &mut cursor)?;
    Some((spell_id, reason))
}

fn read_i32_at(payload: &[u8], cursor: &mut usize) -> Option<i32> {
    let bytes = payload.get(*cursor..*cursor + 4)?;
    *cursor += 4;
    Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// A packed `ObjectGuid` as `WorldPacket::write_packed_guid_unflushed` emits it
/// (`crates/wow-packet/src/world_packet.rs:471-502`): **both** mask bytes first,
/// then the low half's non-zero bytes, then the high half's. Reading a mask and
/// its bytes per half instead is only correct for an all-zero GUID, and silently
/// slips by one byte for any real one.
fn read_packed_guid_at(payload: &[u8], cursor: &mut usize) -> Option<(u64, u64)> {
    let low_mask = *payload.get(*cursor)?;
    let high_mask = *payload.get(*cursor + 1)?;
    *cursor += 2;
    let mut halves = [0u64; 2];
    for (half, mask) in halves.iter_mut().zip([low_mask, high_mask]) {
        for bit in 0..8u32 {
            if mask & (1 << bit) != 0 {
                let byte = *payload.get(*cursor)?;
                *cursor += 1;
                *half |= u64::from(byte) << (bit * 8);
            }
        }
    }
    Some((halves[0], halves[1]))
}

#[allow(clippy::too_many_arguments)]
async fn run_spell_damage_smoke(
    bot: &config::BotConfig,
    character_guid: u64,
    spell_id: i32,
    creature_entry: u32,
    casts: u32,
    seed_spellbook_row: bool,
    timeout_secs: u64,
) -> Result<SpellDamageSmokeOutcome> {
    let bot_index = bot.account_id as usize;
    let mut outcome = SpellDamageSmokeOutcome {
        spell_id,
        creature_entry,
        ..Default::default()
    };

    // Resolve the spawn from the world database the way the melee mode does, then
    // stand next to it: the runtime ObjectGuid only exists once it is visible.
    let target = {
        let options = MeleeSmokeOptions {
            creature_entry,
            creature_spawn_guid: None,
            timeout_secs,
            loot_after_kill: false,
        };
        let fixture_guid = character_guid;
        tokio::task::spawn_blocking(move || {
            let (map_id, x, y) = read_character_map_and_position(fixture_guid)?;
            resolve_melee_smoke_target(&options, map_id, x, y)
        })
        .await
        .map_err(|error| anyhow!("Spell-damage target worker failed: {error}"))??
    };
    info!(
        "[Bot {}] target: entry {} spawn {} \"{}\" on map {} at ({:.1}, {:.1}, {:.1})",
        bot_index,
        target.entry,
        target.spawn_guid,
        target.name,
        target.map_id,
        target.x,
        target.y,
        target.z
    );

    let fixture_bot = bot.clone();
    let (revived, seeded, original_map, original_position) = tokio::task::spawn_blocking({
        let target = (target.map_id, target.x, target.y, target.z);
        move || {
            seed_spell_damage_scenario_like_cpp(
                &fixture_bot,
                character_guid,
                spell_id,
                seed_spellbook_row,
                target,
            )
        }
    })
    .await
    .map_err(|error| anyhow!("Spell-damage fixture worker failed: {error}"))??;
    outcome.revived_by_fixture = revived;
    outcome.spellbook_row_seeded = seeded;
    info!(
        "[Bot {}] fixture: character {} standing {} yards from the spawn, spell {} {}{}",
        bot_index,
        character_guid,
        SPELL_DAMAGE_STAND_OFF_YARDS,
        spell_id,
        if seeded {
            "seeded into character_spell on request"
        } else {
            "left to the login's own grant"
        },
        if revived {
            ", restored from a death a previous run left behind"
        } else {
            ""
        }
    );

    let result = drive_spell_damage_scenario_like_cpp(
        bot,
        character_guid,
        spell_id,
        &target,
        casts,
        timeout_secs,
        &mut outcome,
    )
    .await;

    if let Err(error) = tokio::task::spawn_blocking(move || {
        restore_spell_damage_character_like_cpp(character_guid, original_map, original_position)
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
        "[Bot {}] spell-damage summary: spell={} entry={} casts_sent={} refusals={} logs={} \
         criticals={} resisted_casts={}",
        bot_index,
        spell_id,
        creature_entry,
        outcome.casts_sent,
        outcome.cast_failures,
        outcome.rows.len(),
        outcome.criticals(),
        outcome.resisted_casts()
    );
    if !outcome.cast_failure_reasons.is_empty() {
        info!(
            "[Bot {}] refusal reasons (SpellCastResult): {:?}",
            bot_index, outcome.cast_failure_reasons
        );
    }
    for (index, row) in outcome.rows.iter().enumerate() {
        info!(
            "[Bot {}] cast {}: damage={} original={} resisted={} absorbed={} school=0x{:02X} flags=0x{:02X}",
            bot_index,
            index + 1,
            row.damage,
            row.original_damage,
            row.resisted,
            row.absorbed,
            row.school_mask,
            row.flags
        );
    }
    Ok(outcome)
}

fn read_character_map_and_position(character_guid: u64) -> Result<(u16, f32, f32)> {
    use mysql::prelude::Queryable;
    let url = characters_db_url()?;
    let mut conn = mysql::Conn::new(qa_mysql_opts(&url, "characters")?)
        .map_err(|error| anyhow!("Connect to characters DB failed: {error}"))?;
    let row: Option<(u16, f32, f32)> = conn
        .exec_first(
            "SELECT map, position_x, position_y FROM characters WHERE guid = ?",
            (character_guid,),
        )
        .map_err(|error| anyhow!("Read character position: {error}"))?;
    row.ok_or_else(|| anyhow!("No characters row for guid {character_guid}"))
}

async fn drive_spell_damage_scenario_like_cpp(
    bot: &config::BotConfig,
    character_guid: u64,
    spell_id: i32,
    target: &MeleeSmokeTarget,
    casts: u32,
    timeout_secs: u64,
    outcome: &mut SpellDamageSmokeOutcome,
) -> Result<()> {
    let bot_index = bot.account_id as usize;
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
    // The CREATE blocks that arrive with the login carry the spawn's runtime
    // ObjectGuid, so collect them the way the melee mode does.
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
    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_MOVE_INIT_ACTIVE_MOVER_COMPLETE,
        &build_move_init_active_mover_complete_payload(0),
    )
    .await?;
    info!("[Bot {}] ✅ in the world beside the spawn", bot_index);

    // The creature's runtime ObjectGuid arrives in a CREATE block.
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
            SPELL_DAMAGE_DISCOVERY_RADIUS_YARDS,
            None,
        );
    }
    let discovery_deadline = deadline.min(std::time::Instant::now() + Duration::from_secs(20));
    while discovered.is_none() && std::time::Instant::now() < discovery_deadline {
        for route in [false, true] {
            let target_connection = match route {
                true => match realm_connection.as_mut() {
                    Some(realm) => realm,
                    None => continue,
                },
                false => &mut connection,
            };
            if let Some((opcode, payload)) = read_encrypted_packet_if_ready(
                &mut target_connection.stream,
                &mut target_connection.crypt,
                &mut target_connection.inflater,
                Duration::from_millis(200),
                Duration::from_secs(5),
                "spell damage discovery",
            )
            .await?
            {
                if opcode == SMSG_TIME_SYNC_REQUEST {
                    let (stream, crypt) =
                        (&mut target_connection.stream, &mut target_connection.crypt);
                    respond_to_detour_time_sync_like_cpp(
                        bot_index,
                        stream,
                        crypt,
                        &payload,
                        clock_origin,
                        "spell damage",
                    )
                    .await?;
                } else if opcode == SMSG_UPDATE_OBJECT {
                    discovered = find_creature_guid_near_position_in_update_object(
                        &payload,
                        target.map_id,
                        target.entry,
                        target.x,
                        target.y,
                        target.z,
                        SPELL_DAMAGE_DISCOVERY_RADIUS_YARDS,
                        None,
                    );
                }
            }
        }
        if discovered.is_none() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    let runtime = discovered.ok_or_else(|| {
        anyhow!(
            "the server never published a CREATE block for entry {} near the spawn",
            target.entry
        )
    })?;
    outcome.creature_runtime_counter = runtime.low & 0x0000_FFFF_FFFF_FFFF;
    info!(
        "[Bot {}] ✅ target visible: low=0x{:X} high=0x{:X}",
        bot_index, runtime.low, runtime.high
    );

    // One loop that sends and drains, rather than a read window per cast. The
    // completion of a cast with a cast time is not synchronous with the request —
    // the observed gap between `CMSG_CAST_SPELL` and the damage log is several
    // seconds — so attributing one log to one request by a short window loses
    // rows the server did publish. Every log seen is recorded in the order it
    // arrived.
    let cast_interval = Duration::from_secs(6);
    let mut next_cast_at = tokio::time::Instant::now();
    let mut casts_sent = 0u32;
    let drain_until = deadline
        .min(std::time::Instant::now() + cast_interval.saturating_mul(casts.saturating_add(2)));
    while std::time::Instant::now() < drain_until {
        if casts_sent < casts && tokio::time::Instant::now() >= next_cast_at {
            // C++ correlates the cast by its `CastID`; a fresh one per cast is
            // what a client sends.
            let cast_id = (u64::from(casts_sent) + 1, 0u64);
            let body = crate::cast_lifecycle::build_unit_target_cast_payload_like_cpp(
                spell_id,
                cast_id,
                (runtime.low, runtime.high),
            );
            send_encrypted_packet(
                &mut connection.stream,
                &mut connection.crypt,
                crate::cast_lifecycle::CMSG_CAST_SPELL,
                &body,
            )
            .await?;
            casts_sent += 1;
            outcome.casts_sent = casts_sent;
            next_cast_at = tokio::time::Instant::now() + cast_interval;
        }

        let mut saw_any = false;
        for route in [false, true] {
            let target_connection = match route {
                true => match realm_connection.as_mut() {
                    Some(realm) => realm,
                    None => continue,
                },
                false => &mut connection,
            };
            if let Some((opcode, payload)) = read_encrypted_packet_if_ready(
                &mut target_connection.stream,
                &mut target_connection.crypt,
                &mut target_connection.inflater,
                Duration::from_millis(200),
                Duration::from_secs(5),
                "spell damage",
            )
            .await?
            {
                saw_any = true;
                if std::env::var_os("WOW_BOT_SPELL_DAMAGE_TRACE").is_some() {
                    info!(
                        "[Bot {}] trace: {} opcode 0x{:04X} ({} bytes)",
                        bot_index,
                        if route { "realm" } else { "instance" },
                        opcode,
                        payload.len()
                    );
                }
                if opcode == SMSG_TIME_SYNC_REQUEST {
                    let (stream, crypt) =
                        (&mut target_connection.stream, &mut target_connection.crypt);
                    respond_to_detour_time_sync_like_cpp(
                        bot_index,
                        stream,
                        crypt,
                        &payload,
                        clock_origin,
                        "spell damage",
                    )
                    .await?;
                } else if opcode == SMSG_SPELL_NON_MELEE_DAMAGE_LOG {
                    match parse_spell_non_melee_damage_log(&payload) {
                        Some(row) => outcome.rows.push(row),
                        None => warn!(
                            "[Bot {}] malformed SMSG_SPELL_NON_MELEE_DAMAGE_LOG ({} bytes)",
                            bot_index,
                            payload.len()
                        ),
                    }
                } else if opcode == SMSG_CAST_FAILED || opcode == SMSG_SPELL_FAILURE {
                    outcome.cast_failures += 1;
                    match (opcode == SMSG_CAST_FAILED)
                        .then(|| parse_cast_failed_reason(&payload))
                        .flatten()
                    {
                        Some((failed_spell_id, reason)) => {
                            outcome.cast_failure_reasons.push(reason);
                            warn!(
                                "[Bot {}] cast refused: spell {} SpellCastResult {} ({})",
                                bot_index,
                                failed_spell_id,
                                reason,
                                spell_cast_result_name_like_cpp(reason)
                            );
                        }
                        None => warn!(
                            "[Bot {}] a cast was refused: opcode 0x{:04X} ({} bytes)",
                            bot_index,
                            opcode,
                            payload.len()
                        ),
                    }
                }
            }
        }
        if !saw_any {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // Stop early once every request has had its interval to land.
        if casts_sent >= casts
            && outcome.rows.len() + outcome.cast_failures as usize >= casts as usize
        {
            if std::env::var_os("WOW_BOT_SPELL_DAMAGE_TRACE").is_some() {
                info!(
                    "[Bot {}] trace: loop break with casts_sent={} rows={} failures={}",
                    bot_index,
                    casts_sent,
                    outcome.rows.len(),
                    outcome.cast_failures
                );
            }
            break;
        }
    }
    if std::env::var_os("WOW_BOT_SPELL_DAMAGE_TRACE").is_some() {
        info!(
            "[Bot {}] trace: loop end, casts_sent={} rows={} failures={} drain_expired={}",
            bot_index,
            casts_sent,
            outcome.rows.len(),
            outcome.cast_failures,
            std::time::Instant::now() >= drain_until
        );
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
    info!("[Bot {}] ✅ clean logout", bot_index);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The combat-log reader walks exactly the field order C++
    /// `SpellNonMeleeDamageLog::Write` produces, including the two packed GUIDs
    /// before the spell id and the seven-bit `Flags` tail.
    #[test]
    fn the_damage_log_reader_follows_the_cpp_field_order() {
        let mut payload = Vec::new();
        // `Me` is a real creature GUID, so the two mask bytes come first and the
        // low half's bytes before the high half's. The other two are empty.
        payload.extend([0x01u8, 0xA2u8]); // low mask: byte 0; high mask: bytes 1, 5, 7
        payload.push(0xC1); // low = 0xC1
        payload.extend([0x0A, 0x04, 0x20]); // high = 0x2000_0400_0000_0A00
        payload.extend([0u8, 0u8].repeat(2));
        payload.extend(133i32.to_le_bytes()); // SpellID
        payload.extend(0i32.to_le_bytes()); // Visual
        payload.extend(42i32.to_le_bytes()); // Damage
        payload.extend(60i32.to_le_bytes()); // OriginalDamage
        payload.extend((-1i32).to_le_bytes()); // Overkill
        payload.push(0x10); // SchoolMask: frost
        payload.extend(0i32.to_le_bytes()); // Absorbed
        payload.extend(18i32.to_le_bytes()); // Resisted
        payload.extend(0i32.to_le_bytes()); // ShieldBlock
        payload.extend(0i32.to_le_bytes()); // WorldTextViewers
        payload.extend(0i32.to_le_bytes()); // Supporters
                                            // `Periodic` false in bit 7, `Flags` = 0x02 in bits 6..0.
        payload.push(0b0000_0010);

        let row = parse_spell_non_melee_damage_log(&payload).expect("the row must parse");
        // The reader must have consumed the real GUID exactly, or every field
        // below would be read one byte out of place.
        let mut cursor = 0usize;
        assert_eq!(
            read_packed_guid_at(&payload, &mut cursor),
            Some((0xC1, 0x2000_0400_0000_0A00))
        );
        assert_eq!(cursor, 6);
        assert_eq!(row.damage, 42);
        assert_eq!(row.original_damage, 60);
        assert_eq!(row.resisted, 18);
        assert_eq!(row.absorbed, 0);
        assert_eq!(row.overkill, -1);
        assert_eq!(row.school_mask, 0x10);
        assert_eq!(row.flags, 0x02, "SPELL_HIT_TYPE_CRIT");
    }

    /// A truncated payload is reported rather than read past its end.
    #[test]
    fn a_truncated_damage_log_is_refused() {
        assert!(parse_spell_non_melee_damage_log(&[0u8; 8]).is_none());
    }

    /// `SpellCastVisual` is one `uint32` on this branch, so the reason sits four
    /// bytes earlier than a two-field reading would put it. Getting that wrong
    /// reported every refusal as `SPELL_CAST_OK`.
    #[test]
    fn the_cast_failed_reader_places_the_reason_after_one_visual_field() {
        let mut payload = Vec::new();
        payload.extend([0x01u8, 0x00u8]); // packed CastID: low byte 0 set, high empty
        payload.push(0x07);
        payload.extend(133i32.to_le_bytes()); // SpellID
        payload.extend(0i32.to_le_bytes()); // SpellCastVisual::SpellXSpellVisualID
        payload.extend(32i32.to_le_bytes()); // Reason: SPELL_FAILED_DONT_REPORT
        payload.extend(0i32.to_le_bytes()); // FailedArg1
        payload.extend(0i32.to_le_bytes()); // FailedArg2

        assert_eq!(parse_cast_failed_reason(&payload), Some((133, 32)));
        assert!(parse_cast_failed_reason(&payload[..6]).is_none());
    }
}
