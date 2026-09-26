// Twitter para el cliente 7.3.5 (26972): proxy compatible.
//
// El cliente habla el mismo mismo la API v1.1 de Twitter con OAuth 1.0a (firma HMAC-SHA1 con la
// consumer key de Blizzard) y tiene las URLs parcheadas a nightspire.gg:
//   POST /oauth/request_token            -> oauth_token, oauth_token_secret, oauth_callback_confirmed
//   GET  /oauth/authorize?oauth_token=T  (navegador CEF del juego)
//   POST /oauth/access_token             -> oauth_token, oauth_token_secret, user_id, screen_name
//   GET  /1.1/help/configuration.json    -> short_url_length, short_url_length_https, characters_reserved_per_media
//   POST /1.1/statuses/update.json       (status=...)
//   POST /1.1/statuses/update_with_media.json (multipart: status, media[])
// y espera volver del navegador a https://twitter.social.wow.blizzard.net/callback?oauth_token=T&oauth_verifier=V
// (ese host va en auth.browser_url_map hacia /twitter de este servidor, como la tienda).
//
// La API de Twitter ya no existe; aqui se traduce todo a la API de X v2: OAuth 2.0 con PKCE
// (https://x.com/i/oauth2/authorize, https://api.x.com/2/oauth2/token), subida de medios v2 y
// POST /2/tweets. El vinculo con la cuenta de X se guarda por cuenta Battle.net en auth.twitter_link.
//
// No se verifica la firma OAuth 1.0a del cliente: el oauth_token que emitimos identifica la cuenta y
// todo viaja por HTTPS. La cuenta se conoce porque el worldserver crea el request token al recibir
// CMSG_TWITTER_CONNECT (auth.twitter_request con la IP del jugador); si el cliente pide su propio
// request_token por HTTP se le asocia a la ultima peticion del juego desde esa IP.
//
// Variables (shop.env): X_CLIENT_ID, X_CLIENT_SECRET (app OAuth 2.0 de developer.x.com, tipo
// confidencial), X_REDIRECT_URI (por defecto <TWITTER_PUBLIC_URL>/oauth/x-callback; tiene que estar
// dado de alta en la app), X_SCOPES, X_API, TWITTER_PUBLIC_URL (https://nightspire.gg),
// TWITTER_BLIZZARD_CALLBACK. Sin X_CLIENT_ID/SECRET el modulo contesta "no disponible".
//
// Ojo: X cobra por uso (publicar ~0,015 USD por peticion, mas la subida de medios).

import crypto from "node:crypto";

export function createTwitter({ db, cfg, env, log, esc, page, send, sendJson }) {
  const x = {
    clientId: env.X_CLIENT_ID || "",
    clientSecret: env.X_CLIENT_SECRET || "",
    api: (env.X_API || "https://api.x.com").replace(/\/+$/, ""),
    authorize: env.X_AUTHORIZE_URL || "https://x.com/i/oauth2/authorize",
    scopes: env.X_SCOPES || "tweet.read tweet.write users.read media.write offline.access",
    publicUrl: (env.TWITTER_PUBLIC_URL || "https://nightspire.gg").replace(/\/+$/, ""),
    blizzardCallback: env.TWITTER_BLIZZARD_CALLBACK || "https://twitter.social.wow.blizzard.net/callback",
  };
  x.redirectUri = env.X_REDIRECT_URI || `${x.publicUrl}/oauth/x-callback`;
  // TWITTER_EXTERNAL_BROWSER=0 para autorizar dentro del navegador del juego (Google/X suelen bloquearlo)
  const externalBrowser = String(env.TWITTER_EXTERNAL_BROWSER ?? "1") !== "0";
  const enabled = Boolean(x.clientId && x.clientSecret);
  const A = `\`${cfg.db.auth}\``;
  const TR = `${A}.twitter_request`;
  const TL = `${A}.twitter_link`;
  const hex = (n) => crypto.randomBytes(n).toString("hex");
  const b64url = (b) => Buffer.from(b).toString("base64url");
  const basic = () => "Basic " + Buffer.from(`${x.clientId}:${x.clientSecret}`).toString("base64");
  const now = () => Math.floor(Date.now() / 1000);
  // Detras de Cloudflare la IP real viene en cf-connecting-ip (x-forwarded-for trae la del borde de Cloudflare)
  const ipOf = (req) => String(req.headers["cf-connecting-ip"] || req.headers["x-forwarded-for"] || req.socket?.remoteAddress || "").split(",")[0].trim().replace(/^::ffff:/, "");

  log(`twitter: proxy ${enabled ? "activo" : "SIN CREDENCIALES (X_CLIENT_ID/X_CLIENT_SECRET)"}; publico ${x.publicUrl}, redirect ${x.redirectUri}`);

  // ---------------------------------------------------------------- utilidades HTTP
  function readRaw(req, limit = 6 * 1024 * 1024) {
    return new Promise((resolve, reject) => {
      const chunks = [];
      let size = 0;
      req.on("data", (c) => { size += c.length; if (size > limit) { reject(new Error("body too large")); req.destroy(); return; } chunks.push(c); });
      req.on("end", () => resolve(Buffer.concat(chunks)));
      req.on("error", reject);
    });
  }

  // Authorization: OAuth oauth_token="...", oauth_verifier="...", ...
  function oauthHeader(req) {
    const h = String(req.headers.authorization || "");
    const out = {};
    if (!/^OAuth\s/i.test(h)) return out;
    for (const m of h.slice(6).matchAll(/([a-z_]+)="([^"]*)"/g)) {
      try { out[m[1]] = decodeURIComponent(m[2]); } catch { out[m[1]] = m[2]; }
    }
    return out;
  }

  function parseMultipart(buf, contentType) {
    const m = /boundary="?([^";]+)"?/i.exec(contentType || "");
    if (!m) return [];
    const boundary = Buffer.from("--" + m[1]);
    const parts = [];
    let pos = buf.indexOf(boundary);
    while (pos !== -1) {
      let start = pos + boundary.length;
      if (buf.slice(start, start + 2).toString() === "--") break;
      if (buf.slice(start, start + 2).toString() === "\r\n") start += 2;
      const headEnd = buf.indexOf("\r\n\r\n", start);
      if (headEnd === -1) break;
      const head = buf.slice(start, headEnd).toString("utf8");
      const next = buf.indexOf(boundary, headEnd + 4);
      if (next === -1) break;
      let data = buf.slice(headEnd + 4, next);
      if (data.slice(-2).toString() === "\r\n") data = data.slice(0, -2);
      const name = /name="([^"]*)"/i.exec(head)?.[1] || "";
      const filename = /filename="([^"]*)"/i.exec(head)?.[1];
      const type = /content-type:\s*([^\r\n]+)/i.exec(head)?.[1]?.trim();
      parts.push({ name, filename, type, data });
      pos = next;
    }
    return parts;
  }

  const formBody = (obj) => Object.entries(obj).map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`).join("&");
  const sendForm = (res, code, obj) => send(res, code, formBody(obj), "application/x-www-form-urlencoded; charset=utf-8");
  const twitterError = (res, code, message, errCode = 89) => sendJson(res, code, { errors: [{ code: errCode, message }] });
  const twitterDate = (d = new Date()) => d.toUTCString().replace(/^(\w{3}), (\d{2}) (\w{3}) (\d{4}) ([\d:]+) GMT$/, "$1 $3 $2 $5 +0000 $4");

  // ---------------------------------------------------------------- API de X
  async function xFetch(path, { method = "GET", token, json, form, headers = {}, raw } = {}) {
    const h = { Accept: "application/json", ...headers };
    if (token) h.Authorization = `Bearer ${token}`;
    let body;
    if (json !== undefined) { h["Content-Type"] = "application/json"; body = JSON.stringify(json); }
    else if (form) body = form;
    else if (raw) body = raw;
    const r = await fetch(x.api + path, { method, headers: h, body, signal: AbortSignal.timeout(30000) });
    const text = await r.text();
    let data;
    try { data = JSON.parse(text); } catch { data = { raw: text }; }
    if (!r.ok) {
      const e = new Error(`X ${method} ${path} -> HTTP ${r.status}: ${text.slice(0, 400)}`);
      e.status = r.status;
      e.data = data;
      throw e;
    }
    return data;
  }

  async function tokenRequest(params) {
    return xFetch("/2/oauth2/token", { method: "POST", headers: { Authorization: basic(), "Content-Type": "application/x-www-form-urlencoded" }, form: formBody(params) });
  }

  async function saveTokens(bnetAccount, tok, extra = {}) {
    const expires = now() + Number(tok.expires_in || 7200);
    const cols = { access_token: tok.access_token, refresh_token: tok.refresh_token || null, expires_at: expires, ...extra };
    const keys = Object.keys(cols);
    await db.query(
      `INSERT INTO ${TL} (bnet_account, ${keys.join(", ")}) VALUES (?, ${keys.map(() => "?").join(", ")}) ON DUPLICATE KEY UPDATE ${keys.map((k) => `${k} = VALUES(${k})`).join(", ")}`,
      [bnetAccount, ...keys.map((k) => cols[k])],
    );
  }

  async function linkOf(bnetAccount) {
    const [rows] = await db.query(`SELECT * FROM ${TL} WHERE bnet_account = ?`, [bnetAccount]);
    return rows[0] || null;
  }

  // Token de acceso vigente; se renueva con el refresh token cuando caduca
  async function freshToken(link) {
    if (link.expires_at > now() + 60) return link.access_token;
    if (!link.refresh_token) throw Object.assign(new Error("sin refresh token"), { relink: true });
    const tok = await tokenRequest({ grant_type: "refresh_token", refresh_token: link.refresh_token, client_id: x.clientId });
    await saveTokens(link.bnet_account, tok);
    return tok.access_token;
  }

  async function uploadMedia(token, data, mime) {
    // Subida por trozos de la API v2: initialize -> append -> finalize
    let mediaId;
    try {
      const init = await xFetch("/2/media/upload/initialize", { method: "POST", token, json: { media_type: mime, total_bytes: data.length, media_category: "tweet_image" } });
      mediaId = String(init?.data?.id || init?.media_id_string || init?.id || "");
      if (!mediaId) throw new Error("initialize sin id");
      const chunk = 4 * 1024 * 1024;
      for (let i = 0, seg = 0; i < data.length; i += chunk, ++seg) {
        const fd = new FormData();
        fd.append("segment_index", String(seg));
        fd.append("media", new Blob([data.subarray(i, i + chunk)], { type: mime }), "media");
        await xFetch(`/2/media/upload/${mediaId}/append`, { method: "POST", token, raw: fd });
      }
      const fin = await xFetch(`/2/media/upload/${mediaId}/finalize`, { method: "POST", token });
      const state = fin?.data?.processing_info?.state;
      for (let n = 0; state && state !== "succeeded" && n < 10; ++n) {
        await new Promise((r) => setTimeout(r, 1000));
        const st = await xFetch(`/2/media/upload?command=STATUS&media_id=${mediaId}`, { token });
        if (!st?.data?.processing_info || st.data.processing_info.state === "succeeded") break;
        if (st.data.processing_info.state === "failed") throw new Error("procesado de la imagen fallido");
      }
      return mediaId;
    } catch (e) {
      if (e.status !== 404 || mediaId) throw e;
      // Subida simple (una sola peticion) por si la app no tiene la subida por trozos
      const fd = new FormData();
      fd.append("media_category", "tweet_image");
      fd.append("media", new Blob([data], { type: mime }), "media");
      const r = await xFetch("/2/media/upload", { method: "POST", token, raw: fd });
      return String(r?.data?.id || r?.media_id_string || "");
    }
  }

  // ---------------------------------------------------------------- vinculo por cuenta
  async function requestByToken(token) {
    // El cliente pega el token tal cual en la URL; por si arrastra algun caracter extra se toma el tramo hexadecimal
    const m = /[0-9a-f]{16,64}/.exec(String(token || ""));
    if (!m) return null;
    const [rows] = await db.query(`SELECT * FROM ${TR} WHERE oauth_token = ?`, [m[0]]);
    return rows[0] || null;
  }

  async function lastGameRequestFromIp(ip) {
    const [rows] = await db.query(`SELECT bnet_account, game_account FROM ${TR} WHERE ip = ? AND bnet_account IS NOT NULL AND created > NOW() - INTERVAL 15 MINUTE ORDER BY id DESC LIMIT 1`, [ip]);
    return rows[0] || null;
  }

  async function completeLink(reqRow, bnetAccount) {
    const verifier = hex(16);
    await db.query(`UPDATE ${TR} SET linked = 1, oauth_verifier = ?, bnet_account = ?, linked_at = NOW() WHERE oauth_token = ?`, [verifier, bnetAccount, reqRow.oauth_token]);
    return verifier;
  }

  const landing = (title, msg, ok = true) => page("Twitter", `<div class="card"><h1>${esc(title)}</h1><div class="msg${ok ? "" : " bad"}">${esc(msg)}</div></div>`);

  // ---------------------------------------------------------------- rutas
  async function requestToken(req, res) {
    const ip = ipOf(req);
    const game = await lastGameRequestFromIp(ip);
    const token = hex(20), secret = hex(20);
    await db.query(`INSERT INTO ${TR} (oauth_token, oauth_token_secret, bnet_account, game_account, ip, source) VALUES (?, ?, ?, ?, ?, 'http')`, [token, secret, game?.bnet_account || null, game?.game_account || null, ip]);
    log(`twitter: request_token desde ${ip} -> ${token.slice(0, 8)}... (cuenta ${game?.bnet_account || "desconocida"})`);
    return sendForm(res, 200, { oauth_token: token, oauth_token_secret: secret, oauth_callback_confirmed: "true" });
  }

  async function authorize(req, res, q) {
    log(`twitter: authorize desde ${ipOf(req)}: ${String(req.url || "").slice(0, 300)}`);
    const row = await requestByToken(q.oauth_token);
    if (!row) return send(res, 404, landing("Twitter", "Solicitud de vinculacion no encontrada. Vuelve a intentarlo desde el juego.", false));
    if (!enabled) return send(res, 503, landing("Twitter", "La vinculacion con X no esta configurada en el servidor.", false));

    let bnet = row.bnet_account;
    if (!bnet) {
      const game = await lastGameRequestFromIp(ipOf(req));
      bnet = game?.bnet_account || null;
    }
    if (!bnet) return send(res, 403, landing("Twitter", "No se sabe a que cuenta pertenece esta vinculacion: abre la ventana desde el boton de Twitter del juego.", false));

    // Ya enlazada y con el token de X vigente: no hace falta volver a pasar por X
    const link = await linkOf(bnet);
    if (link && link.refresh_token) {
      try {
        await freshToken(link);
        const verifier = await completeLink(row, bnet);
        log(`twitter: cuenta ${bnet} ya enlazada a @${link.screen_name}; se reutiliza`);
        res.writeHead(302, { Location: `${x.blizzardCallback}?oauth_token=${encodeURIComponent(row.oauth_token)}&oauth_verifier=${encodeURIComponent(verifier)}` });
        return res.end();
      } catch (e) { log(`twitter: el token de la cuenta ${bnet} no se pudo renovar (${e.message}); se vuelve a pedir autorizacion`); }
    }

    const fromGame = externalBrowser && !!req.headers["x-game-client"];
    if (fromGame && row.denied) {
      res.writeHead(302, { Location: `${x.blizzardCallback}?denied=${encodeURIComponent(row.oauth_token)}` });
      return res.end();
    }
    // Recarga de la pagina de espera: NO se toca el PKCE en curso (si se regenerara, X rechazaria el
    // canje con "code verifier did not match"); solo se reconstruye la URL de X con el mismo reto.
    if (fromGame && q.wait && row.browser === "external" && row.code_verifier) {
      const uw = xAuthorizeUrl(row.oauth_token, b64url(crypto.createHash("sha256").update(row.code_verifier).digest()));
      return send(res, 200, waitingPage(req, uw.toString(), false));
    }

    const verifier = b64url(crypto.randomBytes(48));
    const challenge = b64url(crypto.createHash("sha256").update(verifier).digest());
    await db.query(`UPDATE ${TR} SET bnet_account = ?, code_verifier = ? WHERE oauth_token = ?`, [bnet, verifier, row.oauth_token]);
    const u = xAuthorizeUrl(row.oauth_token, challenge);

    // Desde el navegador del juego (manda X-GAME-CLIENT) la autorizacion se hace en el navegador de
    // Windows del jugador: Google y X bloquean el login en navegadores embebidos ("el navegador o la
    // aplicacion no son seguros") y ahi el jugador ya suele tener la sesion abierta. La pagina del juego
    // abre el enlace (window.open lo manda el proxy al navegador del sistema), sondea /oauth/x-status y
    // cuando la tienda recibe el codigo de X salta al callback de Blizzard, que el juego captura.
    if (fromGame) {
      await db.query(`UPDATE ${TR} SET browser = 'external' WHERE oauth_token = ?`, [row.oauth_token]);
      return send(res, 200, waitingPage(req, u.toString(), true));
    }
    res.writeHead(302, { Location: u.toString() });
    return res.end();
  }

  function xAuthorizeUrl(state, challenge) {
    const u = new URL(x.authorize);
    u.searchParams.set("response_type", "code");
    u.searchParams.set("client_id", x.clientId);
    u.searchParams.set("redirect_uri", x.redirectUri);
    u.searchParams.set("scope", x.scopes);
    u.searchParams.set("state", state);
    u.searchParams.set("code_challenge", challenge);
    u.searchParams.set("code_challenge_method", "S256");
    return u;
  }

  function waitingPage(req, xUrl, autoOpen) {
    // La pagina se refresca sola cada 3 s (navegacion de la propia pagina, la normal para el navegador
    // embebido; la que lo tira es la que ordena el juego). Cuando la tienda ve el enlace hecho, esta misma
    // ruta responde con la redireccion al callback de Blizzard, que es lo que el cliente espera leer.
    const qs = new URLSearchParams((req.url || "").split("?")[1] || "");
    qs.set("wait", "1");
    const refresh = "?" + qs.toString();
    return page("Twitter", `<div class="card"><h1>Vincular con X</h1>
<div class="msg">${autoOpen ? "Se abre la autorizacion de X en tu navegador de Windows." : "Esperando a que autorices en tu navegador de Windows."}
Inicia sesion alli y acepta: esta ventana continuara sola.</div>
<div class="row"><span></span><button onclick="abrir()">Abrir el navegador</button></div></div>
<meta http-equiv="refresh" content="3;url=${esc(refresh)}">
<script>
var xUrl = ${JSON.stringify(xUrl || "")};
function abrir(){ if (xUrl) { try { window.open(xUrl, '_blank'); } catch (e) {} } }
${autoOpen ? "setTimeout(abrir, 300);" : ""}
</script>`);
  }

  async function xStatus(req, res, q) {
    const row = await requestByToken(q.oauth_token);
    if (!row) return sendJson(res, 404, { linked: false, denied: true });
    return sendJson(res, 200, { linked: !!row.linked, verifier: row.linked ? row.oauth_verifier : "", denied: !!row.denied });
  }

  async function xCallback(req, res, q) {
    const row = await requestByToken(q.state);
    if (!row) return send(res, 404, landing("Twitter", "Solicitud de vinculacion no encontrada.", false));
    if (q.error || !q.code) {
      log(`twitter: X devolvio ${q.error || "sin code"} para ${row.oauth_token.slice(0, 8)}...`);
      await db.query(`UPDATE ${TR} SET denied = 1 WHERE oauth_token = ?`, [row.oauth_token]);
      if (row.browser === "external") return send(res, 200, landing("Twitter", "Has cancelado la autorizacion. Puedes cerrar esta pestana y volver al juego.", false));
      res.writeHead(302, { Location: `${x.blizzardCallback}?denied=${encodeURIComponent(row.oauth_token)}` });
      return res.end();
    }
    try {
      const tok = await tokenRequest({ grant_type: "authorization_code", code: q.code, client_id: x.clientId, redirect_uri: x.redirectUri, code_verifier: row.code_verifier || "" });
      const me = await xFetch("/2/users/me", { token: tok.access_token });
      const user = me?.data || {};
      const existing = await linkOf(row.bnet_account);
      await saveTokens(row.bnet_account, tok, {
        x_user_id: String(user.id || ""),
        screen_name: String(user.username || ""),
        oauth_token: existing?.oauth_token || hex(20),
        oauth_token_secret: existing?.oauth_token_secret || hex(20),
        linked_at: new Date(),
      });
      const verifier = await completeLink(row, row.bnet_account);
      log(`twitter: cuenta ${row.bnet_account} enlazada a @${user.username} (${user.id})${row.browser === "external" ? " (navegador externo)" : ""}`);
      if (row.browser === "external") return send(res, 200, landing("Twitter", `Cuenta @${user.username || ""} enlazada. Puedes cerrar esta pestana y volver al juego.`));
      res.writeHead(302, { Location: `${x.blizzardCallback}?oauth_token=${encodeURIComponent(row.oauth_token)}&oauth_verifier=${encodeURIComponent(verifier)}` });
      return res.end();
    } catch (e) {
      log(`twitter: fallo al canjear el codigo de X:`, e.message || e);
      return send(res, 409, landing("Twitter", "X no ha aceptado la autorizacion. Cierra esta ventana y vuelve a intentarlo desde el juego.", false));
    }
  }

  async function accessToken(req, res) {
    const body = (await readRaw(req, 65536)).toString("utf8");
    const form = new URLSearchParams(body);
    const h = oauthHeader(req);
    const token = h.oauth_token || form.get("oauth_token") || "";
    const verifier = h.oauth_verifier || form.get("oauth_verifier") || "";
    const row = await requestByToken(token);
    if (!row || !row.linked || !verifier || verifier !== row.oauth_verifier) return twitterError(res, 401, "Invalid or expired token", 89);
    const link = await linkOf(row.bnet_account);
    if (!link) return twitterError(res, 401, "Account not linked", 89);
    await db.query(`UPDATE ${TR} SET used = 1 WHERE oauth_token = ?`, [token]);
    log(`twitter: access_token entregado a la cuenta ${row.bnet_account} (@${link.screen_name})`);
    return sendForm(res, 200, { oauth_token: link.oauth_token, oauth_token_secret: link.oauth_token_secret, user_id: link.x_user_id || "0", screen_name: link.screen_name || "" });
  }

  async function linkFromRequest(req) {
    const h = oauthHeader(req);
    if (h.oauth_token && /^[0-9a-f]{16,64}$/.test(h.oauth_token)) {
      const [rows] = await db.query(`SELECT * FROM ${TL} WHERE oauth_token = ?`, [h.oauth_token]);
      if (rows[0]) return rows[0];
    }
    // Sin token reconocido (el cliente no completo el access_token): ultima vinculacion desde esta IP
    const game = await lastGameRequestFromIp(ipOf(req));
    if (game) {
      const link = await linkOf(game.bnet_account);
      if (link) { log(`twitter: publicacion sin oauth_token valido; se usa la cuenta ${game.bnet_account} por IP ${ipOf(req)}`); return link; }
    }
    return null;
  }

  async function statusUpdate(req, res, withMedia) {
    if (!enabled) return twitterError(res, 503, "Twitter integration not configured", 131);
    const link = await linkFromRequest(req);
    if (!link) return twitterError(res, 401, "Invalid or expired token", 89);

    const raw = await readRaw(req);
    const ctype = String(req.headers["content-type"] || "");
    let status = "";
    const media = [];
    if (/multipart\/form-data/i.test(ctype)) {
      for (const part of parseMultipart(raw, ctype)) {
        if (part.name === "status") status = part.data.toString("utf8");
        else if (part.name === "media[]" || part.name === "media" || part.name === "media_data") media.push(part);
      }
    } else {
      const form = new URLSearchParams(raw.toString("utf8"));
      status = form.get("status") || "";
      const dataB64 = form.get("media_data");
      if (dataB64) media.push({ data: Buffer.from(dataB64, "base64"), type: "image/jpeg" });
    }
    status = status.slice(0, 280);
    if (!status && !media.length) return twitterError(res, 403, "Status is a duplicate or empty", 170);

    try {
      let token = await freshToken(link);
      const mediaIds = [];
      for (const m of media.slice(0, 4)) {
        const mime = m.type && /^image\//i.test(m.type) ? m.type : (m.data[0] === 0x89 ? "image/png" : "image/jpeg");
        mediaIds.push(await uploadMedia(token, m.data, mime));
      }
      const body = { text: status };
      if (mediaIds.length) body.media = { media_ids: mediaIds };
      const tweet = await xFetch("/2/tweets", { method: "POST", token, json: body });
      const id = String(tweet?.data?.id || "");
      await db.query(`UPDATE ${TL} SET last_post = NOW(), posts = posts + 1 WHERE bnet_account = ?`, [link.bnet_account]);
      log(`twitter: @${link.screen_name} (cuenta ${link.bnet_account}) publico ${id}${mediaIds.length ? ` con ${mediaIds.length} imagen(es)` : ""}: ${status.slice(0, 80)}`);
      return sendJson(res, 200, {
        id: Number(id) || 0, id_str: id, text: status, created_at: twitterDate(),
        user: { id: Number(link.x_user_id) || 0, id_str: String(link.x_user_id || ""), screen_name: link.screen_name || "", name: link.screen_name || "" },
        entities: { media: mediaIds.map((m) => ({ id_str: m })) },
        source: "World of Warcraft",
      });
    } catch (e) {
      log(`twitter: fallo publicando para la cuenta ${link.bnet_account}:`, e.message || e);
      if (e.relink || e.status === 401) return twitterError(res, 401, "Invalid or expired token", 89);
      if (e.status === 429) return twitterError(res, 429, "Rate limit exceeded", 88);
      return twitterError(res, 403, "Could not post to X", 131);
    }
  }

  async function handle(req, res, p, q) {
    if (p === "/oauth/request_token" && req.method === "POST") return requestToken(req, res);
    if (p === "/oauth/authorize" && req.method === "GET") return authorize(req, res, q);
    if (p === "/oauth/x-callback" && req.method === "GET") return xCallback(req, res, q);
    if (p === "/oauth/x-status" && req.method === "GET") return xStatus(req, res, q);
    if (p === "/oauth/access_token" && req.method === "POST") return accessToken(req, res);
    if (p === "/1.1/help/configuration.json") return sendJson(res, 200, { characters_reserved_per_media: 24, short_url_length: 23, short_url_length_https: 23, max_media_per_upload: 4, photo_size_limit: 5242880, dm_text_character_limit: 10000 });
    if ((p === "/1.1/statuses/update.json" || p === "/1.1/statuses/update_with_media.json") && req.method === "POST") return statusUpdate(req, res, p.includes("with_media"));
    if (p === "/twitter/callback" || p === "/twitter" || p.startsWith("/twitter/")) {
      // Aterrizaje del callback de Blizzard reescrito por el shim CEF: el juego ya ha leido oauth_verifier de la URL
      if (q.denied) return send(res, 200, landing("Twitter", "Vinculacion cancelada. Puedes cerrar esta ventana.", false));
      return send(res, 200, landing("Twitter", "Cuenta de X enlazada. Ya puedes cerrar esta ventana."));
    }
    return twitterError(res, 404, "Sorry, that page does not exist", 34);
  }

  return { handle, enabled };
}
