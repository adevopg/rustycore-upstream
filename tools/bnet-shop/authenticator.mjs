// Página del autenticador de Battle.net (Google Authenticator / Authy / cualquier app TOTP).
//
// El botón «Activar» de la mochila del cliente (popup BACKPACK_INCREASE_SIZE → LoadURLIndex(41) →
// VISITABLE_URL41 de GlobalStrings.db2, hotfixeada a esta URL) abre el navegador EXTERNO del
// sistema, sin SSO del juego, así que aquí no hay sesión: la vinculación se hace desde el juego
// con `.bnetaccount authenticator on`, que genera la clave y la muestra en el chat. Esta página
// solo explica los pasos y convierte esa clave en un código QR, todo en el navegador (la clave
// se escribe en un campo y el QR se genera con JS: nunca se envía al servidor ni queda en logs).
//
// Rutas: GET /account/authenticator  (también /account/authenticator/<lang>)
//         GET /account/authenticator/qrcode.js  (la librería del QR, servida por nosotros)
//
// LA LIBRERÍA DEL QR SE SIRVE DE AQUÍ, NO DE UN CDN AJENO. Antes la página la cargaba de
// cdnjs.cloudflare.com; cuando ese script no llegaba —cortafuegos, sin salida a internet, el
// navegador del juego— el QR simplemente no se dibujaba y la página no decía nada, que es la
// peor forma de fallar. Ahora viene del mismo sitio que la página: si se ve la página, se ve
// el QR. Y si aun así fallara, se dice en pantalla en vez de callar.

import { readFileSync } from "node:fs";

// Se lee una vez al arrancar: son 20 KB y no cambia nunca.
const QRCODE_JS = readFileSync(new URL("./vendor/qrcode.min.js", import.meta.url));

const I18N = {
  es: {
    title: "Autenticador de Battle.net", lead: "Protege tu cuenta con un código temporal de tu móvil y consigue 4 huecos más en la mochila.",
    steps: "Cómo activarlo",
    s1: "Instala <b>Google Authenticator</b>, Authy o cualquier app de códigos TOTP en tu móvil.",
    s2: "En el juego escribe en el chat: <code>.bnetaccount authenticator on</code>. Verás una clave de 32 letras y números.",
    s3: "Escribe esa clave aquí abajo para ver el código QR y escanéalo con la app (o en la app elige «Introducir clave de configuración» y tecléala).",
    s4: "Escribe en el chat <code>.bnetaccount authenticator confirm 123456</code> con el código de 6 cifras que muestra la app. ¡Listo! Los 4 huecos aparecen al momento.",
    keyLabel: "Clave que muestra el juego", keyPh: "ABCD EFGH IJKL MNOP QRST UVWX YZ23 4567",
    account: "Cuenta (opcional, el nombre que verás en la app)", accountPh: "tu@email.com",
    show: "Mostrar QR", badKey: "La clave debe tener letras A-Z y números 2-7 (normalmente 32 caracteres).",
    confuso: "Ojo: la clave NO lleva ceros, unos, ochos ni nueves. Lo que has escrito como «{malo}» es casi seguro «{bueno}».",
    corto: "Faltan caracteres: la clave tiene 32 y has escrito {n}.",
    scan: "Escanea este código con la app y luego confirma en el juego con el código de 6 cifras.",
    remove: "Para quitarlo: <code>.bnetaccount authenticator off 123456</code> (con el código de la app). Los objetos que hubiera en los 4 huecos extra se envían por correo.",
    login: "Al entrar al juego se pedirá el código del autenticador. Marca «Recordar este equipo» para que no te lo pida durante 30 días en ese PC.",
    lost: "¿Has perdido el móvil? Contacta con un Game Master (abre un ticket desde el juego) para que lo desvinculen.",
    noQr: "No se pudo dibujar el código QR. Usa el enlace o la clave de abajo: en la app elige «Introducir clave de configuración».",
    copy: "Copiar la dirección",
    copied: "Copiado",
    openApp: "Abrir en la app del móvil",
  },
  en: {
    title: "Battle.net Authenticator", lead: "Protect your account with a one-time code from your phone and get 4 extra backpack slots.",
    steps: "How to enable it",
    s1: "Install <b>Google Authenticator</b>, Authy or any TOTP code app on your phone.",
    s2: "In game type in chat: <code>.bnetaccount authenticator on</code>. You will see a 32 character key.",
    s3: "Type that key below to get the QR code and scan it with the app (or choose “Enter a setup key” in the app and type it).",
    s4: "Type in chat <code>.bnetaccount authenticator confirm 123456</code> with the 6 digit code the app shows. Done! The 4 slots appear right away.",
    keyLabel: "Key shown in game", keyPh: "ABCD EFGH IJKL MNOP QRST UVWX YZ23 4567",
    account: "Account (optional, the name you will see in the app)", accountPh: "you@email.com",
    show: "Show QR", badKey: "The key only has letters A-Z and digits 2-7 (usually 32 characters).",
    confuso: "Careful: the key has no zeros, ones, eights or nines. What you typed as “{malo}” is almost certainly “{bueno}”.",
    corto: "Characters missing: the key has 32 and you typed {n}.",
    scan: "Scan this code with the app, then confirm in game with the 6 digit code.",
    remove: "To remove it: <code>.bnetaccount authenticator off 123456</code> (with the app code). Items in the 4 extra slots are mailed to you.",
    login: "When logging in you will be asked for the authenticator code. Tick “Remember this device” to skip it for 30 days on that PC.",
    lost: "Lost your phone? Contact a Game Master (open a ticket from the game) to unlink it.",
    noQr: "The QR code could not be drawn. Use the link or the key below: in the app choose “Enter a setup key”.",
    copy: "Copy the address",
    copied: "Copied",
    openApp: "Open in the phone app",
  },
};

export function createAuthenticator({ cfg, esc, page, send }) {
  const issuer = process.env.AUTHENTICATOR_ISSUER || cfg.shopName || "LegionCore";
  const langOf = (l) => (String(l || "").toLowerCase().startsWith("en") ? "en" : "es");

  function authenticatorPage(lang) {
    const t = I18N[lang] || I18N.en;
    return page(t.title, `<div class="card" style="max-width:640px">
<h1>${esc(t.title)}</h1>
<p>${esc(t.lead)}</p>
<h2>${esc(t.steps)}</h2>
<ol>
<li>${t.s1}</li>
<li>${t.s2}</li>
<li>${t.s3}</li>
<li>${t.s4}</li>
</ol>
<form id="f" onsubmit="return showQr()">
<label>${esc(t.keyLabel)}<br><input id="key" placeholder="${esc(t.keyPh)}" autocomplete="off" spellcheck="false" style="width:100%;font-family:monospace;font-size:16px;padding:8px;margin:6px 0 12px;background:#141a2a;color:#e6e9f0;border:1px solid #2a3350;border-radius:6px"></label>
<label>${esc(t.account)}<br><input id="acc" placeholder="${esc(t.accountPh)}" autocomplete="off" style="width:100%;padding:8px;margin:6px 0 12px;background:#141a2a;color:#e6e9f0;border:1px solid #2a3350;border-radius:6px"></label>
<div class="row"><span id="err" class="bad"></span><button type="submit">${esc(t.show)}</button></div>
</form>
<div id="qrbox" style="display:none;text-align:center;margin-top:16px">
<div id="qr" style="display:inline-block;background:#fff;padding:12px;border-radius:8px;min-height:24px"></div>
<p id="qrerr" class="bad" style="display:none"></p>
<p>${esc(t.scan)}</p>
<p><a id="applink" href="#" style="display:inline-block;margin-bottom:8px">${esc(t.openApp)}</a></p>
<p><code id="uri" style="word-break:break-all;font-size:12px"></code>
<button type="button" id="copy" style="margin-left:8px">${esc(t.copy)}</button></p>
</div>
<hr style="border:0;border-top:1px solid #2a3350;margin:20px 0">
<p class="msg">${t.login}</p>
<p class="msg">${t.remove}</p>
<p class="msg">${t.lost}</p>
</div>
<script src="/account/authenticator/qrcode.js"></script>
<script>
var ISSUER = ${JSON.stringify(issuer)};
function showQr(){
  var key = document.getElementById('key').value.replace(/[\\s=-]/g, '').toUpperCase();
  var acc = document.getElementById('acc').value.trim() || 'account';
  var err = document.getElementById('err');
  /*
   * La clave hay que TECLEARLA: el chat del juego no deja copiar, asi que el jugador la lee de
   * la pantalla. Con 32 caracteres al azar, confundir uno es lo normal, y el base32 no tiene
   * ceros, unos, ochos ni nueves: si aparece alguno, se sabe exactamente por que letra iba.
   * Decirlo ahorra el "me dice que el codigo es incorrecto" media hora despues.
   */
  var PARECIDOS = { '0': 'O', '1': 'I', '8': 'B', '9': 'G' };
  for (var c in PARECIDOS) {
    if (key.indexOf(c) >= 0) {
      err.textContent = ${JSON.stringify(t.confuso)}.replace('{malo}', c).replace('{bueno}', PARECIDOS[c]);
      return false;
    }
  }

  if (!/^[A-Z2-7]*$/.test(key)) { err.textContent = ${JSON.stringify(t.badKey)}; return false; }
  if (key.length !== 32) {
    err.textContent = key.length < 16
      ? ${JSON.stringify(t.badKey)}
      : ${JSON.stringify(t.corto)}.replace('{n}', String(key.length));
    return false;
  }
  err.textContent = '';
  var uri = 'otpauth://totp/' + encodeURIComponent(ISSUER) + ':' + encodeURIComponent(acc) + '?secret=' + key + '&issuer=' + encodeURIComponent(ISSUER) + '&algorithm=SHA1&digits=6&period=30';
  var box = document.getElementById('qr'); box.innerHTML = '';
  var fallo = document.getElementById('qrerr');
  fallo.style.display = 'none';

  // Si el QR no se puede dibujar se DICE. Antes esto era un if (window.QRCode) a secas: sin
  // la libreria no salia el codigo y la pagina se quedaba tan tranquila.
  //
  // Y lo que decide si ha salido es MIRAR EL RECUADRO, no si hubo excepcion: la libreria
  // dibuja y despues hace un retoque de centrado que puede fallar por su cuenta, y entonces
  // el aviso saldria con el codigo delante, ya dibujado.
  try {
    if (window.QRCode) new QRCode(box, { text: uri, width: 220, height: 220, correctLevel: QRCode.CorrectLevel.M });
  } catch (e) {}

  if (!box.innerHTML) {
    fallo.textContent = ${JSON.stringify(t.noQr)};
    fallo.style.display = 'block';
  }

  document.getElementById('uri').textContent = uri;
  document.getElementById('applink').href = uri;
  var caja = document.getElementById('qrbox');
  caja.style.display = 'block';
  // Que se vea que ha pasado algo aunque el recuadro caiga por debajo de la pantalla.
  if (caja.scrollIntoView) caja.scrollIntoView({ block: 'nearest' });
  return false;
}

document.getElementById('copy').onclick = function () {
  var boton = this;
  var texto = document.getElementById('uri').textContent;
  var hecho = function () { boton.textContent = ${JSON.stringify(t.copied)}; };
  if (navigator.clipboard) { navigator.clipboard.writeText(texto).then(hecho, function(){}); return; }
  // Sin portapapeles moderno (el navegador del juego es viejo): se selecciona y ya copia el.
  var sel = window.getSelection();
  var r = document.createRange();
  r.selectNodeContents(document.getElementById('uri'));
  sel.removeAllRanges(); sel.addRange(r);
  try { document.execCommand('copy'); hecho(); } catch (e) {}
};
// la clave puede venir en el fragmento (#KEY) para no pasar por el servidor
if (location.hash.length > 1) { document.getElementById('key').value = decodeURIComponent(location.hash.slice(1)); showQr(); }
</script>`);
  }

  return async function handleAuthenticator(req, res, p, q) {
    // La libreria del QR, desde el mismo sitio que la pagina. Va antes que la ruta de idioma
    // porque esa no admite puntos en el nombre y no llegaria a coincidir.
    if (req.method === "GET" && p === "/account/authenticator/qrcode.js") {
      send(res, 200, QRCODE_JS, "application/javascript; charset=utf-8",
           { "Cache-Control": "public, max-age=86400" });
      return true;
    }

    const m = /^\/account\/authenticator(?:\/([a-zA-Z-]+))?$/.exec(p);
    if (!m || req.method !== "GET") return false;
    const lang = langOf(m[1] || q.locale || q.lang || req.headers["accept-language"]);
    send(res, 200, authenticatorPage(lang));
    return true;
  };
}
