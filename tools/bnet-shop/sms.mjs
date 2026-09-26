/*
 * Envío de los códigos del segundo factor por SMS (Twilio).
 *
 * El bnetserver no manda el SMS: no tiene cliente HTTP saliente. Deja la fila en
 * auth.battlenet_account_sms y aquí se recoge y se envía, igual que ya se hace con los
 * códigos de recuperación por correo en raf.mjs.
 *
 * status:  0 pendiente · 1 enviado · 2 no se pudo (sin configurar / número rechazado)
 *          3 fallo definitivo tras varios intentos
 *
 * A diferencia del correo, aquí el código NUNCA se escribe en el log, ni siquiera cuando
 * Twilio no está configurado: un código de inicio de sesión en un fichero legible es una
 * puerta abierta a la cuenta. Se marca la fila y punto.
 */

const API = "https://api.twilio.com/2010-04-01";

export function createSms({ db, cfg, log }) {
  const env = process.env;
  const twilio = env.TWILIO_ACCOUNT_SID
    ? {
        sid: env.TWILIO_ACCOUNT_SID,
        token: env.TWILIO_AUTH_TOKEN || "",
        from: env.TWILIO_FROM || "",
        messagingService: env.TWILIO_MESSAGING_SERVICE_SID || "",
      }
    : null;

  const T = `\`${cfg.db.auth}\`.battlenet_account_sms`;
  const ST = { Pendiente: 0, Enviado: 1, NoSePudo: 2, Fallido: 3 };
  const MAX_INTENTOS = 3;
  const intentos = new Map();

  const TEXTOS = {
    es: (code, mins) =>
      `${cfg.shopName}: tu codigo de verificacion es ${code}. Caduca en ${mins} minutos. Si no has sido tu, cambia tu contrasena.`,
    en: (code, mins) =>
      `${cfg.shopName}: your verification code is ${code}. It expires in ${mins} minutes. If this was not you, change your password.`,
  };

  async function enviarTwilio(destino, texto) {
    const cuerpo = new URLSearchParams({ To: destino, Body: texto });
    if (twilio.messagingService) cuerpo.set("MessagingServiceSid", twilio.messagingService);
    else cuerpo.set("From", twilio.from);

    const r = await fetch(`${API}/Accounts/${encodeURIComponent(twilio.sid)}/Messages.json`, {
      method: "POST",
      headers: {
        Authorization: "Basic " + Buffer.from(`${twilio.sid}:${twilio.token}`).toString("base64"),
        "Content-Type": "application/x-www-form-urlencoded",
      },
      body: cuerpo.toString(),
      signal: AbortSignal.timeout(15000),
    });

    const texto_ = await r.text();
    let json;
    try {
      json = JSON.parse(texto_);
    } catch {
      json = { message: texto_.slice(0, 200) };
    }

    if (!r.ok) {
      const e = new Error(`Twilio ${r.status}: ${json.message || texto_.slice(0, 200)}`);
      e.codigo = json.code;
      // 21608: cuenta de prueba, el numero de destino no esta verificado.
      // 21211: numero mal formado. Ninguno de los dos mejora reintentando.
      e.definitivo = json.code === 21608 || json.code === 21211 || json.code === 21610;
      throw e;
    }
    return json.sid || "";
  }

  async function enviarPendientes() {
    let filas;
    try {
      [filas] = await db.query(
        `SELECT id, phone, code, expires, created, lang FROM ${T}
          WHERE status = ? AND code <> '' AND expires > UNIX_TIMESTAMP()
          ORDER BY id LIMIT 20`,
        [ST.Pendiente],
      );
    } catch (e) {
      log("sms: error consultando la cola:", e.message || e);
      return;
    }

    for (const fila of filas) {
      if (!twilio) {
        // Sin configurar: se marca y se sigue. El codigo se borra igualmente para que no
        // quede en la base de datos mas tiempo del necesario.
        await db.query(`UPDATE ${T} SET status = ?, code = '', error = ? WHERE id = ? AND status = ?`,
          [ST.NoSePudo, "TWILIO_ACCOUNT_SID sin configurar", fila.id, ST.Pendiente]);
        log(`sms: Twilio no configurado; codigo ${fila.id} descartado sin enviar`);
        continue;
      }

      const minutos = Math.max(1, Math.round((fila.expires - fila.created) / 60));
      const texto = (TEXTOS[fila.lang] || TEXTOS.es)(fila.code, minutos);

      try {
        const ref = await enviarTwilio(fila.phone, texto);
        await db.query(`UPDATE ${T} SET status = ?, code = '', provider_ref = ? WHERE id = ? AND status = ?`,
          [ST.Enviado, ref, fila.id, ST.Pendiente]);
        intentos.delete(fila.id);
        // Ni el codigo ni el numero completo: solo lo justo para seguir el rastro.
        log(`sms: codigo ${fila.id} enviado a ${fila.phone.slice(0, 4)}…${fila.phone.slice(-3)} (${ref})`);
      } catch (e) {
        const n = (intentos.get(fila.id) || 0) + 1;
        intentos.set(fila.id, n);
        log(`sms: codigo ${fila.id}, intento ${n}: ${e.message}`);
        if (e.definitivo || n >= MAX_INTENTOS) {
          await db.query(`UPDATE ${T} SET status = ?, code = '', error = ? WHERE id = ? AND status = ?`,
            [e.definitivo ? ST.NoSePudo : ST.Fallido, String(e.message).slice(0, 255), fila.id, ST.Pendiente]);
          intentos.delete(fila.id);
        }
      }
    }
  }

  // Limpieza: los codigos caducados sin enviar no sirven de nada y no deben quedarse.
  async function limpiar() {
    try {
      await db.query(`UPDATE ${T} SET status = ?, code = '' WHERE status = ? AND expires <= UNIX_TIMESTAMP()`,
        [ST.NoSePudo, ST.Pendiente]);
    } catch { /* sin importancia */ }
  }

  if (!twilio) log("sms: TWILIO_ACCOUNT_SID sin configurar; los codigos del segundo factor no se enviaran");
  else log(`sms: Twilio listo (${twilio.messagingService ? "servicio " + twilio.messagingService : "desde " + twilio.from})`);

  // Cada 10 s: un SMS tiene que llegar mientras el jugador mira la pantalla.
  setTimeout(enviarPendientes, 4000);
  setInterval(enviarPendientes, 10 * 1000).unref();
  setInterval(limpiar, 5 * 60 * 1000).unref();

  return { enviarPendientes };
}
