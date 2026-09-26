// Reclutar a un amigo (botón "Recruit A Friend" de la lista de amigos del cliente 7.3.5).
//
// El worldserver guarda cada invitación (CMSG_RECRUIT_A_FRIEND) en
// auth.battlenet_account_raf_invitations con un token aleatorio. Esta parte de la web:
//
//   1. envía por email el enlace "<RAF_PUBLIC_URL>/raf/<token>" a las invitaciones con status 0
//      (SMTP mínimo sin dependencias: RAF_SMTP_*; sin RAF_SMTP_HOST el enlace solo se escribe en el
//      log, útil para pruebas) y las marca status 1;
//   2. GET /raf/<token> muestra quién invita (BattleTag, reino, facción, nota) y el formulario de
//      registro: BattleTag deseado y contraseña;
//   3. POST /raf/<token> crea la cuenta Battle.net (salt/verifier SRP6 v1 como
//      Battlenet::AccountMgr::CreateBattlenetAccount) y la cuenta de juego "<id>#1" con
//      account.recruiter = cuenta de juego del reclutador y marca la invitación status 2; el
//      worldserver (Battlenet::FriendsMgr::Update) hace amigos de BattleTag a los dos.
//
// Estados: 0 email pendiente, 1 email enviado, 2 aceptada, 3 email fallido, 4 caducada.

import { createHash, randomBytes } from "node:crypto";
import net from "node:net";
import tls from "node:tls";

// Módulo v1 de SRP6.cpp (V1_N_HEX), g = 2, verifier de 128 bytes little-endian.
const SRP_N = BigInt("0x86A7F6DEEB306CE519770FE37D556F29944132554DED0BD68205E27F3231FEF5A10108238A3150C59CAF7B0B6478691C13A6ACF5E1B5ADAFD4A943D4A21A142B800E8A55F8BFBAC700EB77A7235EE5A609E350EA9FC19F10D921C2FA832E4461B7125D38D254A0BE873DFC27858ACB3F8B9F258461E4373BC3A6C2A9634324AB");
const MAX_PASS = 16;   // MAX_PASS_STR del core
const MIN_PASS = 8;
const ST = { Pending: 0, Sent: 1, Accepted: 2, MailFailed: 3, Expired: 4 };

function modPow(base, exp, mod) {
  let r = 1n;
  base %= mod;
  while (exp > 0n) {
    if (exp & 1n) r = (r * base) % mod;
    exp >>= 1n;
    base = (base * base) % mod;
  }
  return r;
}

// Utf8ToUpperOnlyLatin del core: solo ASCII a mayúsculas
const upperLatin = (s) => String(s).replace(/[a-z]/g, (c) => c.toUpperCase());

// Battlenet::AccountMgr / AccountMgr::CreateAccount: usuario SRP = hex mayúsculas de SHA256(login),
// x = SHA256(salt || SHA256(user ":" pass)), v = g^x mod N (little-endian, 128 bytes)
export function srpRegistrationData(login, password) {
  const user = createHash("sha256").update(upperLatin(login)).digest("hex").toUpperCase();
  const salt = randomBytes(32);
  const inner = createHash("sha256").update(`${user}:${upperLatin(password)}`).digest();
  const x = BigInt("0x" + createHash("sha256").update(Buffer.concat([salt, inner])).digest("hex"));
  const v = modPow(2n, x, SRP_N);
  const verifier = Buffer.alloc(128);
  let tmp = v;
  for (let i = 0; i < 128; i++) { verifier[i] = Number(tmp & 0xffn); tmp >>= 8n; }
  return { salt, verifier };
}

// ---------------------------------------------------------------------------------------------
// SMTP mínimo (EHLO, STARTTLS o TLS implícito, AUTH LOGIN/PLAIN, MAIL/RCPT/DATA)
function smtpSend(smtp, { from, to, subject, text, html }) {
  return new Promise((resolve, reject) => {
    const secure = smtp.secure || Number(smtp.port) === 465;
    let sock = secure ? tls.connect({ host: smtp.host, port: smtp.port, servername: smtp.host }) : net.connect({ host: smtp.host, port: smtp.port });
    let buf = "";
    let waiter = null;
    const fail = (e) => { try { sock.destroy(); } catch {} reject(e instanceof Error ? e : new Error(String(e))); };
    const timer = setTimeout(() => fail(new Error("SMTP: timeout")), 30000);

    const attach = () => {
      sock.setEncoding("utf8");
      sock.on("data", (d) => {
        buf += d;
        // respuesta completa: última línea "NNN texto" (sin guion)
        const lines = buf.split(/\r?\n/);
        for (let i = lines.length - 2; i >= 0; i--) {
          if (/^\d{3} /.test(lines[i])) {
            const reply = lines.slice(0, i + 1).join("\n");
            buf = lines.slice(i + 1).join("\n");
            if (waiter) { const w = waiter; waiter = null; w(reply); }
            return;
          }
        }
      });
      sock.on("error", fail);
    };
    attach();

    const read = () => new Promise((res) => { waiter = res; });
    const cmd = async (line, ok = /^[23]\d\d/) => {
      if (line !== null) sock.write(line + "\r\n");
      const reply = await read();
      if (!ok.test(reply)) throw new Error(`SMTP: ${line ? line.split(" ")[0] : "banner"} -> ${reply.split("\n").pop()}`);
      return reply;
    };

    (async () => {
      await cmd(null);
      let ehlo = await cmd(`EHLO ${smtp.helo}`);
      if (!secure && /STARTTLS/i.test(ehlo)) {
        await cmd("STARTTLS", /^220/);
        sock.removeAllListeners("data"); sock.removeAllListeners("error");
        sock = tls.connect({ socket: sock, servername: smtp.host });
        buf = "";
        attach();
        await new Promise((res, rej) => { sock.once("secureConnect", res); sock.once("error", rej); });
        ehlo = await cmd(`EHLO ${smtp.helo}`);
      }
      if (smtp.user) {
        if (/AUTH[^\n]*PLAIN/i.test(ehlo))
          await cmd(`AUTH PLAIN ${Buffer.from(`\0${smtp.user}\0${smtp.pass}`).toString("base64")}`, /^235/);
        else {
          await cmd("AUTH LOGIN", /^334/);
          await cmd(Buffer.from(smtp.user).toString("base64"), /^334/);
          await cmd(Buffer.from(smtp.pass).toString("base64"), /^235/);
        }
      }
      await cmd(`MAIL FROM:<${from.address}>`, /^250/);
      await cmd(`RCPT TO:<${to}>`, /^25\d/);
      await cmd("DATA", /^354/);
      const boundary = "=_raf_" + randomBytes(8).toString("hex");
      const enc = (s) => `=?UTF-8?B?${Buffer.from(s).toString("base64")}?=`;
      const body = [
        `From: ${from.name ? enc(from.name) + " " : ""}<${from.address}>`,
        `To: <${to}>`,
        `Subject: ${enc(subject)}`,
        `Date: ${new Date().toUTCString()}`,
        `Message-ID: <${randomBytes(12).toString("hex")}@${smtp.helo}>`,
        "MIME-Version: 1.0",
        `Content-Type: multipart/alternative; boundary="${boundary}"`,
        "",
        `--${boundary}`,
        "Content-Type: text/plain; charset=utf-8",
        "Content-Transfer-Encoding: base64",
        "",
        Buffer.from(text).toString("base64").replace(/.{76}/g, "$&\r\n"),
        `--${boundary}`,
        "Content-Type: text/html; charset=utf-8",
        "Content-Transfer-Encoding: base64",
        "",
        Buffer.from(html).toString("base64").replace(/.{76}/g, "$&\r\n"),
        `--${boundary}--`,
        "",
      ].join("\r\n").replace(/\r?\n\./g, "\r\n..");
      await cmd(body + "\r\n.", /^250/);
      try { await cmd("QUIT", /^221/); } catch {}
      clearTimeout(timer);
      sock.end();
      resolve();
    })().catch((e) => { clearTimeout(timer); fail(e); });
  });
}

// ---------------------------------------------------------------------------------------------
const I18N = {
  es: {
    title: "Reclutar a un amigo", invited: "%s te ha invitado a jugar", realm: "Reino", faction: "Facción", alliance: "Alianza", horde: "Horda",
    note: "Mensaje de tu amigo", intro: "Crea tu cuenta para unirte. Cuando entres al juego, %s aparecerá automáticamente en tu lista de amigos de Battle.net y vosotros dos tendréis los beneficios de Reclutar a un amigo si jugáis juntos.",
    email: "Correo electrónico (usuario de la cuenta)", battletag: "BattleTag", battletagHelp: "De 3 a 12 letras o números, empezando por una letra. Se le añadirá un número (#1234).",
    password: "Contraseña", passwordHelp: `De ${MIN_PASS} a ${MAX_PASS} caracteres (no distingue mayúsculas).`, confirm: "Repite la contraseña", create: "Crear cuenta",
    done: "¡Cuenta creada!", doneText: "Ya puedes entrar al juego con estos datos:", user: "Usuario", tag: "BattleTag", doneFriend: "En cuanto entres, %s estará en tu lista de amigos de Battle.net.",
    err: { not_found: "Esta invitación no existe.", used: "Esta invitación ya se ha utilizado.", expired: "Esta invitación ha caducado. Pide a tu amigo que te envíe otra.", email_taken: "Ya existe una cuenta con este correo.", bad_tag: "El BattleTag no es válido.", tag_taken: "Ese BattleTag ya está cogido, prueba otro.", bad_pass: `La contraseña debe tener entre ${MIN_PASS} y ${MAX_PASS} caracteres.`, mismatch: "Las contraseñas no coinciden.", generic: "Ha ocurrido un error. Inténtalo de nuevo." },
    mailSubject: "%s te ha invitado a jugar a World of Warcraft", mailHi: "¡Hola!", mailBody: "%s (%s) te ha invitado a jugar en %s con Reclutar a un amigo.", mailNote: "Mensaje de tu amigo:", mailLink: "Crea tu cuenta con este enlace:", mailExpire: "El enlace caduca en %d días.",
  },
  en: {
    title: "Recruit A Friend", invited: "%s invited you to play", realm: "Realm", faction: "Faction", alliance: "Alliance", horde: "Horde",
    note: "Message from your friend", intro: "Create your account to join. When you log in, %s will automatically be in your Battle.net friends list and you will both get the Recruit A Friend benefits when playing together.",
    email: "Email (account name)", battletag: "BattleTag", battletagHelp: "3 to 12 letters or digits, starting with a letter. A number (#1234) will be added.",
    password: "Password", passwordHelp: `${MIN_PASS} to ${MAX_PASS} characters (not case sensitive).`, confirm: "Repeat the password", create: "Create account",
    done: "Account created!", doneText: "You can now log in to the game with:", user: "Account", tag: "BattleTag", doneFriend: "As soon as you log in, %s will be in your Battle.net friends list.",
    err: { not_found: "This invitation does not exist.", used: "This invitation has already been used.", expired: "This invitation has expired. Ask your friend to send a new one.", email_taken: "An account with this email already exists.", bad_tag: "The BattleTag is not valid.", tag_taken: "That BattleTag is already taken, try another one.", bad_pass: `The password must be ${MIN_PASS} to ${MAX_PASS} characters long.`, mismatch: "The passwords do not match.", generic: "Something went wrong. Please try again." },
    mailSubject: "%s invited you to play World of Warcraft", mailHi: "Hi!", mailBody: "%s (%s) invited you to play on %s through Recruit A Friend.", mailNote: "Message from your friend:", mailLink: "Create your account with this link:", mailExpire: "The link expires in %d days.",
  },
};
const fmt = (s, ...a) => s.replace(/%[sd]/g, () => String(a.shift()));

export function createRaf({ db, cfg, log, esc, page, send, readBody }) {
  const env = process.env;
  const raf = {
    publicUrl: (env.RAF_PUBLIC_URL || cfg.publicUrl).replace(/\/+$/, ""),
    days: Math.max(1, Number(env.RAF_INVITE_DAYS || 30)),
    expansion: Number(env.RAF_EXPANSION || 6),
    lang: (env.RAF_MAIL_LANG || "es").toLowerCase().startsWith("es") ? "es" : "en",
    smtp: env.RAF_SMTP_HOST ? {
      host: env.RAF_SMTP_HOST, port: Number(env.RAF_SMTP_PORT || 587), secure: /^(1|true|yes)$/i.test(env.RAF_SMTP_SECURE || ""),
      user: env.RAF_SMTP_USER || "", pass: env.RAF_SMTP_PASS || "", helo: env.RAF_SMTP_HELO || new URL(cfg.publicUrl).hostname,
    } : null,
    from: { address: env.RAF_MAIL_FROM || (env.RAF_SMTP_USER || ""), name: env.RAF_MAIL_FROM_NAME || cfg.shopName },
  };
  const T = {
    inv: `\`${cfg.db.auth}\`.battlenet_account_raf_invitations`,
    bnet: `\`${cfg.db.auth}\`.battlenet_accounts`,
    account: `\`${cfg.db.auth}\`.account`,
  };
  const tr = (lang) => I18N[lang] || I18N.es;
  const langOf = (req, q) => (String(q.lang || "").toLowerCase().startsWith("en") ? "en" : "es");
  const mailAttempts = new Map();

  // ----------------------------------------------------------------------------------------
  // 1. envío de emails
  async function findInvitation(token) {
    if (!/^[0-9a-fA-F]{32}$/.test(token || "")) return null;
    const [rows] = await db.query(
      `SELECT i.*, b.battle_tag AS inviter_tag, (i.created < NOW() - INTERVAL ? DAY) AS old FROM ${T.inv} i LEFT JOIN ${T.bnet} b ON b.id = i.inviter_bnet_account WHERE i.token = ?`,
      [raf.days, token]);
    return rows[0] || null;
  }

  function mailFor(inv) {
    const t = tr(raf.lang);
    const who = inv.inviter_tag || inv.inviter_character || "?";
    const link = `${raf.publicUrl}/raf/${inv.token}?lang=${raf.lang}`;
    const subject = fmt(t.mailSubject, inv.inviter_character || who);
    const lines = [t.mailHi, "", fmt(t.mailBody, inv.inviter_character || who, who, inv.realm_name || "?")];
    if (inv.note) lines.push("", t.mailNote, `"${inv.note}"`);
    lines.push("", t.mailLink, link, "", fmt(t.mailExpire, raf.days));
    const text = lines.join("\n");
    const html = `<div style="font:15px/1.5 Arial,sans-serif;color:#222"><p>${esc(t.mailHi)}</p><p>${esc(fmt(t.mailBody, inv.inviter_character || who, who, inv.realm_name || "?"))}</p>`
      + (inv.note ? `<p>${esc(t.mailNote)}<br><i>“${esc(inv.note)}”</i></p>` : "")
      + `<p>${esc(t.mailLink)}<br><a href="${esc(link)}" style="display:inline-block;margin-top:8px;padding:10px 16px;background:#2b6cb0;color:#fff;text-decoration:none;border-radius:6px">${esc(t.title)}</a><br><small>${esc(link)}</small></p><p style="color:#666">${esc(fmt(t.mailExpire, raf.days))}</p></div>`;
    return { from: raf.from, to: inv.email, subject, text, html, link };
  }

  async function sendPendingMails() {
    let rows;
    try {
      [rows] = await db.query(`SELECT i.*, b.battle_tag AS inviter_tag FROM ${T.inv} i LEFT JOIN ${T.bnet} b ON b.id = i.inviter_bnet_account WHERE i.status = ? ORDER BY i.id LIMIT 20`, [ST.Pending]);
    } catch (e) { log("raf: error consultando invitaciones:", e.message || e); return; }
    for (const inv of rows) {
      const mail = mailFor(inv);
      if (!raf.smtp) {
        log(`raf: SMTP no configurado (RAF_SMTP_HOST); invitación ${inv.id} de ${inv.inviter_character} para ${inv.email}: ${mail.link}`);
        await db.query(`UPDATE ${T.inv} SET status = ?, sent = NOW() WHERE id = ? AND status = ?`, [ST.Sent, inv.id, ST.Pending]);
        continue;
      }
      try {
        await smtpSend(raf.smtp, mail);
        await db.query(`UPDATE ${T.inv} SET status = ?, sent = NOW() WHERE id = ? AND status = ?`, [ST.Sent, inv.id, ST.Pending]);
        mailAttempts.delete(inv.id);
        log(`raf: invitación ${inv.id} de ${inv.inviter_character} enviada a ${inv.email}`);
      } catch (e) {
        const n = (mailAttempts.get(inv.id) || 0) + 1;
        mailAttempts.set(inv.id, n);
        log(`raf: invitación ${inv.id} para ${inv.email}, intento ${n}:`, e.message || e);
        if (n >= 5) { await db.query(`UPDATE ${T.inv} SET status = ? WHERE id = ?`, [ST.MailFailed, inv.id]); mailAttempts.delete(inv.id); }
      }
    }
  }
  setTimeout(sendPendingMails, 5000);
  setInterval(sendPendingMails, 30 * 1000).unref();

  // ----------------------------------------------------------------------------------------
  // 1.b códigos de recuperación de contraseña pedidos desde el launcher
  // El bnetserver (POST /bnetserver/launcher/recover/) deja la fila con el código; aquí se manda
  // por correo y se borra el texto en claro, quedando solo el hash que valida el canje.
  const RECOVERY = `\`${cfg.db.auth}\`.battlenet_account_recovery`;
  const recoveryAttempts = new Map();

  function recoveryMail(row) {
    const es = (row.lang || raf.lang).toLowerCase().startsWith("es");
    const subject = es ? `Código para recuperar tu cuenta de ${cfg.shopName}` : `Your ${cfg.shopName} account recovery code`;
    const intro = es
      ? "Has pedido cambiar la contraseña de tu cuenta desde el launcher. Escribe este código en la ventana de recuperación:"
      : "You asked to change your account password from the launcher. Type this code in the recovery window:";
    const tail = es
      ? "El código caduca en 30 minutos. Si no has sido tú, no hagas nada: tu contraseña no cambia."
      : "The code expires in 30 minutes. If this was not you, do nothing: your password stays as it is.";
    const text = [es ? "¡Hola!" : "Hi!", "", intro, "", row.code, "", tail].join("\n");
    const html = `<div style="font:15px/1.5 Arial,sans-serif;color:#222"><p>${esc(es ? "¡Hola!" : "Hi!")}</p><p>${esc(intro)}</p>`
      + `<p style="font:700 26px/1.2 Consolas,monospace;letter-spacing:4px;background:#f2f4f7;padding:14px 18px;border-radius:8px;display:inline-block">${esc(row.code)}</p>`
      + `<p style="color:#666">${esc(tail)}</p></div>`;
    return { from: raf.from, to: row.email, subject, text, html };
  }

  async function sendPendingRecoveryMails() {
    let rows;
    try {
      [rows] = await db.query(`SELECT id, email, code, lang FROM ${RECOVERY} WHERE status = 0 AND code <> '' AND expires > UNIX_TIMESTAMP() ORDER BY id LIMIT 20`);
    } catch (e) { log("recover: error consultando códigos:", e.message || e); return; }

    for (const row of rows) {
      const mail = recoveryMail(row);
      if (!raf.smtp) {
        // Sin SMTP configurado el código va al log del servicio, como los enlaces de RAF.
        log(`recover: SMTP no configurado (RAF_SMTP_HOST); código para ${row.email}: ${row.code}`);
        await db.query(`UPDATE ${RECOVERY} SET status = 1, code = '' WHERE id = ? AND status = 0`, [row.id]);
        continue;
      }
      try {
        await smtpSend(raf.smtp, mail);
        await db.query(`UPDATE ${RECOVERY} SET status = 1, code = '' WHERE id = ? AND status = 0`, [row.id]);
        recoveryAttempts.delete(row.id);
        log(`recover: código enviado a ${row.email}`);
      } catch (e) {
        const n = (recoveryAttempts.get(row.id) || 0) + 1;
        recoveryAttempts.set(row.id, n);
        log(`recover: código para ${row.email}, intento ${n}:`, e.message || e);
        if (n >= 5) { await db.query(`UPDATE ${RECOVERY} SET status = 3, code = '' WHERE id = ?`, [row.id]); recoveryAttempts.delete(row.id); }
      }
    }
  }
  setTimeout(sendPendingRecoveryMails, 5000);
  setInterval(sendPendingRecoveryMails, 15 * 1000).unref();

  // ----------------------------------------------------------------------------------------
  // 2. página
  const errorPage = (lang, key) => page(tr(lang).title, `<div class="card"><h1>${esc(tr(lang).title)}</h1><div class="msg bad">${esc(tr(lang).err[key] || tr(lang).err.generic)}</div></div>`);

  function formPage(lang, inv, values = {}, error = "") {
    const t = tr(lang);
    const who = inv.inviter_tag || inv.inviter_character;
    return page(t.title, `<div class="card"><h1>${esc(t.title)}</h1>
<p><b>${esc(fmt(t.invited, inv.inviter_character ? `${inv.inviter_character} (${who})` : who))}</b></p>
<div class="box">${esc(t.realm)}: <b>${esc(inv.realm_name || "?")}</b> · ${esc(t.faction)}: <b>${esc(Number(inv.faction) === 1 ? t.horde : t.alliance)}</b>${inv.note ? `<br><br>${esc(t.note)}:<br><i>“${esc(inv.note)}”</i>` : ""}</div>
<p>${esc(fmt(t.intro, who))}</p>
${error ? `<div class="msg bad">${esc(t.err[error] || t.err.generic)}</div>` : ""}
<form method="post" action="/raf/${esc(inv.token)}?lang=${lang}" autocomplete="off">
<label>${esc(t.email)}<input type="email" value="${esc(inv.email)}" readonly></label>
<label>${esc(t.battletag)}<input name="battletag" maxlength="12" pattern="[A-Za-z][A-Za-z0-9]{2,11}" required value="${esc(values.battletag || "")}"><small>${esc(t.battletagHelp)}</small></label>
<label>${esc(t.password)}<input type="password" name="password" minlength="${MIN_PASS}" maxlength="${MAX_PASS}" required><small>${esc(t.passwordHelp)}</small></label>
<label>${esc(t.confirm)}<input type="password" name="confirm" minlength="${MIN_PASS}" maxlength="${MAX_PASS}" required></label>
<div class="row"><span></span><button type="submit">${esc(t.create)}</button></div>
</form></div>`);
  }

  function donePage(lang, inv, email, battleTag) {
    const t = tr(lang);
    return page(t.title, `<div class="card"><h1>${esc(t.title)}</h1><div class="msg ok"><b>${esc(t.done)}</b></div>
<p>${esc(t.doneText)}</p><div class="box">${esc(t.user)}: <b>${esc(email)}</b><br>${esc(t.tag)}: <b>${esc(battleTag)}</b></div>
<p>${esc(fmt(t.doneFriend, inv.inviter_tag || inv.inviter_character))}</p></div>`);
  }

  // ----------------------------------------------------------------------------------------
  // 3. registro
  async function register(inv, battletagName, password) {
    const email = upperLatin(inv.email.trim());
    const [dup] = await db.query(`SELECT id FROM ${T.bnet} WHERE email = ? LIMIT 1`, [email]);
    if (dup.length) return { error: "email_taken" };

    // BattleTag como LoginRESTService::GenerateBattleTag: Nombre#dddd único
    const name = battletagName[0].toUpperCase() + battletagName.slice(1).toLowerCase();
    let battleTag = "";
    for (let attempt = 0; attempt < 20 && !battleTag; attempt++) {
      const candidate = `${name}#${1000 + Math.floor(Math.random() * 9000)}`;
      const [exists] = await db.query(`SELECT 1 FROM ${T.bnet} WHERE battle_tag = ? LIMIT 1`, [candidate]);
      if (!exists.length) battleTag = candidate;
    }
    if (!battleTag) return { error: "tag_taken" };

    const conn = await db.getConnection();
    try {
      await conn.beginTransaction();
      const bnetSrp = srpRegistrationData(email, password);
      const [r1] = await conn.query(
        `INSERT INTO ${T.bnet} (email, battle_tag, salt, verifier, srp_version, joindate, recruiter) VALUES (?, ?, ?, ?, 1, NOW(), ?)`,
        [email, battleTag, bnetSrp.salt, bnetSrp.verifier, inv.inviter_bnet_account]);
      const bnetId = r1.insertId;
      const username = `${bnetId}#1`;
      const gameSrp = srpRegistrationData(username, password);
      const [r2] = await conn.query(
        `INSERT INTO ${T.account} (username, salt, verifier, srp_version, email, battlenet_account, battlenet_index, expansion, recruiter, joindate) VALUES (?, ?, ?, 1, ?, ?, 1, ?, ?, NOW())`,
        [username, gameSrp.salt, gameSrp.verifier, email, bnetId, raf.expansion, inv.inviter_account]);
      const [r3] = await conn.query(
        `UPDATE ${T.inv} SET status = ?, invitee_bnet_account = ?, invitee_account = ?, accepted = NOW() WHERE id = ? AND status IN (?, ?)`,
        [ST.Accepted, bnetId, r2.insertId, inv.id, ST.Pending, ST.Sent]);
      if (!r3.affectedRows) throw new Error("used");
      await conn.commit();
      log(`raf: invitación ${inv.id} aceptada: cuenta bnet ${bnetId} (${email}, ${battleTag}) / juego ${r2.insertId} (${username}), reclutador ${inv.inviter_account}`);
      return { email, battleTag };
    } catch (e) {
      await conn.rollback().catch(() => {});
      if (e.message === "used") return { error: "used" };
      if (e.code === "ER_DUP_ENTRY") return { error: /battle_tag/i.test(e.message) ? "tag_taken" : "email_taken" };
      throw e;
    } finally {
      conn.release();
    }
  }

  return async function handleRaf(req, res, p, q) {
    const lang = langOf(req, q);
    const m = p.match(/^\/raf\/([0-9a-fA-F]{32})$/);
    if (!m) return send(res, 404, errorPage(lang, "not_found"));
    const inv = await findInvitation(m[1]);
    if (!inv) return send(res, 404, errorPage(lang, "not_found"));
    if (inv.status === ST.Accepted) return send(res, 410, errorPage(lang, "used"));
    if (inv.status === ST.Expired || inv.old) {
      if (inv.status !== ST.Expired) await db.query(`UPDATE ${T.inv} SET status = ? WHERE id = ? AND status IN (?, ?, ?)`, [ST.Expired, inv.id, ST.Pending, ST.Sent, ST.MailFailed]);
      return send(res, 410, errorPage(lang, "expired"));
    }

    if (req.method === "GET") return send(res, 200, formPage(lang, inv));
    if (req.method !== "POST") return send(res, 405, errorPage(lang, "generic"));

    const form = Object.fromEntries(new URLSearchParams(await readBody(req)));
    const battletag = String(form.battletag || "").trim();
    const password = String(form.password || "");
    if (!/^[A-Za-z][A-Za-z0-9]{2,11}$/.test(battletag)) return send(res, 400, formPage(lang, inv, form, "bad_tag"));
    if (password.length < MIN_PASS || password.length > MAX_PASS) return send(res, 400, formPage(lang, inv, form, "bad_pass"));
    if (password !== String(form.confirm || "")) return send(res, 400, formPage(lang, inv, form, "mismatch"));

    const r = await register(inv, battletag, password);
    if (r.error) return send(res, r.error === "used" ? 410 : 400, r.error === "used" ? errorPage(lang, "used") : formPage(lang, inv, form, r.error));
    send(res, 200, donePage(lang, inv, r.email, r.battleTag));
  };
}
