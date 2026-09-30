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
