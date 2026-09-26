// Soporte in-game (Customer Support del cliente 7.3.5): base de conocimiento, abrir ticket y
// estado del ticket. El cliente abre https://<region>.battle.net/support/<lang>/{games/wow,
// ticket/submit,ticket/status} (VISITABLE_URL5/6/7) TAL CUAL, sin SSO previo. Todo llega aquí
// por auth.browser_url_map (*.battle.net -> esta web).
//
// SSO (ingeniería inversa de Wow-64.exe 26972 + WowBrowserProxy.exe): el cliente no detecta
// ninguna URL de login; lo dispara una cabecera HTTP en una respuesta 200 que ve el proxy CEF:
//   1. página protegida sin cookie -> 302 a /login/?ref=<URL completa>&app=support (como Blizzard)
//   2. GET /login/ -> 200 + "X-BNET-Authenticate: BattlenetToken server-salt="..."" -> el proxy manda
//      SSOSalt(url) al cliente -> CMSG_GENERATE_SSO_TOKEN (kind 0) al worldserver
//   3. el cliente carga <scheme>://<host>/login/sso?token=<tok>&<query de la URL del paso 2>
//      (la RUTA se descarta, solo se conserva la query -> por eso hace falta ref=) con cabecera
//      X-GAME-CLIENT: WoW/7.3.5.26972
//   4. /login/sso valida el token, deja la cookie wowsso y redirige (302) a ref. Blizzard contestaba
//      200 + "Authentication-Info: BattlenetToken server-proof" + Location (SSOProof); un 302 lo
//      sigue CEF sin pasar por el cliente y no deja estado pendiente, así que es más seguro.
//
// La web nunca escribe en characters.gm_tickets: encola acciones en gm_ticket_web_queue y el
// worldserver las aplica (TicketMgr::Update), así los ids y el estado en memoria son del core.

import { randomBytes } from "node:crypto";

const QUEUE = { Create: 0, Close: 1, Message: 2 };
const QUEUE_ERR = { 1: "no_character", 2: "ticket_not_found", 3: "too_many", 4: "disabled", 5: "bad_action" };
const MAX_OPEN = 3;
const COOKIE = "wowsso";

const I18N = {
  es: {
    support: "Soporte", kb: "Base de conocimiento", submit: "Abrir ticket", status: "Mis tickets", back: "Volver",
    title: "Asunto", body: "Describe el problema", send: "Enviar ticket", sending: "Enviando el ticket al servidor…",
    character: "Personaje", choose: "Elige el personaje con el que abres el ticket", none: "No tienes tickets.",
    open: "Abierto", answered: "Respondido", closed: "Cerrado", pending: "Pendiente", created: "Creado", updated: "Actualizado",
    reply: "Responder", replyPh: "Añade más información para el GM…", close: "Cerrar ticket", closeConfirm: "¿Cerrar este ticket?",
    gmResponse: "Respuesta del GM", yourMessage: "Tu mensaje", noResponse: "Todavía no hay respuesta de un Game Master. Te avisaremos en el juego.",
    waiting: "Un Game Master atenderá tu ticket lo antes posible.", queued: "Tu ticket se ha enviado. En unos segundos aparecerá en «Mis tickets».",
    tooMany: `Ya tienes ${MAX_OPEN} tickets abiertos. Cierra alguno antes de abrir otro.`, required: "Rellena el asunto y la descripción.",
    noSession: "No hay sesión. Abre esta página desde el juego (Menú → Ayuda) para identificarte.", noChar: "No se ha encontrado el personaje.",
    ssoWait: "Identificando con el cliente del juego… Si esta página no cambia sola, ábrela desde el juego (Menú → Ayuda).",
    ticket: "Ticket", faqTitle: "Preguntas frecuentes",
    bug: "Reportar un fallo", bugTitle: "¿Qué falla?", bugBody: "Cuéntalo con detalle", bugSend: "Enviar el fallo",
    bugIntro: "¿Algo del juego no funciona como debería? Cuéntalo aquí y queda apuntado en la lista de fallos del servidor para que se arregle.",
    bugNotTicket: "Esto no es un ticket: no lo atiende un Game Master. Si necesitas que alguien haga algo por ti —recuperar un objeto, sacarte de un sitio— abre un ticket.",
    bugPh: "Qué hacías, qué esperabas que pasara y qué pasó en su lugar. Si sabes cómo repetirlo, dilo: es lo que más ayuda.",
    bugOk: "Gracias. El fallo ha quedado apuntado como #%n y se revisará.",
    bugSee: "Ver el fallo", bugAnother: "Reportar otro fallo",
    bugLeft: "Puedes reportar %n fallos más hoy.",
    bugRate: "Has reportado ya bastantes fallos hoy. Vuelve mañana o amplía alguno de los que enviaste.",
    bugEmpty: "Rellena el resumen y la descripción.",
    bugFail: "No se ha podido enviar el fallo ahora mismo. Inténtalo más tarde.",
    bugOff: "Los reportes de fallos están desactivados en este momento.",
    bugMine: "Tus fallos", bugNoMine: "Todavía no has reportado ningún fallo.",
    bugListFail: "No se ha podido consultar tu lista de fallos ahora mismo.",
    bugOnGithub: "Verlo en GitHub", bugAnswers: "Respuestas", bugNoAnswers: "Todavía no ha contestado nadie.",
    bugYourText: "Lo que contaste", bugBackList: "Volver a tus fallos", bugNotFound: "Ese fallo no existe o no lo reportaste tú.",
    faq: [
      ["Mi personaje está atascado", "Escribe <code>/stuck</code> en el chat o usa la piedra de hogar. Si sigue atascado, abre un ticket indicando dónde estás."],
      ["He comprado algo en la tienda y no lo he recibido", "Vuelve a abrir la tienda del juego: los pedidos pagados se entregan al abrirla. Si en 10 minutos no lo tienes, abre un ticket con la fecha y el producto."],
      ["Quiero denunciar a un jugador", "Haz clic derecho sobre su nombre en el chat → Denunciar, o abre un ticket con su nombre, la hora y qué ocurrió."],
      ["He perdido un objeto o una misión no avanza", "Abre un ticket con el nombre del objeto o la misión y cuándo pasó; un Game Master lo revisará."],
    ],
    err: { no_character: "El personaje no pertenece a esta cuenta.", ticket_not_found: "El ticket ya no existe o está cerrado.", too_many: `Ya tienes ${MAX_OPEN} tickets abiertos.`, disabled: "El sistema de tickets está desactivado en este momento.", bad_action: "Acción no válida.", generic: "Ha ocurrido un error." },
  },
  en: {
    support: "Support", kb: "Knowledge base", submit: "Open a ticket", status: "My tickets", back: "Back",
    title: "Subject", body: "Describe the problem", send: "Send ticket", sending: "Sending the ticket to the server…",
    character: "Character", choose: "Choose the character opening the ticket", none: "You have no tickets.",
    open: "Open", answered: "Answered", closed: "Closed", pending: "Pending", created: "Created", updated: "Updated",
    reply: "Reply", replyPh: "Add more information for the GM…", close: "Close ticket", closeConfirm: "Close this ticket?",
    gmResponse: "GM response", yourMessage: "Your message", noResponse: "No Game Master has answered yet. You will be notified in game.",
    waiting: "A Game Master will handle your ticket as soon as possible.", queued: "Your ticket was sent. It will show up in “My tickets” in a few seconds.",
    tooMany: `You already have ${MAX_OPEN} open tickets. Close one before opening another.`, required: "Fill in the subject and the description.",
    noSession: "No session. Open this page from the game (Menu → Help) to identify yourself.", noChar: "Character not found.",
    ssoWait: "Signing in with the game client… If this page does not change by itself, open it from the game (Menu → Help).",
    ticket: "Ticket", faqTitle: "Frequently asked questions",
    bug: "Report a bug", bugTitle: "What is broken?", bugBody: "Tell us the details", bugSend: "Send the bug",
    bugIntro: "Something in the game not working as it should? Tell us here and it goes straight onto the server's bug list so it gets fixed.",
    bugNotTicket: "This is not a ticket: no Game Master handles it. If you need someone to do something for you — restore an item, get you unstuck — open a ticket instead.",
    bugPh: "What you were doing, what you expected and what happened instead. If you know how to reproduce it, say so: that helps most.",
    bugOk: "Thanks. The bug was filed as #%n and will be looked at.",
    bugSee: "See the bug", bugAnother: "Report another bug",
    bugLeft: "You can report %n more bugs today.",
    bugRate: "You have reported plenty of bugs today. Come back tomorrow, or add to one you already sent.",
    bugEmpty: "Fill in the summary and the description.",
    bugFail: "The bug could not be sent right now. Please try again later.",
    bugOff: "Bug reports are disabled at the moment.",
    bugMine: "Your bugs", bugNoMine: "You have not reported any bug yet.",
    bugListFail: "Your bug list could not be fetched right now.",
    bugOnGithub: "See it on GitHub", bugAnswers: "Replies", bugNoAnswers: "Nobody has replied yet.",
    bugYourText: "What you told us", bugBackList: "Back to your bugs", bugNotFound: "That bug does not exist, or you did not report it.",
    faq: [
      ["My character is stuck", "Type <code>/stuck</code> in chat or use your hearthstone. If it is still stuck, open a ticket saying where you are."],
      ["I bought something in the shop and did not receive it", "Open the in-game shop again: paid orders are delivered when it opens. If you still don't have it after 10 minutes, open a ticket with the date and the product."],
      ["I want to report a player", "Right-click their name in chat → Report, or open a ticket with their name, the time and what happened."],
      ["I lost an item or a quest does not progress", "Open a ticket with the item or quest name and when it happened; a Game Master will look into it."],
    ],
    err: { no_character: "The character does not belong to this account.", ticket_not_found: "The ticket no longer exists or is closed.", too_many: `You already have ${MAX_OPEN} open tickets.`, disabled: "The ticket system is currently disabled.", bad_action: "Invalid action.", generic: "Something went wrong." },
  },
};

export function createSupport({ db, cfg, log, esc, page, send, sendJson, readBody, bugs }) {
  const T = {
    token: `\`${cfg.db.auth}\`.battlenet_account_web_token`,
    chars: `\`${cfg.db.characters}\`.characters`,
    tickets: `\`${cfg.db.characters}\`.gm_tickets`,
    queue: `\`${cfg.db.characters}\`.gm_ticket_web_queue`,
  };

  const langOf = (l) => (String(l || "").toLowerCase().startsWith("en") ? "en" : "es");
  const tr = (lang) => I18N[lang] || I18N.es;
  const base = (lang) => `/support/${lang}`;
  const fmtDate = (unix, lang) => (unix ? new Date(Number(unix) * 1000).toLocaleString(lang === "es" ? "es-ES" : "en-GB") : "");
  const nl2br = (s) => esc(s).replace(/\r?\n/g, "<br>");
  const parseCookies = (req) => Object.fromEntries((req.headers.cookie || "").split(";").map((c) => c.trim().split("=")).filter((kv) => kv.length === 2).map(([k, v]) => [k, decodeURIComponent(v)]));
  const cookieHeader = (token) => `${COOKIE}=${encodeURIComponent(token)}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=86400`;
  const redirect = (res, to, extra = {}) => { res.writeHead(302, { Location: to, "Cache-Control": "no-store", ...extra }); res.end(); };

  // ------------------------------------------------------------------ sesión
  async function findToken(token) {
    if (!/^[0-9a-fA-F]{16,128}$/.test(token || "")) return null;
    const [rows] = await db.query(`SELECT token, battlenet_account, account, realm, character_guid, kind, (expires > NOW()) AS valid FROM ${T.token} WHERE token = ?`, [token]);
    return rows[0] || null;
  }

  async function findCharacter(account, guid) {
    if (!guid) return null;
    const [rows] = await db.query(`SELECT guid, name, level, map, zone, position_x, position_y, position_z, online FROM ${T.chars} WHERE guid = ? AND account = ?`, [guid, account]);
    return rows[0] || null;
  }

  async function accountCharacters(account) {
    const [rows] = await db.query(`SELECT guid, name, level FROM ${T.chars} WHERE account = ? AND deleteDate IS NULL ORDER BY guid`, [account]);
    return rows;
  }

  // Sesión = token SSO (query ?token= o cookie) + personaje (el del token o ?char= de la misma cuenta)
  async function session(req, q) {
    const cookies = parseCookies(req);
    const fromQuery = !!q.token;
    const tok = await findToken(q.token || cookies[COOKIE]);
    if (!tok || !tok.valid) return { error: "no_session" };
    const account = Number(tok.account);
    let guid = Number(q.char || 0) || Number(tok.character_guid || 0);
    let character = await findCharacter(account, guid);
    if (!character && q.char) character = null;
    const setCookie = fromQuery ? { "Set-Cookie": cookieHeader(tok.token) } : {};
    return { token: tok.token, account, bnet: Number(tok.battlenet_account), realm: Number(tok.realm), character, setCookie };
  }

  // ------------------------------------------------------------------ datos
  async function listTickets(guid) {
    const [rows] = await db.query(`SELECT ticketId, message, response, createTime, lastModifiedTime, closedBy, completed, assignedTo FROM ${T.tickets} WHERE guid = ? ORDER BY (closedBy <> 0), ticketId DESC LIMIT 20`, [guid]);
    return rows;
  }
  async function getTicket(guid, id) {
    const [rows] = await db.query(`SELECT ticketId, message, response, createTime, lastModifiedTime, closedBy, completed, assignedTo FROM ${T.tickets} WHERE ticketId = ? AND guid = ?`, [id, guid]);
    return rows[0] || null;
  }
  async function pendingQueue(guid) {
    const [rows] = await db.query(`SELECT id, action, ticketId, text, created, processed, result FROM ${T.queue} WHERE guid = ? AND (processed = 0 OR created > UNIX_TIMESTAMP() - 120) ORDER BY id DESC LIMIT 10`, [guid]);
    return rows;
  }
  async function openCount(guid) {
    const [[a]] = await db.query(`SELECT COUNT(*) AS n FROM ${T.tickets} WHERE guid = ? AND closedBy = 0`, [guid]);
    const [[b]] = await db.query(`SELECT COUNT(*) AS n FROM ${T.queue} WHERE guid = ? AND action = ? AND processed = 0`, [guid, QUEUE.Create]);
    return Number(a.n) + Number(b.n);
  }
  async function enqueue(s, action, ticketId, text) {
    const c = s.character;
    const [r] = await db.query(`INSERT INTO ${T.queue} (action, account, guid, ticketId, text, mapId, posX, posY, posZ, created) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, UNIX_TIMESTAMP())`,
      [action, s.account, c.guid, ticketId, text, c.map, c.position_x, c.position_y, c.position_z]);
    log(`soporte: cola #${r.insertId} acción ${action} cuenta ${s.account} personaje ${c.name} (${c.guid}) ticket ${ticketId}`);
    return r.insertId;
  }

  const ticketState = (tk, t) => (Number(tk.closedBy) !== 0 ? ["closed", t.closed] : Number(tk.completed) ? ["answered", t.answered] : ["open", t.open]);

  // ------------------------------------------------------------------ páginas
  const CLIENT_JS = `
try { if (window.wowClient && typeof wowClient.getClientData === 'function') { var cd = wowClient.getClientData(); fetch('/support/api/clientdata', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: typeof cd === 'string' ? cd : JSON.stringify(cd) }); } } catch (e) {}`;

  function layout(lang, s, title, body, active) {
    const t = tr(lang);
    const who = s && s.character ? `<span class="who">${esc(s.character.name)}</span>` : "";
    // La pestaña de fallos solo aparece si hay adónde mandarlos (token de GitHub configurado):
    // un enlace que lleva a «esto está desactivado» es peor que no tener el enlace.
    const nav = [["kb", `${base(lang)}/games/wow`, t.kb], ["submit", `${base(lang)}/ticket/submit`, t.submit], ["status", `${base(lang)}/ticket/status`, t.status]]
      .concat(bugs && bugs.enabled ? [["bug", `${base(lang)}/bug`, t.bug]] : [])
      .map(([k, href, label]) => `<a class="tab${active === k ? " on" : ""}" href="${href}">${esc(label)}</a>`).join("");
    return page(`${title} - ${cfg.shopName}`, `<div class="card wide"><div class="head"><h1>${esc(cfg.shopName)} · ${esc(t.support)}</h1>${who}</div><nav class="tabs">${nav}</nav>${body}</div><script>${CLIENT_JS}</script>`);
  }

  const msgBox = (lang, s, title, text, cls = "") => layout(lang, s, title, `<div class="msg ${cls}">${esc(text)}</div>`);

  function kbPage(lang, s) {
    const t = tr(lang);
    const faq = t.faq.map(([q, a]) => `<details><summary>${esc(q)}</summary><p>${a}</p></details>`).join("");
    const reportar = bugs && bugs.enabled ? ` <a class="btn sec" href="${base(lang)}/bug">${esc(t.bug)}</a>` : "";
    return layout(lang, s, t.kb, `<h2>${esc(t.faqTitle)}</h2>${faq}<p class="foot"><a class="btn" href="${base(lang)}/ticket/submit">${esc(t.submit)}</a>${reportar}</p>`, "kb");
  }

  async function submitPage(lang, s, err) {
    const t = tr(lang);
    if (!s.character) {
      const chars = await accountCharacters(s.account);
      const list = chars.map((c) => `<a class="btn sec" href="${base(lang)}/ticket/submit?char=${c.guid}">${esc(c.name)} (${c.level})</a>`).join(" ");
      return layout(lang, s, t.submit, `<p>${esc(t.choose)}</p><div class="chars">${list || esc(t.noChar)}</div>`, "submit");
    }
    const form = `<form method="post" action="${base(lang)}/ticket/submit?char=${s.character.guid}">
<label>${esc(t.title)}<input name="title" maxlength="100" required></label>
<label>${esc(t.body)}<textarea name="body" rows="8" maxlength="4000" required></textarea></label>
<div class="row"><span class="foot">${esc(t.character)}: ${esc(s.character.name)}</span><button type="submit">${esc(t.send)}</button></div></form>`;
    return layout(lang, s, t.submit, `${err ? `<div class="msg bad">${esc(err)}</div>` : ""}${form}`, "submit");
  }

  // Reportar un fallo. A diferencia del ticket, esto no pasa por el core: se va directo a la API
  // de GitHub desde aquí (bugs.mjs), así que no hay cola ni página de espera.
  // La lista de fallos propios no sale de aquí: se lee de GitHub (ver bugs.mjs). Por eso puede
  // fallar por su cuenta, y cuando falla se dice en vez de enseñar una lista vacía, que
  // parecería que los reportes se han perdido.
  function listaDeFallos(lang, mios) {
    const t = tr(lang);
    if (!mios.ok) return `<div class="msg">${esc(t.bugListFail)}</div>`;
    if (!mios.lista.length) return `<div class="msg">${esc(t.bugNoMine)}</div>`;
    return mios.lista.map((f) => {
      const cerrado = f.estado === "closed";
      const respuestas = f.respuestas ? ` <span class="foot">· ${f.respuestas} ${esc(t.bugAnswers.toLowerCase())}</span>` : "";
      return `<a class="tk ${cerrado ? "closed" : "open"}" href="${base(lang)}/bug/${f.numero}"><span class="badge">${esc(cerrado ? t.closed : t.open)}</span> <b>#${f.numero}</b> ${esc(f.titulo.slice(0, 80))}${respuestas}<span class="foot right">${esc(fmtDate(Math.floor(f.creado / 1000), lang))}</span></a>`;
    }).join("");
  }

  async function bugPage(lang, s, aviso, hecho) {
    const t = tr(lang);
    if (hecho)
      return layout(lang, s, t.bug, `<div class="msg ok">${esc(t.bugOk.replace("%n", hecho.numero))}</div>
<p><a class="btn" href="${base(lang)}/bug/${hecho.numero}">${esc(t.bugSee)}</a> <a class="btn sec" href="${base(lang)}/bug">${esc(t.bugAnother)}</a></p>
<h2>${esc(t.bugMine)}</h2>${listaDeFallos(lang, await bugs.mios(s.account))}`, "bug");

    const [quedan, mios] = await Promise.all([bugs.quedan(s.account), bugs.mios(s.account)]);
    const form = quedan > 0 ? `<form method="post" action="${base(lang)}/bug${s.character ? `?char=${s.character.guid}` : ""}">
<label>${esc(t.bugTitle)}<input name="title" maxlength="120" required></label>
<label>${esc(t.bugBody)}<textarea name="body" rows="8" maxlength="4000" required placeholder="${esc(t.bugPh)}"></textarea></label>
<div class="row"><span class="foot">${esc(t.bugLeft.replace("%n", quedan))}</span><button type="submit">${esc(t.bugSend)}</button></div></form>`
      : `<div class="msg bad">${esc(t.bugRate)}</div>`;

    return layout(lang, s, t.bug, `<p>${esc(t.bugIntro)}</p><p class="foot">${esc(t.bugNotTicket)} <a href="${base(lang)}/ticket/submit">${esc(t.submit)}</a></p>
${aviso ? `<div class="msg bad">${esc(aviso)}</div>` : ""}${form}
<h2>${esc(t.bugMine)}</h2>${listaDeFallos(lang, mios)}`, "bug");
  }

  // Un fallo propio con sus respuestas, dentro del soporte. El enlace a GitHub se deja, pero el
  // navegador del juego es incomodo para salir y volver, asi que lo normal es leerlo aqui.
  async function bugDetallePage(lang, s, numero) {
    const t = tr(lang);
    const r = await bugs.uno(s.account, numero);
    if (!r.ok) return msgBox(lang, s, t.bug, t.bugNotFound, "bad");

    const f = r.fallo;
    const cerrado = f.estado === "closed";
    // El texto de las respuestas lo escriben personas en GitHub: se pinta escapado, nunca como
    // HTML, aunque venga del lado "de casa".
    const respuestas = r.respuestas.length
      ? r.respuestas.map((c) => `<div class="box gm"><div class="foot">${esc(c.autor)} · ${esc(fmtDate(Math.floor(c.cuando / 1000), lang))}</div>${nl2br(c.texto)}</div>`).join("")
      : `<div class="box">${esc(t.bugNoAnswers)}</div>`;

    const body = `<div class="tk ${cerrado ? "closed" : "open"} nohover"><span class="badge">${esc(cerrado ? t.closed : t.open)}</span> <b>#${f.numero}</b> ${esc(f.titulo)}
<div class="foot">${esc(t.created)}: ${esc(fmtDate(Math.floor(f.creado / 1000), lang))}</div></div>
<h2>${esc(t.bugYourText)}</h2><div class="box">${nl2br(f.texto)}</div>
<h2>${esc(t.bugAnswers)}</h2>${respuestas}
<p><a class="btn sec" href="${base(lang)}/bug">${esc(t.bugBackList)}</a> <a class="btn sec" href="${esc(f.url)}" target="_blank" rel="noopener">${esc(t.bugOnGithub)}</a></p>`;
    return layout(lang, s, `${t.bug} #${numero}`, body, "bug");
  }

  async function statusPage(lang, s) {
    const t = tr(lang);
    if (!s.character) return submitPage(lang, s);
    const [tickets, queue] = await Promise.all([listTickets(s.character.guid), pendingQueue(s.character.guid)]);
    const pend = queue.filter((qi) => Number(qi.action) === QUEUE.Create && Number(qi.processed) === 0)
      .map((qi) => `<div class="tk pending"><span class="badge">${esc(t.pending)}</span> <b>${esc(qi.text.split("\n")[0])}</b> <span class="foot">${esc(t.sending)}</span></div>`).join("");
    const rows = tickets.map((tk) => {
      const [cls, label] = ticketState(tk, t);
      return `<a class="tk ${cls}" href="${base(lang)}/ticket/${tk.ticketId}"><span class="badge">${esc(label)}</span> <b>#${tk.ticketId}</b> ${esc(tk.message.split("\n")[0].slice(0, 80))}<span class="foot right">${esc(fmtDate(tk.lastModifiedTime || tk.createTime, lang))}</span></a>`;
    }).join("");
    const body = pend + rows || `<div class="msg">${esc(t.none)}</div>`;
    const refresh = pend ? `<meta http-equiv="refresh" content="4">` : "";
    return layout(lang, s, t.status, `${refresh}${body}`, "status");
  }

  async function ticketPage(lang, s, id, notice) {
    const t = tr(lang);
    if (!s.character) return msgBox(lang, s, t.ticket, t.noChar, "bad");
    const tk = await getTicket(s.character.guid, id);
    if (!tk) return msgBox(lang, s, t.ticket, t.err.ticket_not_found, "bad");
    const [cls, label] = ticketState(tk, t);
    const queue = (await pendingQueue(s.character.guid)).filter((qi) => Number(qi.ticketId) === Number(id) && Number(qi.processed) === 0);
    const pendingHtml = queue.map((qi) => `<div class="msg">${esc(t.pending)}: ${esc(Number(qi.action) === QUEUE.Close ? t.close : qi.text.slice(0, 80))}</div>`).join("");
    const closed = Number(tk.closedBy) !== 0;
    const actions = closed ? "" : `<form method="post" action="${base(lang)}/ticket/${id}?char=${s.character.guid}">
<label>${esc(t.reply)}<textarea name="body" rows="4" maxlength="4000" placeholder="${esc(t.replyPh)}"></textarea></label>
<div class="row"><button type="submit" name="action" value="close" class="sec" onclick="return confirm(${JSON.stringify(t.closeConfirm)})">${esc(t.close)}</button><button type="submit" name="action" value="reply">${esc(t.reply)}</button></div></form>`;
    const body = `<div class="tk ${cls} nohover"><span class="badge">${esc(label)}</span> <b>#${tk.ticketId}</b> ${esc(tk.message.split("\n")[0])}
<div class="foot">${esc(t.created)}: ${esc(fmtDate(tk.createTime, lang))} · ${esc(t.updated)}: ${esc(fmtDate(tk.lastModifiedTime, lang))}</div></div>
${notice ? `<div class="msg ok">${esc(notice)}</div>` : ""}${pendingHtml}
<h2>${esc(t.yourMessage)}</h2><div class="box">${nl2br(tk.message)}</div>
<h2>${esc(t.gmResponse)}</h2><div class="box ${tk.response ? "gm" : ""}">${tk.response ? nl2br(tk.response) : esc(closed ? "" : Number(tk.completed) ? t.noResponse : t.waiting)}</div>
${actions}<p><a class="btn sec" href="${base(lang)}/ticket/status">${esc(t.back)}</a></p>${queue.length ? `<meta http-equiv="refresh" content="4">` : ""}`;
    return layout(lang, s, `${t.ticket} #${id}`, body, "status");
  }

  async function pendingPage(lang, s, queueId) {
    const t = tr(lang);
    const [rows] = await db.query(`SELECT id, action, processed, result, text FROM ${T.queue} WHERE id = ? AND account = ?`, [queueId, s.account]);
    const qi = rows[0];
    if (!qi) return msgBox(lang, s, t.submit, t.err.generic, "bad");
    if (Number(qi.processed) === 1) return { redirect: `${base(lang)}/ticket/${qi.result}?char=${s.character ? s.character.guid : ""}` };
    if (Number(qi.processed) === 2) return msgBox(lang, s, t.submit, t.err[QUEUE_ERR[qi.result]] || t.err.generic, "bad");
    return layout(lang, s, t.submit, `<meta http-equiv="refresh" content="3"><div class="msg"><span class="spin"></span>${esc(t.sending)}</div><p class="foot">${esc(t.queued)}</p>`, "submit");
  }

  // ------------------------------------------------------------------ router
  const parseForm = (body) => Object.fromEntries(new URLSearchParams(body));

  // Destino de vuelta tras el SSO: ruta local (/support, /shop) o URL absoluta (el cliente manda la
  // URL original https://<region>.battle.net/... que llegó aquí por browser_url_map).
  function refTarget(q, lang) {
    let target = q.ref || q.redirect || q.next || q.returnUrl || q.return_to || q.url || q.dest || q.continue || "";
    try { if (/^https?:\/\//i.test(target)) { const u = new URL(target); target = u.pathname + u.search; } } catch { target = ""; }
    return /^\/(support|shop)(\/|$)/.test(target) && !/^\/login/.test(target) ? target : `${base(lang)}/ticket/status`;
  }

  // URL absoluta que pidió el cliente (el shim la manda en X-Wow-Original-Url); si no, la pública
  const originalUrl = (req) => req.headers["x-wow-original-url"] || new URL(req.url, cfg.publicUrl).href;

  return async function handleSupport(req, res, p, q) {
    const orig = req.headers["x-wow-original-url"] || "";

    // Paso 3: el cliente carga /login/sso?token=...&<query de /login/> (ruta descartada, query conservada)
    if (p === "/login/sso" || p.startsWith("/login/sso/")) {
      log(`sso ${req.method} ${req.url}${orig ? ` (original ${orig})` : ""} cliente ${req.headers["x-game-client"] || "-"}`);
      const tok = await findToken(q.token);
      const lang = langOf(q.locale || q.loc || q.lang);
      if (!tok || !tok.valid) return send(res, 403, msgBox(lang, null, tr(lang).support, tr(lang).noSession, "bad"));
      let target = refTarget(q, lang);
      if (!q.ref && p.length > "/login/sso".length && /^\/login\/sso\/(support|shop)/.test(p)) target = p.slice("/login/sso".length);
      return redirect(res, target, { "Set-Cookie": cookieHeader(tok.token) });
    }

    // Paso 2: /login/[<lang>/]?ref=...&app=support -> 200 con el salt que dispara el SSO en el proxy
    if (/^\/login(?:\/[a-zA-Z]{2}(?:-[a-zA-Z]{2})?)?$/.test(p)) {
      const lang = langOf(p.split("/")[2] || q.locale || q.loc);
      const target = refTarget(q, lang);
      const s = await session(req, q);
      if (!s.error) return redirect(res, target, s.setCookie || {}); // ya hay sesión: no repetir el SSO
      const salt = randomBytes(16).toString("hex");
      log(`sso salt ${req.method} ${req.url}${orig ? ` (original ${orig})` : ""} -> ${target}`);
      const t = tr(lang);
      const body = layout(lang, null, t.support, `<div class="msg">${esc(t.ssoWait)}</div><p class="foot"><a class="btn sec" href="${esc(target)}">${esc(t.back)}</a></p>`);
      return send(res, 200, body, "text/html; charset=utf-8", { "X-BNET-Authenticate": `BattlenetToken server-salt="${salt}"` });
    }

    if (req.method === "POST" && p === "/support/api/clientdata") {
      log(`soporte: wowClient.getClientData = ${(await readBody(req)).slice(0, 2000)}`);
      return sendJson(res, 200, { ok: true });
    }

    // /support[/<lang>][/games/wow | /ticket/submit | /ticket/status | /ticket/<id>]
    const m = p.match(/^\/support(?:\/([a-zA-Z]{2}(?:-[a-zA-Z]{2})?))?(?:\/(.*))?$/);
    if (!m) return false;
    const lang = langOf(m[1] || q.locale || q.loc);
    const rest = (m[2] || "").replace(/\/+$/, "");
    const t = tr(lang);

    if (rest === "" || rest.startsWith("games")) {
      const s = await session(req, q);
      return send(res, 200, kbPage(lang, s.error ? null : s), "text/html; charset=utf-8", s.setCookie || {});
    }

    const s = await session(req, q);
    if (s.error) {
      // Paso 1: sin sesión -> /login/?ref=<URL completa>&app=support (el cliente hará el SSO); un POST
      // (formulario con la cookie caducada) no se puede reanudar: aviso.
      if (req.method !== "GET") return send(res, 403, msgBox(lang, null, t.support, t.noSession, "bad"));
      log(`soporte: sin sesión ${req.method} ${p}${orig ? ` (original ${orig})` : ""} -> /login/`);
      return redirect(res, `/login/?ref=${encodeURIComponent(originalUrl(req))}&app=support`);
    }
    const hdr = s.setCookie || {};

    // El fallo va a GitHub, no a la cola de tickets; pide sesion igual, para saber quien lo manda
    // y poder ponerle un tope por cuenta.
    if (rest === "bug") {
      if (!bugs || !bugs.enabled) return send(res, 404, msgBox(lang, s, t.bug, t.bugOff, "bad"), "text/html; charset=utf-8", hdr);
      if (req.method === "POST") {
        const f = parseForm(await readBody(req, 65536));
        const c = s.character;
        const r = await bugs.reportar({
          titulo: f.title,
          cuerpo: f.body,
          cuenta: s.account,
          personaje: c ? `${c.name} (${c.level})` : "",
          reino: cfg.shopName,
          lugar: c ? `mapa ${c.map} (${Number(c.position_x).toFixed(1)}, ${Number(c.position_y).toFixed(1)}, ${Number(c.position_z).toFixed(1)})` : "",
          lang,
        });
        if (r.ok) return send(res, 200, await bugPage(lang, s, null, r), "text/html; charset=utf-8", hdr);
        const aviso = { empty: t.bugEmpty, rate: t.bugRate, disabled: t.bugOff }[r.error] || t.bugFail;
        return send(res, r.error === "rate" ? 429 : 400, await bugPage(lang, s, aviso), "text/html; charset=utf-8", hdr);
      }
      return send(res, 200, await bugPage(lang, s), "text/html; charset=utf-8", hdr);
    }

    const bm = rest.match(/^bug\/(\d+)$/);
    if (bm) {
      if (!bugs || !bugs.enabled) return send(res, 404, msgBox(lang, s, t.bug, t.bugOff, "bad"), "text/html; charset=utf-8", hdr);
      return send(res, 200, await bugDetallePage(lang, s, Number(bm[1])), "text/html; charset=utf-8", hdr);
    }

    if (rest === "ticket/submit") {
      if (req.method === "POST") {
        const f = parseForm(await readBody(req, 65536));
        const title = String(f.title || "").trim().slice(0, 100);
        const body = String(f.body || "").trim().slice(0, 4000);
        if (!s.character) return send(res, 400, await submitPage(lang, s, t.noChar), "text/html; charset=utf-8", hdr);
        if (!title || !body) return send(res, 400, await submitPage(lang, s, t.required), "text/html; charset=utf-8", hdr);
        if ((await openCount(s.character.guid)) >= MAX_OPEN) return send(res, 409, await submitPage(lang, s, t.tooMany), "text/html; charset=utf-8", hdr);
        const id = await enqueue(s, QUEUE.Create, 0, `${title}\n\n${body}`);
        return redirect(res, `${base(lang)}/ticket/pending?id=${id}&char=${s.character.guid}`, hdr);
      }
      return send(res, 200, await submitPage(lang, s), "text/html; charset=utf-8", hdr);
    }
    if (rest === "ticket/pending") {
      const r = await pendingPage(lang, s, Number(q.id || 0));
      return r.redirect ? redirect(res, r.redirect, hdr) : send(res, 200, r, "text/html; charset=utf-8", hdr);
    }
    if (rest === "ticket/status" || rest === "ticket") return send(res, 200, await statusPage(lang, s), "text/html; charset=utf-8", hdr);

    const tm = rest.match(/^ticket\/(\d+)$/);
    if (tm) {
      const id = Number(tm[1]);
      if (req.method === "POST") {
        const f = parseForm(await readBody(req, 65536));
        if (!s.character) return send(res, 400, await ticketPage(lang, s, id), "text/html; charset=utf-8", hdr);
        const tk = await getTicket(s.character.guid, id);
        if (!tk || Number(tk.closedBy) !== 0) return send(res, 404, msgBox(lang, s, t.ticket, t.err.ticket_not_found, "bad"), "text/html; charset=utf-8", hdr);
        if (f.action === "close") await enqueue(s, QUEUE.Close, id, "");
        else {
          const body = String(f.body || "").trim().slice(0, 4000);
          if (!body) return send(res, 400, await ticketPage(lang, s, id), "text/html; charset=utf-8", hdr);
          await enqueue(s, QUEUE.Message, id, body);
        }
        return redirect(res, `${base(lang)}/ticket/${id}?char=${s.character.guid}`, hdr);
      }
      return send(res, 200, await ticketPage(lang, s, id), "text/html; charset=utf-8", hdr);
    }
    return false;
  };
}
