//! Character provisioning through the real CMSG_CREATE_CHARACTER path.
//!
//! Every other QA workflow in this tool assumes a `characters` row already
//! exists: `validate_local_bot_character_owner` refuses to run without one, and
//! the shipped example config points at a guid from a pre-existing development
//! database. On a freshly bootstrapped realm there is no such row and no way to
//! make one, because the bot never implemented character creation.
//!
//! This module closes that gap over the wire instead of by SQL insert: it
//! authenticates, enumerates, sends CMSG_CREATE_CHARACTER, reads the server's
//! SMSG_CREATE_CHAR verdict, enumerates again and only then confirms the row in
//! the characters database. The created guid is whatever the server assigned —
//! nothing here fabricates a guid, a position or an item.

use super::*;

/// `CMSG_ENUM_CHARACTERS` (3.4.3 `0x35E9`).
pub(crate) const CMSG_ENUM_CHARACTERS: u16 = 0x35E9;
/// `SMSG_ENUM_CHARACTERS_RESULT` (3.4.3 `0x2583`).
pub(crate) const SMSG_ENUM_CHARACTERS_RESULT: u16 = 0x2583;
/// `CMSG_CREATE_CHARACTER` (3.4.3 `0x3645`).
pub(crate) const CMSG_CREATE_CHARACTER: u16 = 0x3645;
/// `SMSG_CREATE_CHAR` (3.4.3 `0x2701`).
pub(crate) const SMSG_CREATE_CHAR: u16 = 0x2701;

/// `CMSG_PLAYER_LOGIN` (3.4.3 `0x35EB`).
pub(crate) const CMSG_PLAYER_LOGIN: u16 = 0x35EB;
/// `SMSG_LOGIN_VERIFY_WORLD` (3.4.3 `0x2597`).
pub(crate) const SMSG_LOGIN_VERIFY_WORLD: u16 = 0x2597;

/// C++ `ResponseCodes` (`SharedDefines.h`), character-creation range.
pub(crate) const CHAR_CREATE_SUCCESS_LIKE_CPP: u8 = 24;
/// C++ `AT_LOGIN_FIRST` (`Entities/Player/Player.h:535`). Character creation
/// sets it (`Handlers/CharacterHandler.cpp:888`) and the first
/// `HandlePlayerLogin` clears it (`CharacterHandler.cpp:1271`), so a freshly
/// created character carries exactly this flag and nothing else.
pub(crate) const AT_LOGIN_FIRST_LIKE_CPP: u16 = 0x020;

/// Human, the race whose `playercreateinfo` start position and starting items
/// TDB always ships; nothing here depends on it beyond the default.
pub(crate) const DEFAULT_CREATE_CHARACTER_RACE: u8 = 1;
/// Warrior: no starting spell power requirement, so the creation path is the
/// least data-dependent one available.
pub(crate) const DEFAULT_CREATE_CHARACTER_CLASS: u8 = 1;
pub(crate) const DEFAULT_CREATE_CHARACTER_TIMEOUT_SECS: u64 = 30;

/// C++ `CharacterCreateInfo::Name` is bounded by the 6-bit wire length field.
const MAX_CREATE_CHARACTER_NAME_BITS: usize = 0x3F;

#[derive(Debug, Clone)]
pub(crate) struct CreateCharacterOptions {
    pub(crate) name: String,
    pub(crate) race: u8,
    pub(crate) class: u8,
    pub(crate) sex: i8,
    pub(crate) timeout_secs: u64,
}

/// The `characters` row the server wrote, read back after creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreatedCharacterRow {
    pub(crate) guid: u64,
    pub(crate) name: String,
    pub(crate) account: u32,
    pub(crate) race: u8,
    pub(crate) class: u8,
    pub(crate) level: u8,
    pub(crate) map: u16,
    pub(crate) online: u8,
    pub(crate) at_login: u16,
}

/// C++ `WorldPackets::Character::CreateChar::Write`: `uint8 Code` followed by a
/// packed `ObjectGuid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CreateCharResponseLikeCpp {
    pub(crate) code: u8,
    pub(crate) guid_low: u64,
    pub(crate) guid_high: u64,
}

/// Build the `CMSG_CREATE_CHARACTER` body exactly as C++
/// `WorldPackets::Character::CreateCharacter::Read` consumes it
/// (`Server/Packets/CharacterPackets.cpp:353`):
///
/// `ReadBits(6)` name length, `ReadBit()` hasTemplateSet, `ReadBit()`
/// IsTrialBoost, `ReadBit()` UseNPE, then — because the next `read<T>` resets
/// the bit position — byte-aligned Race, Class, Sex, a `uint32` customization
/// count, the name bytes, the optional template set and the choice pairs.
///
/// `ByteBuffer` packs bits most-significant first, so the nine header bits
/// occupy two bytes: the length lands in bits 7..2 of the first one and UseNPE
/// in bit 7 of the second, whose remaining bits are the padding `FlushBits`
/// leaves behind. This builder never sets a template set, so no `int32`
/// follows the name.
pub(crate) fn build_cmsg_create_character_like_cpp(
    options: &CreateCharacterOptions,
    customizations: &[(i32, i32)],
) -> Result<Vec<u8>> {
    let name_bytes = options.name.as_bytes();
    if name_bytes.is_empty() {
        bail!("Character name is empty");
    }
    if name_bytes.len() > MAX_CREATE_CHARACTER_NAME_BITS {
        bail!(
            "Character name is {} bytes; the 6-bit wire length field holds at most {}",
            name_bytes.len(),
            MAX_CREATE_CHARACTER_NAME_BITS
        );
    }

    let mut data = Vec::with_capacity(9 + name_bytes.len() + customizations.len() * 8);
    data.push((name_bytes.len() as u8) << 2);
    data.push(0);
    data.push(options.race);
    data.push(options.class);
    data.push(options.sex as u8);
    data.extend_from_slice(&(customizations.len() as u32).to_le_bytes());
    data.extend_from_slice(name_bytes);
    for (option_id, choice_id) in customizations {
        data.extend_from_slice(&option_id.to_le_bytes());
        data.extend_from_slice(&choice_id.to_le_bytes());
    }
    Ok(data)
}

pub(crate) fn parse_smsg_create_char_like_cpp(payload: &[u8]) -> Result<CreateCharResponseLikeCpp> {
    let code = *payload
        .first()
        .ok_or_else(|| anyhow!("SMSG_CREATE_CHAR carried no response code"))?;
    let (_, guid_low, guid_high) = parse_packed_guid(&payload[1..])
        .ok_or_else(|| anyhow!("SMSG_CREATE_CHAR carried no packed guid"))?;
    Ok(CreateCharResponseLikeCpp {
        code,
        guid_low,
        guid_high,
    })
}

/// C++ `ResponseCodes` names for the character-creation range, so a refusal is
/// reported as the server meant it instead of as a bare number.
pub(crate) fn create_char_response_name_like_cpp(code: u8) -> &'static str {
    match code {
        23 => "CHAR_CREATE_IN_PROGRESS",
        24 => "CHAR_CREATE_SUCCESS",
        25 => "CHAR_CREATE_ERROR",
        26 => "CHAR_CREATE_FAILED",
        27 => "CHAR_CREATE_NAME_IN_USE",
        28 => "CHAR_CREATE_DISABLED",
        29 => "CHAR_CREATE_PVP_TEAMS_VIOLATION",
        30 => "CHAR_CREATE_SERVER_LIMIT",
        31 => "CHAR_CREATE_ACCOUNT_LIMIT",
        32 => "CHAR_CREATE_SERVER_QUEUE",
        33 => "CHAR_CREATE_ONLY_EXISTING",
        34 => "CHAR_CREATE_EXPANSION",
        35 => "CHAR_CREATE_EXPANSION_CLASS",
        36 => "CHAR_CREATE_CHARACTER_IN_GUILD",
        37 => "CHAR_CREATE_RESTRICTED_RACECLASS",
        38 => "CHAR_CREATE_CHARACTER_CHOOSE_RACE",
        39 => "CHAR_CREATE_CHARACTER_ARENA_LEADER",
        40 => "CHAR_CREATE_CHARACTER_DELETE_MAIL",
        41 => "CHAR_CREATE_CHARACTER_SWAP_FACTION",
        42 => "CHAR_CREATE_CHARACTER_RACE_ONLY",
        43 => "CHAR_CREATE_CHARACTER_GOLD_LIMIT",
        44 => "CHAR_CREATE_FORCE_LOGIN",
        45 => "CHAR_CREATE_TRIAL",
        51 => "CHAR_CREATE_NEW_PLAYER",
        _ => "unmapped ResponseCodes value",
    }
}

/// Reject a name the server would refuse before spending a round trip on it.
/// The authority stays with C++ `ObjectMgr::CheckPlayerName`; this only mirrors
/// the length bound and the ASCII-letter rule the Rust handler enforces.
pub(crate) fn validate_create_character_name(name: &str) -> Result<()> {
    if name.chars().count() < 2 || name.chars().count() > 12 {
        bail!("Character name must be 2 to 12 characters, got {:?}", name);
    }
    if !name.chars().all(|c| c.is_ascii_alphabetic()) {
        bail!("Character name must be ASCII letters only, got {:?}", name);
    }
    Ok(())
}

fn connect_characters_db() -> Result<mysql::Conn> {
    let char_db = characters_db_url()?;
    let opts = qa_mysql_opts(&char_db, "characters")?;
    mysql::Conn::new(opts).map_err(|e| anyhow!("Connect to characters DB failed: {e}"))
}

pub(crate) fn read_created_character_row(
    conn: &mut mysql::Conn,
    guid: u64,
) -> Result<Option<CreatedCharacterRow>> {
    use mysql::prelude::Queryable;

    let row: Option<(String, u32, u8, u8, u8, u16, u8, u16)> = conn
        .exec_first(
            "SELECT name, account, race, class, level, map, online, at_login \
             FROM characters WHERE guid = ?",
            (guid,),
        )
        .map_err(|e| anyhow!("Read back created character {guid}: {e}"))?;

    Ok(row.map(
        |(name, account, race, class, level, map, online, at_login)| CreatedCharacterRow {
            guid,
            name,
            account,
            race,
            class,
            level,
            map,
            online,
            at_login,
        },
    ))
}

/// Confirm the row the server wrote is the character that was asked for, and
/// that it carries exactly the flags C++ character creation leaves behind.
pub(crate) fn verify_created_character_row(
    row: &CreatedCharacterRow,
    bot: &config::BotConfig,
    options: &CreateCharacterOptions,
) -> Result<()> {
    if row.account != bot.account_id {
        bail!(
            "Created character {} belongs to account {}, expected {}",
            row.guid,
            row.account,
            bot.account_id
        );
    }
    if !row.name.eq_ignore_ascii_case(&options.name) {
        bail!(
            "Created character {} is named {:?}, expected {:?}",
            row.guid,
            row.name,
            options.name
        );
    }
    if row.race != options.race || row.class != options.class {
        bail!(
            "Created character {} is race {} class {}, expected race {} class {}",
            row.guid,
            row.race,
            row.class,
            options.race,
            options.class
        );
    }
    if row.level == 0 {
        bail!("Created character {} was stored at level 0", row.guid);
    }
    if row.online != 0 {
        bail!("Created character {} was stored online", row.guid);
    }
    if row.at_login != AT_LOGIN_FIRST_LIKE_CPP {
        bail!(
            "Created character {} has at_login {}, expected exactly AT_LOGIN_FIRST ({})",
            row.guid,
            row.at_login,
            AT_LOGIN_FIRST_LIKE_CPP
        );
    }
    Ok(())
}

/// After the first login and a clean logout the row must be the offline fixture
/// every other workflow requires (`validate_local_bot_character_owner`): C++
/// clears `AT_LOGIN_FIRST` on that login and the logout save persists it.
pub(crate) fn verify_first_login_cleared_the_fixture(row: &CreatedCharacterRow) -> Result<()> {
    if row.online != 0 || row.at_login != 0 {
        bail!(
            "Character {} is not a clean offline fixture after its first login              (online={}, at_login={})",
            row.guid,
            row.online,
            row.at_login
        );
    }
    Ok(())
}

/// Create one character for `bot` over the wire and return the guid the server
/// assigned.
pub(crate) async fn run_create_character(
    bot: &config::BotConfig,
    options: &CreateCharacterOptions,
) -> Result<u64> {
    validate_create_character_name(&options.name)?;
    let payload = build_cmsg_create_character_like_cpp(options, &[])?;
    let bot_index = bot.account_id as usize;

    info!(
        "[Bot {}] Character provisioning: name={:?} race={} class={} sex={}",
        bot_index, options.name, options.race, options.class, options.sex
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

    let deadline = std::time::Instant::now() + Duration::from_secs(options.timeout_secs);

    // A real client always enumerates before offering the creation screen, and
    // the enumeration is what proves the account starts without this character.
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
        "SMSG_ENUM_CHARACTERS_RESULT before creation",
    )
    .await?;
    info!("[Bot {}] ✅ enumeration before creation", bot_index);

    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_CREATE_CHARACTER,
        &payload,
    )
    .await?;
    info!("[Bot {}] ✅ CMSG_CREATE_CHARACTER sent", bot_index);

    let response_payload = expect_encrypted_opcode(
        &mut connection,
        SMSG_CREATE_CHAR,
        deadline,
        "SMSG_CREATE_CHAR",
    )
    .await?;
    let response = parse_smsg_create_char_like_cpp(&response_payload)?;
    if response.code != CHAR_CREATE_SUCCESS_LIKE_CPP {
        bail!(
            "Server refused character creation: code {} ({})",
            response.code,
            create_char_response_name_like_cpp(response.code)
        );
    }
    let guid = response.guid_low;
    info!(
        "[Bot {}] ✅ SMSG_CREATE_CHAR {} guid={} (high=0x{:016X})",
        bot_index,
        create_char_response_name_like_cpp(response.code),
        guid,
        response.guid_high
    );

    // The server must now list it; that is the end-to-end evidence, not the
    // response packet alone.
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
        "SMSG_ENUM_CHARACTERS_RESULT after creation",
    )
    .await?;
    info!("[Bot {}] ✅ enumeration after creation", bot_index);

    let bot_for_db = bot.clone();
    let options_for_db = options.clone();
    let row = tokio::task::spawn_blocking(move || -> Result<CreatedCharacterRow> {
        let mut conn = connect_characters_db()?;
        let row = read_created_character_row(&mut conn, guid)?
            .ok_or_else(|| anyhow!("Server reported guid {guid} but wrote no characters row"))?;
        verify_created_character_row(&row, &bot_for_db, &options_for_db)?;
        Ok(row)
    })
    .await
    .map_err(|e| anyhow!("Character read-back worker join failed: {e}"))??;

    info!(
        "[Bot {}] ✅ characters row: guid={} name={:?} account={} race={} class={} level={} map={} at_login={}",
        bot_index,
        row.guid,
        row.name,
        row.account,
        row.race,
        row.class,
        row.level,
        row.map,
        row.at_login
    );

    // A character that still carries AT_LOGIN_FIRST is not yet the fixture the
    // other workflows accept, and one that cannot enter the world is useless as
    // a fixture anyway. Let the server clear the flag the way C++ does — on the
    // first login — and let the logout save persist it.
    info!(
        "[Bot {}] First login for guid {} to clear AT_LOGIN_FIRST",
        bot_index, guid
    );
    let login_body = build_player_login(guid, realm_id(), 500.0);
    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_PLAYER_LOGIN,
        &login_body,
    )
    .await?;
    let login_deadline = std::time::Instant::now() + Duration::from_secs(options.timeout_secs);
    let mut realm_connection: Option<EncryptedWorldConnection> = None;
    expect_login_opcode_across_connect_to(
        bot_index,
        &mut connection,
        &mut realm_connection,
        &authenticated.derived_session_key,
        SMSG_LOGIN_VERIFY_WORLD,
        login_deadline,
        "SMSG_LOGIN_VERIFY_WORLD",
        None,
    )
    .await?;
    info!("[Bot {}] ✅ SMSG_LOGIN_VERIFY_WORLD received", bot_index);

    send_encrypted_packet(
        &mut connection.stream,
        &mut connection.crypt,
        CMSG_LOGOUT_REQUEST,
        &[0],
    )
    .await?;
    let logout_deadline =
        std::time::Instant::now() + Duration::from_secs(NORMAL_LOGOUT_COMPLETE_WAIT_SECS);
    // C++ routes the two login answers to different sockets:
    // SMSG_LOGIN_VERIFY_WORLD is CONNECTION_TYPE_INSTANCE
    // (`Server/Protocol/Opcodes.cpp:1658`) while SMSG_LOGOUT_COMPLETE is
    // CONNECTION_TYPE_REALM (`Opcodes.cpp:1660`). After the redirect the
    // completion therefore arrives on the preserved realm connection, not on
    // the instance socket the request went out on.
    let logout_route = realm_connection.as_mut().unwrap_or(&mut connection);
    expect_encrypted_opcode(
        logout_route,
        SMSG_LOGOUT_COMPLETE,
        logout_deadline,
        "SMSG_LOGOUT_COMPLETE",
    )
    .await?;
    info!("[Bot {}] ✅ SMSG_LOGOUT_COMPLETE received", bot_index);

    let row = tokio::task::spawn_blocking(move || -> Result<CreatedCharacterRow> {
        let mut conn = connect_characters_db()?;
        let row = read_created_character_row(&mut conn, guid)?
            .ok_or_else(|| anyhow!("Character {guid} disappeared after its first login"))?;
        verify_first_login_cleared_the_fixture(&row)?;
        Ok(row)
    })
    .await
    .map_err(|e| anyhow!("Fixture read-back worker join failed: {e}"))??;

    info!(
        "[Bot {}] ✅ clean offline fixture: guid={} level={} map={} online={} at_login={}",
        bot_index, row.guid, row.level, row.map, row.online, row.at_login
    );
    info!(
        "[Bot {}] Set \"character_guid\": {} in the bot config to use it",
        bot_index, row.guid
    );

    Ok(row.guid)
}

/// The `--create-character` entry point: one exclusive mode, one bot, one
/// character. Keeping the guards here rather than in `main` leaves the mode's
/// preconditions next to the workflow they protect.
pub(crate) async fn run_create_character_mode(
    cli: &CliOptions,
    mut bots: Vec<config::BotConfig>,
) -> Result<()> {
    if any_exclusive_workflow_mode_selected(cli) {
        bail!("--create-character provisions a character and runs no other workflow");
    }
    if bots.len() != 1 {
        bail!(
            "--create-character needs exactly one enabled bot; select it with --single (got {})",
            bots.len()
        );
    }
    let name = cli
        .create_character_name
        .clone()
        .context("--create-character requires --create-character-name")?;
    let bot = bots.remove(0);
    if bot.password.trim().is_empty() {
        bail!(
            "No password for {}; export {}",
            bot.account,
            password_env_name(&bot.account)
        );
    }
    let options = CreateCharacterOptions {
        name,
        race: cli.create_character_race,
        class: cli.create_character_class,
        sex: cli.create_character_sex,
        timeout_secs: cli.create_character_timeout_secs,
    };
    // Account provisioning stays create-only, but it must tolerate the missing
    // character: that is the state this mode exists to leave behind.
    let bots_for_db = vec![bot.clone()];
    tokio::task::spawn_blocking(move || {
        ensure_test_accounts_allowing_absent_character(&bots_for_db)
    })
    .await
    .map_err(|e| anyhow!("DB worker join failed while provisioning test accounts: {e}"))?
    .map_err(|e| anyhow!("Failed to provision test accounts: {e}"))?;
    let guid = run_create_character(&bot, &options).await?;
    info!("Created character guid {guid} for account {}", bot.account);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(name: &str) -> CreateCharacterOptions {
        CreateCharacterOptions {
            name: name.to_string(),
            race: 1,
            class: 1,
            sex: 0,
            timeout_secs: 30,
        }
    }

    #[test]
    fn create_character_body_matches_the_cpp_read_order() {
        let body = build_cmsg_create_character_like_cpp(&options("Rustyqa"), &[]).unwrap();

        // 7 name bytes in bits 7..2 of the first byte; hasTemplateSet and
        // IsTrialBoost are the two low bits, UseNPE the top bit of the second.
        assert_eq!(body[0], 7 << 2);
        assert_eq!(body[1], 0);
        assert_eq!(body[2], 1, "race");
        assert_eq!(body[3], 1, "class");
        assert_eq!(body[4], 0, "sex");
        assert_eq!(&body[5..9], &0u32.to_le_bytes(), "customization count");
        assert_eq!(&body[9..], b"Rustyqa");
        assert_eq!(body.len(), 16);
    }

    #[test]
    fn create_character_body_carries_customizations_after_the_name() {
        let body =
            build_cmsg_create_character_like_cpp(&options("Rustyqa"), &[(1, 2), (3, 4)]).unwrap();

        assert_eq!(&body[5..9], &2u32.to_le_bytes());
        assert_eq!(&body[9..16], b"Rustyqa");
        assert_eq!(&body[16..20], &1i32.to_le_bytes());
        assert_eq!(&body[20..24], &2i32.to_le_bytes());
        assert_eq!(&body[24..28], &3i32.to_le_bytes());
        assert_eq!(&body[28..32], &4i32.to_le_bytes());
        assert_eq!(body.len(), 32);
    }

    #[test]
    fn create_character_body_rejects_a_name_longer_than_the_bit_field() {
        let long = "a".repeat(MAX_CREATE_CHARACTER_NAME_BITS + 1);
        assert!(build_cmsg_create_character_like_cpp(&options(&long), &[]).is_err());
    }

    #[test]
    fn create_char_response_reads_code_then_packed_guid() {
        // code 24, low mask 0x01 (counter 17), high mask 0xC0 ((2 << 58) | realm 1 << 42).
        let high: u64 = (2u64 << 58) | (1u64 << 42);
        let mut payload = vec![CHAR_CREATE_SUCCESS_LIKE_CPP, 0x01, 0x00];
        payload[2] = 0;
        payload.push(17);
        let (high_mask, high_bytes) = pack_u64(high);
        payload[2] = high_mask;
        payload.extend_from_slice(&high_bytes);

        let parsed = parse_smsg_create_char_like_cpp(&payload).unwrap();
        assert_eq!(parsed.code, CHAR_CREATE_SUCCESS_LIKE_CPP);
        assert_eq!(parsed.guid_low, 17);
        assert_eq!(parsed.guid_high, high);
    }

    #[test]
    fn create_char_response_rejects_a_truncated_guid() {
        assert!(parse_smsg_create_char_like_cpp(&[]).is_err());
        assert!(parse_smsg_create_char_like_cpp(&[24]).is_err());
        assert!(parse_smsg_create_char_like_cpp(&[24, 0x01, 0x00]).is_err());
    }

    #[test]
    fn response_names_cover_the_cpp_creation_range() {
        assert_eq!(
            create_char_response_name_like_cpp(24),
            "CHAR_CREATE_SUCCESS"
        );
        assert_eq!(
            create_char_response_name_like_cpp(27),
            "CHAR_CREATE_NAME_IN_USE"
        );
        assert_eq!(
            create_char_response_name_like_cpp(31),
            "CHAR_CREATE_ACCOUNT_LIMIT"
        );
        assert_eq!(
            create_char_response_name_like_cpp(200),
            "unmapped ResponseCodes value"
        );
    }

    #[test]
    fn names_the_rust_handler_would_refuse_never_reach_the_wire() {
        assert!(validate_create_character_name("A").is_err());
        assert!(validate_create_character_name("Waytoolongname").is_err());
        assert!(validate_create_character_name("Rusty1").is_err());
        assert!(validate_create_character_name("Rustyqa").is_ok());
    }

    #[test]
    fn read_back_verification_demands_the_clean_offline_fixture() {
        let bot = config::BotConfig {
            account: "TESTBOT1@bot.local".to_string(),
            password: String::new(),
            character_guid: 0,
            account_id: 8,
            lfg_role: 2,
            class: "warrior".to_string(),
            enabled: true,
            session_key_bnet: String::new(),
        };
        let options = options("Rustyqa");
        let clean = CreatedCharacterRow {
            guid: 17,
            name: "Rustyqa".to_string(),
            account: 8,
            race: 1,
            class: 1,
            level: 1,
            map: 0,
            online: 0,
            at_login: 0,
        };
        assert!(
            verify_created_character_row(&clean, &bot, &options).is_err(),
            "at_login must be exactly AT_LOGIN_FIRST right after creation"
        );
        let fresh = CreatedCharacterRow {
            at_login: AT_LOGIN_FIRST_LIKE_CPP,
            ..clean.clone()
        };
        assert!(verify_created_character_row(&fresh, &bot, &options).is_ok());
        assert!(
            verify_first_login_cleared_the_fixture(&fresh).is_err(),
            "AT_LOGIN_FIRST must be gone once the character has logged in"
        );
        assert!(verify_first_login_cleared_the_fixture(&clean).is_ok());

        let foreign = CreatedCharacterRow {
            account: 9,
            ..fresh.clone()
        };
        assert!(verify_created_character_row(&foreign, &bot, &options).is_err());

        let recustomize = CreatedCharacterRow {
            at_login: AT_LOGIN_FIRST_LIKE_CPP | 8,
            ..fresh.clone()
        };
        assert!(verify_created_character_row(&recustomize, &bot, &options).is_err());

        let online = CreatedCharacterRow {
            online: 1,
            ..fresh.clone()
        };
        assert!(verify_created_character_row(&online, &bot, &options).is_err());

        let level_zero = CreatedCharacterRow {
            level: 0,
            ..fresh.clone()
        };
        assert!(verify_created_character_row(&level_zero, &bot, &options).is_err());

        let wrong_class = CreatedCharacterRow {
            class: 2,
            ..fresh.clone()
        };
        assert!(verify_created_character_row(&wrong_class, &bot, &options).is_err());
    }
}
