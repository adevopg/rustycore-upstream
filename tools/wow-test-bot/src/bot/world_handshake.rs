//! The world-server connection and authentication handshake.
//!
//! Extracted verbatim from steps 2–6 of `run_bot_with_void_storage` so a second
//! workflow can reach an encrypted session without duplicating the AES-GCM
//! bring-up. The sequence, the timeouts and the logged text are unchanged; the
//! observed opcodes are returned to the caller instead of being pushed straight
//! into its `BotRunResult`.

use super::*;

/// An authenticated, encrypted world connection.
pub(crate) struct AuthenticatedWorldSessionLikeCpp {
    pub(crate) stream: TcpStream,
    pub(crate) crypt: WorldCrypt,
    /// C++ `WorldSession::_sessionKey` for this realm connection; the instance
    /// socket reuses it for CMSG_AUTH_CONTINUED_SESSION.
    pub(crate) derived_session_key: [u8; 40],
    /// Opcodes seen between SMSG_AUTH_RESPONSE and the encryption switch, in
    /// arrival order, formatted as the run report records them.
    pub(crate) seen_opcodes: Vec<String>,
}

pub(crate) async fn establish_encrypted_world_session_like_cpp(
    bot_index: usize,
    session_key: &[u8],
    wow_username: &str,
    win64_auth_seed: &[u8; 16],
) -> Result<AuthenticatedWorldSessionLikeCpp> {
    let mut seen_opcodes = Vec::new();

    // ── Step 2: Connect to World Server ─────────────────────────────────────
    info!(
        "[Bot {}] Step 2: Connecting to World Server {}:{}",
        bot_index,
        world_host(),
        world_port()
    );
    let world_addr = format!("{}:{}", world_host(), world_port());
    let mut stream =
        tokio::time::timeout(INITIAL_NETWORK_IO_TIMEOUT, TcpStream::connect(&world_addr))
            .await
            .map_err(|_| anyhow!("Timed out connecting to world server {world_addr}"))?
            .map_err(|e| anyhow!("Failed to connect to world server: {}", e))?;
    info!("[Bot {}] ✅ TCP connected", bot_index);

    // ── Step 3: World Server Handshake ──────────────────────────────────────
    info!("[Bot {}] Step 3: Handshake...", bot_index);
    let mut init_buf = vec![0u8; 256];
    let n = tokio::time::timeout(INITIAL_NETWORK_IO_TIMEOUT, stream.read(&mut init_buf))
        .await
        .map_err(|_| anyhow!("Timed out reading SERVER_INIT"))??;
    if !init_buf[..n].starts_with(&SERVER_INIT[..SERVER_INIT.len().min(n)]) {
        bail!(
            "Unexpected server init: {:?}",
            String::from_utf8_lossy(&init_buf[..n])
        );
    }
    info!("[Bot {}] ✅ SERVER_INIT received", bot_index);

    tokio::time::timeout(INITIAL_NETWORK_IO_TIMEOUT, async {
        stream.write_all(CLIENT_INIT).await?;
        stream.flush().await
    })
    .await
    .map_err(|_| anyhow!("Timed out writing CLIENT_INIT"))??;
    info!("[Bot {}] ✅ CLIENT_INIT sent", bot_index);

    // ── Step 4: Read SMSG_AUTH_CHALLENGE ────────────────────────────────────
    info!("[Bot {}] Step 4: Reading SMSG_AUTH_CHALLENGE...", bot_index);
    let (opcode, challenge_data) = tokio::time::timeout(
        INITIAL_NETWORK_IO_TIMEOUT,
        read_unencrypted_packet(&mut stream),
    )
    .await
    .map_err(|_| anyhow!("Timed out reading SMSG_AUTH_CHALLENGE"))??;
    if opcode != 0x3048 {
        bail!(
            "Expected SMSG_AUTH_CHALLENGE (0x3048), got 0x{:04X}",
            opcode
        );
    }
    if challenge_data.len() < 48 {
        bail!(
            "SMSG_AUTH_CHALLENGE too short: {} bytes",
            challenge_data.len()
        );
    }
    let server_challenge: [u8; 16] = challenge_data[32..48].try_into()?;
    info!("[Bot {}] ✅ SMSG_AUTH_CHALLENGE received", bot_index);

    // ── Step 5: Send CMSG_AUTH_SESSION ──────────────────────────────────────
    info!(
        "[Bot {}] Step 5: Sending CMSG_AUTH_SESSION (build={})...",
        bot_index,
        client_build()
    );
    let local_challenge: [u8; 16] = rand::random();
    let digest = compute_auth_digest(
        &local_challenge,
        &server_challenge,
        session_key,
        win64_auth_seed,
    );
    let derived_session_key =
        derive_realm_session_key(session_key, &local_challenge, &server_challenge);

    // RealmJoinTicket on the worldserver side is the WoW account name (account.username),
    // World auth uses the game-account username and `session_key_bnet`, not a
    // BNet login ticket (sending that ticket yields "unknown account").
    let auth_data = build_cmsg_auth_session(realm_id(), &local_challenge, &digest, wow_username);
    send_unencrypted_packet(&mut stream, 0x3765, &auth_data).await?;
    info!("[Bot {}] ✅ CMSG_AUTH_SESSION sent", bot_index);

    // ── Step 6: Wait for SMSG_AUTH_RESPONSE & encryption activation ─────────
    info!(
        "[Bot {}] Step 6: Waiting for auth response & encryption...",
        bot_index
    );
    let mut world_crypt: Option<WorldCrypt> = None;
    let mut encrypted = false;

    for _ in 0..10 {
        match tokio::time::timeout(Duration::from_secs(5), read_unencrypted_packet(&mut stream))
            .await
        {
            Ok(Ok((op, payload))) => {
                seen_opcodes.push(format!("0x{:04X}", op));
                let parsed = parse_packet(op, &payload);
                info!("[Bot {}] 📦 {}", bot_index, parsed);

                if op == 0x256D {
                    // SMSG_AUTH_RESPONSE
                    info!("[Bot {}] ✅ SMSG_AUTH_RESPONSE received", bot_index);
                } else if op == 0x3049 {
                    // SMSG_ENTER_ENCRYPTED_MODE
                    info!("[Bot {}] ✅ SMSG_ENTER_ENCRYPTED_MODE received", bot_index);

                    let enc_key =
                        derive_encryption_key(session_key, &local_challenge, &server_challenge);
                    info!(
                        "[Bot {}] Encryption key derived: {:02x}{:02x}...",
                        bot_index, enc_key[0], enc_key[1]
                    );

                    // Server's WorldPacketCrypt increments _clientCounter / _serverCounter
                    // on every packet — including the unencrypted SMSG_AUTH_CHALLENGE,
                    // SMSG_ENTER_ENCRYPTED_MODE, CMSG_AUTH_SESSION, and CMSG_ENTER_ENCRYPTED_MODE_ACK
                    // exchanges that happen before _authCrypt.Init() is called. By the
                    // time the first AES-GCM packet flies, both counters are at 2.
                    world_crypt = Some(WorldCrypt::new_with_counters(&enc_key, 2, 2));
                    encrypted = true;

                    // Send ACK
                    send_unencrypted_packet(&mut stream, 0x3767, &[]).await?;
                    info!("[Bot {}] ✅ CMSG_ENTER_ENCRYPTED_MODE_ACK sent", bot_index);
                    break;
                } else if op == 0x256E {
                    // SMSG_AUTH_RESPONSE (error variant)
                    warn!(
                        "[Bot {}] ⚠️ Auth response error code: {:?}",
                        bot_index,
                        payload.first()
                    );
                }
            }
            Ok(Err(e)) => {
                warn!("[Bot {}] Error reading packet: {}", bot_index, e);
                break;
            }
            Err(_) => {
                warn!("[Bot {}] Timeout waiting for encryption", bot_index);
                break;
            }
        }
    }

    if !encrypted {
        bail!("Encryption not established");
    }

    Ok(AuthenticatedWorldSessionLikeCpp {
        stream,
        crypt: world_crypt.take().expect("encryption established"),
        derived_session_key,
        seen_opcodes,
    })
}

/// Live BNet SRP6 plus the `account.session_key_bnet` / `account.os` write every
/// world login needs, shared by the workflows that open their own session.
pub(crate) async fn prepare_live_world_session_key_like_cpp(
    bot: &config::BotConfig,
) -> Result<(Vec<u8>, WorldAuthDbContext)> {
    let bnet_url = format!("https://{}:{}", bnet_host(), bnet_port());
    let (_login_ticket, session_key_32) =
        bot_srp6::authenticate_bot(&bnet_url, &bot.account, &bot.password)
            .await
            .map_err(|e| anyhow!("Bot SRP6 failed: {e}"))?;
    if session_key_32.len() != 32 {
        bail!(
            "Bot SRP6 returned K of unexpected length: {}",
            session_key_32.len()
        );
    }
    let session_key = expand_session_key(&session_key_32).to_vec();

    let account_for_db = bot.account.clone();
    let session_key_for_db = session_key.clone();
    let realm_id_for_db = realm_id();
    let context = tokio::task::spawn_blocking(move || {
        prepare_world_auth_context(&account_for_db, &session_key_for_db, realm_id_for_db)
    })
    .await
    .map_err(|e| anyhow!("DB worker join failed for {}: {e}", bot.account))?
    .map_err(|e| {
        anyhow!(
            "Failed to prepare world auth context for {}: {e}",
            bot.account
        )
    })?;
    Ok((session_key, context))
}

pub(crate) async fn expect_encrypted_opcode(
    connection: &mut EncryptedWorldConnection,
    wanted: u16,
    deadline: std::time::Instant,
    label: &str,
) -> Result<Vec<u8>> {
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            bail!("Timed out waiting for {label}");
        }
        let read = tokio::time::timeout(
            remaining.min(Duration::from_secs(3)),
            read_encrypted_packet(
                &mut connection.stream,
                &mut connection.crypt,
                &mut connection.inflater,
            ),
        )
        .await;
        match read {
            Ok(Ok((op, payload))) => {
                if op == wanted {
                    return Ok(payload);
                }
                debug!("while waiting for {label}: 0x{:04X}", op);
            }
            Ok(Err(e)) => bail!("Read error while waiting for {label}: {e}"),
            Err(_) => { /* keep waiting until the deadline */ }
        }
    }
}

/// `SMSG_CONNECT_TO` (3.4.3 `0x304D`): the login answer that redirects the
/// client to the instance socket. SMSG_LOGIN_VERIFY_WORLD arrives there, not on
/// the realm connection, so the wait has to follow the redirect.
pub(crate) const SMSG_CONNECT_TO: u16 = 0x304D;

pub(crate) async fn expect_login_opcode_across_connect_to(
    bot_index: usize,
    connection: &mut EncryptedWorldConnection,
    realm_connection: &mut Option<EncryptedWorldConnection>,
    derived_session_key: &[u8; 40],
    wanted: u16,
    deadline: std::time::Instant,
    label: &str,
    // The login burst carries the CREATE blocks of everything already in
    // visibility range. A caller that needs them — to find a creature's live
    // ObjectGuid, for instance — collects them here instead of losing them,
    // because they arrive before SMSG_LOGIN_VERIFY_WORLD and are not resent.
    mut update_objects: Option<&mut Vec<Vec<u8>>>,
) -> Result<Vec<u8>> {
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            bail!("Timed out waiting for {label}");
        }
        let read = tokio::time::timeout(
            remaining.min(Duration::from_secs(3)),
            read_encrypted_packet(
                &mut connection.stream,
                &mut connection.crypt,
                &mut connection.inflater,
            ),
        )
        .await;
        match read {
            Ok(Ok((op, payload))) => {
                if op == wanted {
                    return Ok(payload);
                }
                if op != SMSG_CONNECT_TO {
                    if op == SMSG_UPDATE_OBJECT {
                        if let Some(collected) = update_objects.as_deref_mut() {
                            collected.push(payload);
                        }
                    } else {
                        debug!("while waiting for {label}: 0x{:04X}", op);
                    }
                    continue;
                }
                let target = parse_connect_to(&payload)
                    .ok_or_else(|| anyhow!("Unable to parse SMSG_CONNECT_TO payload"))?;
                info!(
                    "[Bot {}] SMSG_CONNECT_TO: {}:{} serial={} con={} key={}",
                    bot_index,
                    target.address,
                    target.port,
                    target.serial,
                    target.connection_type,
                    target.key
                );
                if realm_connection.is_some() {
                    bail!("Received more than one SMSG_CONNECT_TO");
                }
                let (instance_stream, instance_crypt) =
                    connect_to_instance(bot_index, &target, derived_session_key).await?;
                // The server keeps cross-socket ordering fences alive after
                // SMSG_CONNECT_TO, so the realm socket stays open: closing it
                // makes the realm writer vanish before it acknowledges the
                // fence and the session is kicked.
                let realm_stream = std::mem::replace(&mut connection.stream, instance_stream);
                let realm_crypt = std::mem::replace(&mut connection.crypt, instance_crypt);
                let realm_inflater = std::mem::take(&mut connection.inflater);
                *realm_connection = Some(EncryptedWorldConnection {
                    stream: realm_stream,
                    crypt: realm_crypt,
                    inflater: realm_inflater,
                });
                info!("[Bot {}] ✅ Instance socket authenticated", bot_index);
            }
            Ok(Err(e)) => bail!("Read error while waiting for {label}: {e}"),
            Err(_) => { /* keep waiting until the deadline */ }
        }
    }
}
