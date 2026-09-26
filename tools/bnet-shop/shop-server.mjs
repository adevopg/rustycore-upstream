#!/usr/bin/env node
// Web de la tienda in-game (checkout con SumUp) para LegionCore 7.3.5 con Bpay.WebCheckout = 1.
//
// El cliente (WowBrowserProxy.exe + tools/wow-cef-shim) llega aquí a través del mapa de URLs
// de auth.browser_url_map (www.battle.net y nydus.battle.net -> este servidor):
//
//   1. pantalla "loading" del checkout: pide /nydus/Bnet/client/purchase/jsutil, que define
//      SimpleCheckoutUtil.getCheckoutUrl() a partir de scene.purchaseRequest (ssoToken,
//      externalTransactionId, serverValidationSignature...) y redirige a /shop/checkout;
//   2. /shop/checkout valida el token SSO (auth.battlenet_account_web_token) y el pedido
//      (auth.battlepay_purchase, status 0), crea un checkout en SumUp y muestra el widget de
//      tarjeta de SumUp;
//   3. cuando el widget termina, la página pregunta a /shop/api/status: el servidor consulta
//      GET /v0.1/checkouts/{id} en SumUp (sin webhooks) y, si status == "PAID", marca el pedido
//      como pagado (status 1, paid, payment_ref, web_order_id) y la página llama
//      scene.notifyPurchaseSubmitted({GlobalOrderId}) -> el worldserver entrega el producto.
//
// Configuración por variables de entorno (ver shop.env.dist). Sin dependencias salvo mysql2.

import { createServer } from "node:https";
import fs from "node:fs";
import mysql from "mysql2/promise";
import { createSupport } from "./support.mjs";
import { createBugs } from "./bugs.mjs";
import { createRaf } from "./raf.mjs";
import { createAuthenticator } from "./authenticator.mjs";
import { createSms } from "./sms.mjs";
import { page as uiPage } from "./ui.mjs";
import { createTwitter } from "./twitter.mjs";

const env = process.env;
const cfg = {
  port: Number(env.SHOP_PORT || 8095),
  bind: env.SHOP_BIND || "0.0.0.0",
  publicUrl: (env.SHOP_PUBLIC_URL || "").replace(/\/+$/, ""),
  certFile: env.SHOP_TLS_CERT,
  keyFile: env.SHOP_TLS_KEY,
  shopName: env.SHOP_NAME || "Tienda",
  db: {
    host: env.SHOP_DB_HOST || "127.0.0.1",
    port: Number(env.SHOP_DB_PORT || 3306),
    user: env.SHOP_DB_USER || env.LEGION_DB_USER,
    password: env.SHOP_DB_PASS || env.LEGION_DB_PASS,
    auth: env.SHOP_DB_AUTH || "auth",
    world: env.SHOP_DB_WORLD || "world",
    characters: env.SHOP_DB_CHARACTERS || "characters",
  },
  sumup: {
    api: (env.SUMUP_API || "https://api.sumup.com").replace(/\/+$/, ""),
    key: env.SUMUP_API_KEY || "",
    merchant: env.SUMUP_MERCHANT_CODE || "",
    sdk: env.SUMUP_SDK || "https://gateway.sumup.com/gateway/ecom/card/v2/sdk.js",
  },
  // Reporte de fallos del juego: el servidor abre la incidencia en GitHub con un token suyo, para
  // que los jugadores puedan reportar sin tener cuenta ni acceso al repositorio. Si falta el token
  // la opcion sencillamente no aparece.
  github: {
    repo: env.GITHUB_BUGS_REPO || "",
    token: env.GITHUB_BUGS_TOKEN || "",
    labels: env.GITHUB_BUGS_LABELS || "",
  },
  pollMs: Number(env.SHOP_POLL_MS || 3000),
  logFile: env.SHOP_LOG || "",
};

for (const [k, v] of Object.entries({ SHOP_PUBLIC_URL: cfg.publicUrl, SHOP_TLS_CERT: cfg.certFile, SHOP_TLS_KEY: cfg.keyFile, "SHOP_DB_USER/LEGION_DB_USER": cfg.db.user, SUMUP_API_KEY: cfg.sumup.key, SUMUP_MERCHANT_CODE: cfg.sumup.merchant }))
  if (!v) { console.error(`falta ${k} en el entorno (ver shop.env.dist)`); process.exit(2); }

const log = (...a) => {
  const line = `${new Date().toISOString()} ${a.map((x) => (x instanceof Error ? x.stack || x.message : typeof x === "string" ? x : JSON.stringify(x))).join(" ")}`;
  console.log(line);
  if (cfg.logFile) fs.appendFile(cfg.logFile, line + "\n", () => {});
};

const db = mysql.createPool({ host: cfg.db.host, port: cfg.db.port, user: cfg.db.user, password: cfg.db.password, waitForConnections: true, connectionLimit: 5, charset: "utf8mb4", decimalNumbers: false });
const T = { token: `\`${cfg.db.auth}\`.battlenet_account_web_token`, order: `\`${cfg.db.auth}\`.battlepay_purchase`, product: `\`${cfg.db.world}\`.battlepay_product`, productWeb: `\`${cfg.db.world}\`.battlepay_product_web`, display: `\`${cfg.db.world}\`.battlepay_display_info`, displayLocale: `\`${cfg.db.world}\`.battlepay_display_info_locales` };

// Estados de auth.battlepay_purchase (Battlepay::WebPurchaseStatus del core)
const ST = { Created: 0, Paid: 1, Delivered: 2, Failed: 3, Revoked: 4 };

// ---------------------------------------------------------------------------------------------
// SumUp
async function sumup(method, path, body) {
  const r = await fetch(cfg.sumup.api + path, {
    method,
    headers: { Authorization: `Bearer ${cfg.sumup.key}`, Accept: "application/json", ...(body ? { "Content-Type": "application/json" } : {}) },
    body: body ? JSON.stringify(body) : undefined,
    signal: AbortSignal.timeout(15000),
  });
  const text = await r.text();
  let json;
  try { json = JSON.parse(text); } catch { json = { raw: text }; }
  if (!r.ok) {
    const e = new Error(`SumUp ${method} ${path} -> HTTP ${r.status}: ${text.slice(0, 300)}`);
    e.status = r.status;
    throw e;
  }
  return json;
}

const sumupIdOf = (order) => (order.payment_ref || "").startsWith("sumup:") ? order.payment_ref.slice(6).split(";")[0] : "";

async function sumupCreateCheckout(order, product) {
  return sumup("POST", "/v0.1/checkouts", {
    checkout_reference: order.external_id,
    amount: Number(order.price),
    currency: order.currency,
    merchant_code: cfg.sumup.merchant,
    description: `${cfg.shopName}: ${product.name}`.slice(0, 255),
  });
}

// Transacción con éxito del checkout (SumUp devuelve transaction_code arriba y/o en transactions[])
function sumupTransactionCode(chk) {
  if (chk.transaction_code) return String(chk.transaction_code);
  const ok = (chk.transactions || []).find((t) => String(t.status || "").toUpperCase() === "SUCCESSFUL") || (chk.transactions || [])[0];
  return ok ? String(ok.transaction_code || ok.id || "") : "";
}

// ---------------------------------------------------------------------------------------------
// Datos
async function findToken(token) {
  if (!/^[0-9a-fA-F]{16,128}$/.test(token || "")) return null;
  const [rows] = await db.query(`SELECT token, battlenet_account, account, kind, (expires > NOW()) AS valid FROM ${T.token} WHERE token = ?`, [token]);
  return rows[0] || null;
}

async function findOrder(ext, sig) {
  if (!/^[0-9a-fA-F]{8,40}$/.test(ext || "") || !/^[0-9a-fA-F]{8,40}$/.test(sig || "")) return null;
  const [rows] = await db.query(`SELECT * FROM ${T.order} WHERE external_id = ? AND signature = ?`, [ext, sig]);
  return rows[0] || null;
}

// Los textos de los productos viven en battlepay_display_info (en ingles) y sus traducciones
// en battlepay_display_info_locales, indexadas por el numero de LocaleConstant de
// src/common/Common.h. El juego manda su locale en la peticion de compra.
const LOCALES = { enUS: 0, koKR: 1, frFR: 2, deDE: 3, zhCN: 4, zhTW: 5, esES: 6, esMX: 7, ruRU: 8, ptBR: 10, itIT: 11 };

function localeNum(locale) {
  const l = String(locale || "").replace(/[-_]/g, "");
  if (LOCALES[l] !== undefined) return LOCALES[l];
  // "es", "es-ES", "es_MX"... Sin nada reconocible, espanol: es un servidor espanol.
  const dos = l.slice(0, 2).toLowerCase();
  const porIdioma = { en: 0, ko: 1, fr: 2, de: 3, zh: 4, es: 6, ru: 8, pt: 10, it: 11 };
  return porIdioma[dos] !== undefined ? porIdioma[dos] : LOCALES.esES;
}

async function findProduct(productId, locale) {
  const [rows] = await db.query(
    `SELECT p.ProductID AS id, p.CurrentPriceFixedPoint AS price,
            COALESCE(NULLIF(l.Name1, ''), NULLIF(d.Name1, ''), CONCAT('Producto ', p.ProductID)) AS name,
            COALESCE(NULLIF(l.Name3, ''), d.Name3, '') AS description
       FROM ${T.product} p
       LEFT JOIN ${T.display} d ON d.DisplayInfoId = p.DisplayInfoID
       LEFT JOIN ${T.displayLocale} l ON l.Id = d.DisplayInfoId AND l.Locale = ?
      WHERE p.ProductID = ?`, [localeNum(locale), productId]);
  return rows[0] || { id: productId, name: `Producto ${productId}`, description: "", price: null };
}

/*
 * El catalogo para la web.
 *
 * Sale de las MISMAS tablas que usa el juego: `battlepay_product` es el catalogo y
 * `battlepay_display_info(_locales)` los nombres. La web tenia el suyo aparte en otra base
 * de datos, asi que habia dos tiendas que no compartian nada; esto lo deja en una.
 *
 * `battlepay_product_web` no es un segundo catalogo: solo anyade lo que un escaparate
 * necesita y una tienda dentro del juego no (categoria, destacado, imagen). Con INNER JOIN
 * y Visible=1, un producto que no se quiera anunciar -- la recompensa de invitar a un
 * amigo, por ejemplo -- se sigue vendiendo en el juego y no sale aqui.
 */
async function catalogo(locale) {
  const [rows] = await db.query(
    `SELECT p.ProductID AS id,
            p.CurrentPriceFixedPoint AS price,
            COALESCE(NULLIF(l.Name1, ''), NULLIF(d.Name1, ''), CONCAT('Producto ', p.ProductID)) AS name,
            COALESCE(NULLIF(l.Name3, ''), d.Name3, '') AS description,
            w.Category AS category, w.Featured AS featured, w.ImageUrl AS image, w.Icon AS icon
       FROM ${T.product} p
       JOIN ${T.productWeb} w ON w.ProductID = p.ProductID AND w.Visible = 1
       LEFT JOIN ${T.display} d ON d.DisplayInfoId = p.DisplayInfoID
       LEFT JOIN ${T.displayLocale} l ON l.Id = d.DisplayInfoId AND l.Locale = ?
      ORDER BY w.Featured DESC, w.SortOrder, p.ProductID`, [localeNum(locale)]);

  return rows.map((r) => ({
    id: r.id,
    name: r.name,
    description: r.description || "",
    // En centimos: el precio se guarda como decimal y en coma flotante 20.10 no es 20.10.
    // Un entero de centimos no tiene ese problema y es lo que la web ya sabia pintar.
    price_cents: Math.round(Number(r.price) * 100),
    category: r.category || "",
    featured: Boolean(r.featured),
    icon: r.icon || "",
    image: r.image || "",
  }));
}

async function setPaymentRef(order, ref) {
  await db.query(`UPDATE ${T.order} SET payment_ref = ? WHERE id = ? AND status = ?`, [ref, order.id, ST.Created]);
  order.payment_ref = ref;
}

// Marca el pedido como pagado; sólo pasa de 0 a 1 (idempotente frente a dobles notificaciones).
async function markPaid(order, chk) {
  const txCode = sumupTransactionCode(chk) || chk.id;
  const ref = `sumup:${chk.id};tx=${txCode}`;
  // También desde Failed (3): el jugador puede cerrar la ventana justo después de pagar y el
  // worldserver marca el pedido como cancelado antes de que la página vea el PAID de SumUp.
  const [r] = await db.query(`UPDATE ${T.order} SET status = ?, paid = NOW(), payment_ref = ?, web_order_id = ? WHERE id = ? AND status IN (?, ?)`,
    [ST.Paid, ref, String(txCode).slice(0, 40), order.id, ST.Created, ST.Failed]);
  if (r.affectedRows) log(`pedido ${order.external_id} PAGADO (${order.price} ${order.currency}) cuenta ${order.account} producto ${order.product_id} sumup ${chk.id} tx ${txCode}`);
  order.status = ST.Paid;
  order.web_order_id = String(txCode).slice(0, 40);
  order.payment_ref = ref;
}

async function markFailed(order, why) {
  const [r] = await db.query(`UPDATE ${T.order} SET status = ? WHERE id = ? AND status = ?`, [ST.Failed, order.id, ST.Created]);
  if (r.affectedRows) log(`pedido ${order.external_id} FALLIDO: ${why}`);
  order.status = ST.Failed;
}

// Consulta el estado en SumUp y actualiza el pedido. Devuelve "PAID" | "PENDING" | "FAILED" | "NONE".
async function refreshFromSumup(order) {
  const id = sumupIdOf(order);
  if (!id) return "NONE";
  let chk;
  try { chk = await sumup("GET", `/v0.1/checkouts/${encodeURIComponent(id)}`); }
  catch (e) {
    if (e.status === 404) return "NONE";
    throw e;
  }
  const st = String(chk.status || "").toUpperCase();
  if (st === "PAID") { await markPaid(order, chk); return "PAID"; }
  if (st === "PENDING") return "PENDING";
  // FAILED / EXPIRED / cualquier otro: el checkout de SumUp ya no sirve, se creará otro.
  // Se anota ";failed" en payment_ref para que el worldserver, cuando el jugador cierre la
  // ventana, muestre "Pago fallido" en vez de tratarlo como una cancelación normal.
  if (!String(order.payment_ref || "").endsWith(";failed")) {
    order.payment_ref = `${order.payment_ref};failed`;
    await db.query(`UPDATE ${T.order} SET payment_ref = ? WHERE id = ? AND status = ?`, [order.payment_ref, order.id, ST.Created]);
    log(`pedido ${order.external_id}: pago SumUp ${id} ${st || "?"}`);
  }
  return "FAILED";
}

// Comprueba token + pedido. Devuelve {order, product, token, locale} o {error}.
async function authorize(q) {
  const [tok, order] = await Promise.all([findToken(q.token), findOrder(q.ext, q.sig)]);
  if (!order) return { error: "order_not_found" };
  if (!tok) return { error: "bad_token" };
  if (!tok.valid) return { error: "token_expired" };
  if (Number(tok.account) !== Number(order.account)) return { error: "token_mismatch" };
  const product = await findProduct(order.product_id, q.locale);
  return { order, product, token: tok };
}

// ---------------------------------------------------------------------------------------------
// HTTP
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const jsStr = (s) => JSON.stringify(String(s ?? ""));

function send(res, code, body, type = "text/html; charset=utf-8", extra = {}) {
  res.writeHead(code, { "Content-Type": type, "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff", ...extra });
  res.end(body);
}
const sendJson = (res, code, obj) => send(res, code, JSON.stringify(obj), "application/json; charset=utf-8");

function readBody(req, limit = 16384) {
  return new Promise((resolve, reject) => {
    let data = "";
    req.on("data", (c) => { data += c; if (data.length > limit) { reject(new Error("body too large")); req.destroy(); } });
    req.on("end", () => resolve(data));
    req.on("error", reject);
  });
}

// "esES" / "es-ES" / "es_es" -> "es-ES"; idioma de los textos: es o en
function normLocale(l) {
  const m = String(l || "").replace("_", "-").match(/^([a-zA-Z]{2})-?([a-zA-Z]{2})?$/);
  if (!m) return "es-ES";   // servidor espanol: sin locale reconocible, espanol
  return m[1].toLowerCase() + "-" + (m[2] || m[1]).toUpperCase();
}
function fmtPrice(amount, currency, locale) {
  try { return new Intl.NumberFormat(locale, { style: "currency", currency }).format(Number(amount)); }
  catch { return `${Number(amount).toFixed(2)} ${currency}`; }
}

const I18N = {
  es: { title: "Finalizar compra", pay: "Pagar", cancel: "Cancelar", total: "Total", processing: "Comprobando el pago…", paid: "¡Pago recibido!", delivered: "Tu compra se está entregando en el juego. Esta ventana se cerrará sola.", failed: "El pago no se ha completado.", retry: "Intentar de nuevo", already: "Este pedido ya está pagado.", cancelled: "Pedido cancelado.", secure: "Pago seguro con tarjeta a través de SumUp.", entrega: "La entrega es automática: en cuanto se confirme el pago recibirás la compra en el juego.", err: { order_not_found: "Pedido no encontrado.", bad_token: "Sesión no válida. Cierra esta ventana y vuelve a intentarlo desde la tienda del juego.", token_expired: "La sesión ha caducado. Cierra esta ventana y vuelve a intentarlo desde la tienda del juego.", token_mismatch: "La sesión no corresponde a este pedido.", order_failed: "Este pedido fue cancelado. Vuelve a comprarlo desde la tienda del juego.", sumup: "No se ha podido iniciar el pago. Inténtalo más tarde.", generic: "Ha ocurrido un error." } },
  en: { title: "Checkout", pay: "Pay", cancel: "Cancel", total: "Total", processing: "Checking the payment…", paid: "Payment received!", delivered: "Your purchase is being delivered in game. This window will close by itself.", failed: "The payment was not completed.", retry: "Try again", already: "This order is already paid.", cancelled: "Order cancelled.", secure: "Secure card payment through SumUp.", entrega: "Delivery is automatic: as soon as the payment clears you get your purchase in game.", err: { order_not_found: "Order not found.", bad_token: "Invalid session. Close this window and try again from the in-game shop.", token_expired: "The session has expired. Close this window and try again from the in-game shop.", token_mismatch: "The session does not match this order.", order_failed: "This order was cancelled. Buy it again from the in-game shop.", sumup: "The payment could not be started. Please try again later.", generic: "Something went wrong." } },
};
const tr = (locale) => I18N[locale.slice(0, 2)] || I18N.es;

// El diseno vive en ui.mjs, compartido con soporte, RAF y autenticador: es el mismo
// lenguaje visual que www.nightspire.gg y se cambia en un solo sitio.

const page = (title, body, opts) => uiPage(esc, title, body, opts);

// Twitter (X) del cliente: proxy compatible con la API v1.1/OAuth 1.0a que habla el juego (ver twitter.mjs)
const twitter = createTwitter({ db, cfg, env, log, esc, page, send, sendJson });

// scene.* del navegador del juego, con guardas para poder abrir la página en un navegador normal
const SCENE_JS = `
var scene = window.scene || null;
function sceneCall(fn){ var a = Array.prototype.slice.call(arguments, 1); try { if (scene && typeof scene[fn] === 'function') return scene[fn].apply(scene, a); } catch (e) { console.log('scene.' + fn + ' failed: ' + e); } return undefined; }
// Nombre distinto de 'purchaseRequest': el navegador del cliente 3.4.3 registra la peticion de
// compra como variable global (window.purchaseRequest) y una funcion global con ese nombre la
// pisaria. Se mira, por orden, la variable global, scene.purchaseRequest y simpleCheckout.
function readPurchaseRequest(){ try { var pr = (typeof window.purchaseRequest !== 'function') ? window.purchaseRequest : null; if (!pr && scene) pr = scene.purchaseRequest; if (!pr && window.simpleCheckout) pr = window.simpleCheckout.purchaseRequest; if (typeof pr === 'string') pr = JSON.parse(pr); return pr || null; } catch (e) { return null; } }
// Cliente 3.4.3 (bnl_checkout 5.3.4): el navegador integrado expone un origen propio,
// https://blz-data/. La pagina de carga oficial de Blizzard hace GET a
// https://blz-data/purchaseRequest para obtener el pedido (JSON con productId, purchaseType,
// gameAccountId, externalTransactionId, serverValidationSignature, currencyCode, locale...),
// y los mensajes de vuelta (purchaseSubmitted, orderComplete, purchaseError,
// purchaseCanceledBeforeSubmit, windowCloseRequested...) se le entregan como POST JSON
// {type, payload} a ese origen ("data post"). Se mantiene scene.* para el cliente 7.3.5.
var BLZ_DATA = 'https://blz-data/';
function blzGet(path, cb){ try { var x = new XMLHttpRequest(); x.onreadystatechange = function(){ if (x.readyState === 4) cb(x.status === 200 ? x.responseText : null, x.status); }; x.open('GET', BLZ_DATA + path, true); x.send(); } catch (e) { cb(null, 0); } }
// Sonda del 26/09: el cliente acepta (200) un POST a https://blz-data/<ruta> con cuerpo JSON
// solo si es una peticion "simple" (text/plain; con application/json hay preflight y falla),
// con ruta no vacia y cuerpo no vacio. El nombre del campo que lleva el tipo de mensaje no se
// ha podido observar (siempre responde 200), asi que se mandan las variantes plausibles; las
// desconocidas solo generan un aviso en el cliente.
function blzPost(type, payload){
  var envelopes = [
    { type: type, payload: payload || {} }, { code: type, payload: payload || {} },
    { message: type, payload: payload || {} }, { event: type, payload: payload || {} },
    { name: type, payload: payload || {} }, { messageType: type, payload: payload || {} }
  ];
  envelopes.forEach(function(body){
    try { var x = new XMLHttpRequest(); x.open('POST', BLZ_DATA + type, true); x.setRequestHeader('Content-Type', 'text/plain'); x.send(JSON.stringify(body)); }
    catch (e) { console.log('blz-data post failed: ' + e); }
  });
}
function notifyClient(sceneFn, type, payload){ var r = sceneCall(sceneFn, JSON.stringify(payload || {})); blzPost(type, payload); return r; }
function closeWindow(){ notifyClient('requestCloseWindow', 'windowCloseRequested', {}); }`;

function errorPage(locale, code) {
  const t = tr(locale);
  return page(t.title, `<div class="card"><h1>${esc(cfg.shopName)}</h1><div class="msg bad">${esc(t.err[code] || t.err.generic)}</div>
<div class="row"><span></span><button class="sec" onclick="closeWindow()">${esc(t.cancel)}</button></div></div>
<script>${SCENE_JS}
notifyClient('notifyPurchaseError', 'purchaseError', { ErrorCodes: ${jsStr(code)}, errorCodes: [${jsStr(code)}] });
</script>`, { tema: "claro" });
}

// Página que abre el cliente antes del checkout (si el shim reescribe www.battle.net hacia aquí)
// y JS SimpleCheckoutUtil que pide la página "loading" embebida en WowBrowserProxy.exe.
function checkoutUrlJs() {
  return `
(function(){
  function q(o){ return Object.keys(o).map(function(k){ return encodeURIComponent(k) + '=' + encodeURIComponent(o[k] == null ? '' : o[k]); }).join('&'); }
  function pr(){ try { var p = (window.simpleCheckout && window.simpleCheckout.purchaseRequest) || (window.scene && scene.purchaseRequest); if (typeof p === 'string') p = JSON.parse(p); return p || {}; } catch (e) { return {}; } }
  // 3.4.3 (54261): purchaseRequest no trae ssoToken; el cliente lo anade a la URL de la pagina
  // de carga como ?token=... (o &token=...). Se toma de ahi cuando falta en purchaseRequest.
  function urlToken(){ try { var m = /[?&]token=([^&#]*)/.exec(window.location.search || ''); return m ? decodeURIComponent(m[1]) : ''; } catch (e) { return ''; } }
  window.SimpleCheckoutUtil = {
    getCheckoutUrl: function(){ var p = pr(); return ${jsStr(cfg.publicUrl)} + '/shop/checkout?' + q({ token: p.ssoToken || urlToken(), ext: p.externalTransactionId, sig: p.serverValidationSignature, product: p.productId, locale: p.locale, ga: p.gameAccountId }); },
    getNavbarUrl: function(){ return ${jsStr(cfg.publicUrl)} + '/shop/simplecheckout/navbar'; },
    isShopUrl: function(u){ return typeof u === 'string' && u.indexOf(${jsStr(cfg.publicUrl)}) === 0; }
  };
  if (typeof window.onSCUtilLoad === 'function') window.onSCUtilLoad();
})();`;
}

function loadingPage() {
  return page(cfg.shopName, `<div class="card"><h1>${esc(cfg.shopName)}</h1><div class="msg"><span class="spin"></span>…</div></div>
<script>${SCENE_JS}
${checkoutUrlJs()}
window.simpleCheckout = window.simpleCheckout || {};
function go(){ var p = readPurchaseRequest(); if (!p) return false; window.simpleCheckout.purchaseRequest = p; window.location.href = SimpleCheckoutUtil.getCheckoutUrl(); return true; }
// 3.4.3: pedir el pedido al cliente como hace la pagina oficial de Blizzard.
var blzTried = false;
function askBlzData(){ if (blzTried) return; blzTried = true; blzGet('purchaseRequest', function(body, status){ if (body) { try { window.simpleCheckout.purchaseRequest = JSON.parse(body); } catch (e) { window.blzErr = 'json:' + e; } } else { window.blzErr = 'status ' + status; } }); }
function diag(){ var d = {}; try { d.scene = typeof window.scene; d.keys = window.scene ? Object.getOwnPropertyNames(window.scene).slice(0, 40) : []; d.pr = window.scene ? String(window.scene.purchaseRequest).slice(0, 300) : ''; d.gpr = typeof window.purchaseRequest + ':' + String(window.purchaseRequest).slice(0, 300); d.win = Object.getOwnPropertyNames(window).filter(function(k){ return /purchase|checkout|scene|navbar|bnl|wow|blizz|order/i.test(k); }); d.sc = window.simpleCheckout ? Object.keys(window.simpleCheckout) : []; d.blz = String(window.blzErr || ''); d.href = location.href; d.ua = navigator.userAgent; } catch (e) { d.err = String(e); } return d; }
// Si esta pagina se carga en el marco de la barra de navegacion (3.4.3 usa la misma URL para
// todo), no hay que redirigir ese marco al checkout: solo el marco principal.
if (window.top !== window.self) { document.body.innerHTML = ''; }
var tries = 0; (function tick(){ if (window.top !== window.self) return; askBlzData(); if (go()) return; if (++tries > 40) { var img = new Image(); img.src = ${jsStr(cfg.publicUrl)} + '/shop/simplecheckout/diag?d=' + encodeURIComponent(JSON.stringify(diag())); setTimeout(function(){ window.location.href = ${jsStr(cfg.publicUrl)} + '/shop/simplecheckout/error?error=no_purchase_request'; }, 500); return; } setTimeout(tick, 250); })();
</script>`, { tema: "claro" });
}

function checkoutPage(locale, order, product, sumupCheckoutId, q) {
  const t = tr(locale);
  const price = fmtPrice(order.price, order.currency, locale);
  const api = { token: q.token, ext: q.ext, sig: q.sig };
  return page(t.title, `<div class="card checkout"><h1>${esc(cfg.shopName)} · ${esc(t.title)}</h1>
<div class="prod"><div><div class="n">${esc(product.name)}</div><div class="d">${esc(String(product.description || "").slice(0, 400))}</div></div><div class="p">${esc(price)}</div></div>
<div class="notas"><p>${esc(t.entrega)}</p><p>${esc(t.secure)}</p></div>
<div id="pay"><div id="sumup-card"></div></div>
<div id="status"></div>
<div class="row"><button class="sec" id="cancel" onclick="cancelOrder()">${esc(t.cancel)}</button></div>
</div>
<script src="${esc(cfg.sumup.sdk)}"></script>
<script>${SCENE_JS}
var API = ${JSON.stringify(api)}, T = ${JSON.stringify({ processing: t.processing, paid: t.paid, delivered: t.delivered, failed: t.failed, retry: t.retry, cancelled: t.cancelled, generic: t.err.generic })};
var done = false, polling = false, pollTimer = null, pollCount = 0;
function setStatus(html, cls){ var s = document.getElementById('status'); s.innerHTML = html ? '<div class="msg ' + (cls || '') + '">' + html + '</div>' : ''; }
function post(path, body){ return fetch(path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(Object.assign({}, API, body || {})) }).then(function(r){ return r.json(); }); }
function finish(orderId){
  if (done) return; done = true;
  document.getElementById('pay').style.display = 'none'; document.getElementById('cancel').style.display = 'none';
  setStatus('<b>' + T.paid + '</b><br>' + T.delivered, 'ok');
  // El cliente manda CMSG_BATTLE_PAY_PURCHASE_SUBMITTED (GlobalOrderId + externalTransactionId) y el
  // worldserver comprueba en la base de datos que el pedido está pagado antes de entregar.
  notifyClient('notifyPurchaseSubmitted', 'purchaseSubmitted', { OrderStatus: 1, GlobalOrderId: String(orderId || ''), GiftData: '', globalOrderId: String(orderId || '') });
  notifyClient('notifyOrderComplete', 'orderComplete', { globalOrderId: String(orderId || '') });
  setTimeout(closeWindow, 5000);
}
function failed(msg){
  setStatus((msg || T.failed) + '<br><br><button onclick="location.reload()">' + T.retry + '</button>', 'bad');
}
function poll(){
  if (done || polling) return; polling = true;
  post('/shop/api/status').then(function(r){
    polling = false;
    if (r.status === 'PAID') return finish(r.orderId);
    if (r.status === 'FAILED') return failed();
    if (r.error) return failed(r.message || T.generic);
    if (++pollCount < 40) pollTimer = setTimeout(poll, ${cfg.pollMs});
  }).catch(function(){ polling = false; if (++pollCount < 40) pollTimer = setTimeout(poll, ${cfg.pollMs}); });
}
function cancelOrder(){
  if (done) return closeWindow();
  post('/shop/api/cancel').then(function(){ setStatus(T.cancelled); notifyClient('requestCancelPurchase', 'purchaseCanceledBeforeSubmit', {}); setTimeout(closeWindow, 800); }).catch(closeWindow);
}
(function mount(){
  if (typeof SumUpCard === 'undefined') { failed(T.generic); return; }
  SumUpCard.mount({
    id: 'sumup-card',
    checkoutId: ${jsStr(sumupCheckoutId)},
    locale: ${jsStr(locale)},
    showFooter: false,
    onResponse: function(type, body) {
      console.log('sumup ' + type, body);
      if (type === 'success') { setStatus('<span class="spin"></span>' + T.processing); pollCount = 0; poll(); }
      else if (type === 'fail' || type === 'error') { setStatus('<span class="spin"></span>' + T.processing); pollCount = 30; poll(); }
    }
  });
})();
</script>`, { tema: "claro" });
}

function paidPage(locale, order) {
  const t = tr(locale);
  return page(t.title, `<div class="card"><h1>${esc(cfg.shopName)}</h1><div class="msg ok"><b>${esc(t.already)}</b><br>${esc(t.delivered)}</div></div>
<script>${SCENE_JS}
notifyClient('notifyPurchaseSubmitted', 'purchaseSubmitted', { OrderStatus: 1, GlobalOrderId: ${jsStr(order.web_order_id || "")}, GiftData: '', globalOrderId: ${jsStr(order.web_order_id || "")} });
notifyClient('notifyOrderComplete', 'orderComplete', { globalOrderId: ${jsStr(order.web_order_id || "")} });
setTimeout(closeWindow, 5000);
</script>`, { tema: "claro" });
}

async function handleCheckout(q, res) {
  const locale = normLocale(q.locale);
  const a = await authorize(q);
  if (a.error) return send(res, a.error === "order_not_found" ? 404 : 403, errorPage(locale, a.error));
  const { order, product } = a;

  if (order.status === ST.Paid || order.status === ST.Delivered) return send(res, 200, paidPage(locale, order));
  if (order.status === ST.Failed) return send(res, 410, errorPage(locale, "order_failed"));

  // ¿Ya había un checkout de SumUp para este pedido? (recarga, ventana cerrada a medias...)
  let sumupId = sumupIdOf(order);
  if (sumupId) {
    const st = await refreshFromSumup(order);
    if (st === "PAID") return send(res, 200, paidPage(locale, order));
    if (st !== "PENDING") sumupId = "";
  }
  if (!sumupId) {
    let chk;
    try { chk = await sumupCreateCheckout(order, product); }
    catch (e) {
      log(`SumUp: no se pudo crear el checkout del pedido ${order.external_id}:`, e.message);
      return send(res, 502, errorPage(locale, "sumup"));
    }
    sumupId = chk.id;
    await setPaymentRef(order, `sumup:${sumupId}`);
    log(`pedido ${order.external_id}: checkout SumUp ${sumupId} (${order.price} ${order.currency}) cuenta ${order.account} producto ${order.product_id} "${product.name}"`);
  }
  send(res, 200, checkoutPage(locale, order, product, sumupId, q));
}

async function handleApi(pathname, req, res) {
  let body;
  try { body = JSON.parse((await readBody(req)) || "{}"); } catch { return sendJson(res, 400, { error: "bad_json" }); }
  const a = await authorize(body);
  if (a.error) return sendJson(res, 403, { error: a.error });
  const { order } = a;

  if (pathname === "/shop/api/status") {
    if (order.status === ST.Paid || order.status === ST.Delivered) return sendJson(res, 200, { status: "PAID", orderId: order.web_order_id });
    if (order.status === ST.Failed) return sendJson(res, 200, { status: "FAILED" });
    const st = await refreshFromSumup(order);
    if (st === "PAID") return sendJson(res, 200, { status: "PAID", orderId: order.web_order_id });
    return sendJson(res, 200, { status: st === "PENDING" ? "PENDING" : st === "NONE" ? "PENDING" : "FAILED" });
  }
  if (pathname === "/shop/api/cancel") {
    if (order.status === ST.Created) {
      // por si el pago entró justo antes de cancelar
      if ((await refreshFromSumup(order)) === "PAID") return sendJson(res, 200, { status: "PAID", orderId: order.web_order_id });
      await markFailed(order, "cancelado por el jugador");
    }
    return sendJson(res, 200, { status: order.status === ST.Failed ? "CANCELLED" : "PAID", orderId: order.web_order_id });
  }
  sendJson(res, 404, { error: "not_found" });
}

const bugs = createBugs({ cfg, log });
log(bugs.enabled ? `fallos: se abriran incidencias en ${cfg.github.repo}` : "fallos: sin GITHUB_BUGS_REPO/TOKEN, la opcion de reportar no se muestra");
const handleSupport = createSupport({ db, cfg, log, esc, page, send, sendJson, readBody, bugs });
const handleRaf = createRaf({ db, cfg, log, esc, page, send, readBody });
const handleAuthenticator = createAuthenticator({ cfg, esc, page, send });

// Emisor de los codigos del segundo factor por SMS. No sirve rutas: solo vacia la cola
// que deja el bnetserver en auth.battlenet_account_sms.
createSms({ db, cfg, log });

const server = createServer({ cert: fs.readFileSync(cfg.certFile), key: fs.readFileSync(cfg.keyFile) }, async (req, res) => {
  const url = new URL(req.url, cfg.publicUrl);
  const q = Object.fromEntries(url.searchParams);
  const p = url.pathname.replace(/\/+$/, "") || "/";
  const orig = req.headers["x-wow-original-url"];

  // TODA peticion, venga marcada o no. El registro de mas abajo solo anota las que llegan
  // SIN marca del juego, asi que una que si la traiga no aparecia en ninguna parte: al
  // mirar el log parecia que no habia llegado nadie cuando podia haber llegado de todo.
  log(`<- ${req.socket?.remoteAddress || "?"} ${req.method} ${p}`);

  // Este servicio es para dentro del juego. Todo lo que no sea el RAF (cuyas invitaciones
  // se abren desde el correo, fuera del juego) o /health se sirve solo a quien llega con
  // credenciales del juego: el token SSO en la URL, la cookie que deja ese token, o la
  // cabecera que pone el shim al reescribir las URL de Blizzard.
  //
  // Para el resto se responde 404, el mismo que una ruta inexistente, para no anunciar que
  // aqui hay nada. Ojo: esto no es la frontera de seguridad, que sigue siendo el token
  // (una cabecera la pone cualquiera); esto evita que la tienda y el soporte esten a la
  // vista de quien pase por el puerto.
  const abiertoAlPublico = p === "/health" || p === "/raf" || p.startsWith("/raf/") || p.startsWith("/oauth/") || p.startsWith("/1.1/") || p === "/twitter" || p.startsWith("/twitter/");
  if (!abiertoAlPublico) {
    const cookies = req.headers.cookie || "";
    const vieneDelJuego = Boolean(orig) || Boolean(q.token) || cookies.includes("wowsso=");
    if (!vieneDelJuego) {
      // EN OBSERVACION, sin bloquear todavia.
      //
      // El primer intento daba por hecho que el shim pone X-WoW-Original-Url en cada
      // peticion reescrita, como dice el comentario de cef_shim.ini. No es asi: el juego
      // pidio /shop/simplecheckout/loading sin esa cabecera y el guardia le devolvio un 404,
      // dejando la tienda muerta dentro del juego.
      //
      // Asi que aqui solo se anota que llego y con que cabeceras, para construir el guardia
      // sobre algo comprobado en vez de sobre una suposicion. Mientras tanto no se corta
      // nada: la frontera de verdad sigue siendo el token, que ya validaban los modulos.
      const interesantes = ["user-agent", "referer", "origin", "accept-language", "sec-fetch-site", "sec-fetch-mode", "sec-fetch-dest", "x-wow-original-host", "x-wow-original-url"];
      const vistas = interesantes
        .filter((h) => req.headers[h])
        .map((h) => `${h}=${String(req.headers[h]).slice(0, 160)}`)
        .join(" | ");
      // La IP hace falta para saber DE DONDE llega: sin ella, con dos equipos
      // probando a la vez no hay forma de distinguir cual llego y cual no.
      const de = req.socket?.remoteAddress || "?";
      log(`SIN-MARCA ${de} ${req.method} ${p} :: ${vistas || "(ninguna cabecera de interes)"}`);
    }
  }

  try {
    if (req.method === "GET" && (p === "/nydus/Bnet/client/purchase/jsutil" || p === "/Bnet/client/purchase/jsutil"))
      return send(res, 200, checkoutUrlJs(), "application/javascript; charset=utf-8");
    if (req.method === "GET" && p === "/shop/simplecheckout/loading") return send(res, 200, loadingPage());
    // /nb: alias corto para el hueco de la URL del navbar en el ejecutable 3.4.3 (55 bytes)
    if (req.method === "GET" && (p === "/shop/simplecheckout/navbar" || p === "/nb")) return send(res, 200, page(cfg.shopName, "", { marca: false, tema: "claro" }));
    if (req.method === "GET" && p === "/shop/simplecheckout/diag") { log(`diag loading: ${String(q.d || "").slice(0, 1500)}`); return sendJson(res, 200, { ok: true }); }
    if (req.method === "GET" && p === "/shop/simplecheckout/error") return send(res, 200, errorPage(normLocale(q.locale), q.error && I18N.en.err[q.error] ? q.error : "generic"));
    if (req.method === "GET" && p === "/shop/checkout") return await handleCheckout(q, res);
    if (req.method === "POST" && p.startsWith("/shop/api/")) return await handleApi(p, req, res);
    if (req.method === "GET" && p === "/health") return sendJson(res, 200, { ok: true });

    // El catalogo, para la web. Es de solo lectura y no dice nada que no este ya en el
    // escaparate, asi que no pide credenciales; la web lo pide por dentro (127.0.0.1).
    if (req.method === "GET" && p === "/shop/api/catalogo")
      return sendJson(res, 200, { productos: await catalogo(q.lang || q.locale) });
    if (p.startsWith("/oauth/") || p.startsWith("/1.1/") || p === "/twitter" || p.startsWith("/twitter/")) return await twitter.handle(req, res, p, q);
    if (p.startsWith("/raf/")) return await handleRaf(req, res, p, q);
    if (p.startsWith("/account/authenticator") && await handleAuthenticator(req, res, p, q)) return;
    if (p.startsWith("/support") || p === "/login" || p.startsWith("/login/")) {
      await handleSupport(req, res, p, q);
      if (res.writableEnded) return;
    }
    log(`404 ${req.method} ${p}${orig ? ` (original ${orig})` : ""}`);
    send(res, 404, page(cfg.shopName, `<div class="card"><h1>${esc(cfg.shopName)}</h1><div class="msg">404</div></div>`));
  } catch (e) {
    log(`error ${req.method} ${p}:`, e);
    if (!res.headersSent) send(res, 500, page(cfg.shopName, `<div class="card"><h1>${esc(cfg.shopName)}</h1><div class="msg bad">${esc(I18N.es.err.generic)}</div></div>`));
  }
});

// Barrido: pedidos con checkout de SumUp que quedaron en Created/Failed (ventana cerrada antes de
// ver el PAID, cancelación del cliente...) se reconcilian con SumUp; si están pagados pasan a Paid
// y el worldserver los entrega la próxima vez que el jugador abra la tienda.
async function reconcilePending() {
  let rows;
  try {
    [rows] = await db.query(`SELECT * FROM ${T.order} WHERE status IN (?, ?) AND payment_ref LIKE 'sumup:%' AND created > NOW() - INTERVAL 2 DAY ORDER BY id DESC LIMIT 50`, [ST.Created, ST.Failed]);
  } catch (e) { log("reconcile: error consultando pedidos:", e.message || e); return; }
  for (const order of rows) {
    try {
      const st = await refreshFromSumup(order);
      if (st === "PAID") log(`reconcile: pedido ${order.external_id} estaba pagado en SumUp, marcado Paid`);
    } catch (e) { log(`reconcile: pedido ${order.external_id}:`, e.message || e); }
  }
}
setTimeout(reconcilePending, 15000);
setInterval(reconcilePending, 5 * 60 * 1000).unref();

// Devoluciones y contracargos: los pedidos entregados se consultan en SumUp (transacción por
// transaction_code, guardado en payment_ref como ";tx=...") durante 180 días. Si la transacción
// aparece devuelta o con contracargo, el pedido pasa a Revoked y la distribución (boost) queda
// revocada; el worldserver bloquea al personaje en su siguiente pasada (BattlePayRevocation) y
// el jugador puede levantarlo gastando otro boost igual. Solo se marca; nunca se devuelve nada
// automáticamente desde aquí.
const sumupTxOf = (order) => { const m = /;tx=([^;]+)/.exec(order.payment_ref || ""); return m ? m[1] : (order.web_order_id || ""); };
function sumupLooksRefunded(tx) {
  if (!tx) return false;
  const st = String(tx.status || "").toUpperCase();
  if (st === "REFUNDED" || st === "CHARGEBACK" || st === "CHARGE_BACK") return true;
  const amount = Number(tx.amount || 0), refunded = Number(tx.refunded_amount || 0);
  if (refunded > 0 && refunded >= amount) return true;
  return (tx.events || []).some((e) => ["REFUND", "CHARGE_BACK", "CHARGEBACK"].includes(String(e.type || "").toUpperCase()) && String(e.status || "SUCCESSFUL").toUpperCase() === "SUCCESSFUL");
}
async function reconcileRefunds() {
  let rows;
  try {
    [rows] = await db.query(`SELECT * FROM ${T.order} WHERE status IN (?, ?) AND payment_ref LIKE 'sumup:%' AND paid > NOW() - INTERVAL 180 DAY ORDER BY id DESC LIMIT 100`, [ST.Paid, ST.Delivered]);
  } catch (e) { log("devoluciones: error consultando pedidos:", e.message || e); return; }
  for (const order of rows) {
    const code = sumupTxOf(order);
    if (!code) continue;
    let tx;
    try {
      const r = await sumup("GET", `/v0.1/me/transactions?transaction_code=${encodeURIComponent(code)}`);
      tx = Array.isArray(r) ? r[0] : (r && r.items ? r.items[0] : r);
    } catch (e) {
      if (e.status !== 404) log(`devoluciones: pedido ${order.external_id} tx ${code}:`, e.message || e);
      continue;
    }
    if (!sumupLooksRefunded(tx)) continue;
    try {
      const [r] = await db.query(`UPDATE ${T.order} SET status = ?, revoked = NOW(), payment_ref = CONCAT(payment_ref, ';revoked') WHERE id = ? AND status IN (?, ?)`, [ST.Revoked, order.id, ST.Paid, ST.Delivered]);
      if (!r.affectedRows) continue;
      await db.query(`UPDATE \`${cfg.db.auth}\`.battlepay_distribution SET revoked = 1 WHERE purchase_id = ?`, [order.id]);
      log(`pedido ${order.external_id} DEVUELTO/CONTRACARGO en SumUp (tx ${code}, estado ${tx.status || "?"}): cuenta ${order.account} producto ${order.product_id} -> revocado`);
    } catch (e) { log(`devoluciones: pedido ${order.external_id}: error marcando:`, e.message || e); }
  }
}
setTimeout(reconcileRefunds, 60000);
setInterval(reconcileRefunds, 30 * 60 * 1000).unref();

server.listen(cfg.port, cfg.bind, () => log(`tienda escuchando en https://${cfg.bind}:${cfg.port} (público ${cfg.publicUrl}), SumUp ${cfg.sumup.api} comercio ${cfg.sumup.merchant}`));
for (const sig of ["SIGINT", "SIGTERM"]) process.on(sig, () => { server.close(); db.end().finally(() => process.exit(0)); });
