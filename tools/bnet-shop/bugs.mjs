// Reportar fallos del juego desde el cliente, abriendo una incidencia en GitHub.
//
// POR QUE ASÍ. Un ticket de GM es para el problema de un jugador —«he perdido un objeto»,
// «estoy atascado»— y lo resuelve un GM a mano. Un fallo del servidor es otra cosa: no lo
// arregla nadie en caliente, hay que apuntarlo, y el sitio donde se apunta el trabajo de este
// servidor es GitHub. Mezclarlos llena la cola de tickets de cosas que ningún GM puede hacer.
//
// QUIÉN ABRE LA INCIDENCIA. El servidor, con un token suyo. El jugador no necesita cuenta en
// GitHub ni acceso de ninguna clase al repositorio: escribe en una página del juego y aquí se
// traduce a una llamada a la API. Así no hay que repartir permisos sobre el código a nadie.
//
// EL TOKEN. Uno "fine-grained" de GitHub, con UN permiso —Issues: Read and write— sobre UN
// repositorio y nada más. Aunque se filtrara no da acceso al código: solo permite abrir y leer
// incidencias de ese repositorio. Va en shop.env (GITHUB_BUGS_TOKEN), que vive fuera del
// repositorio, como el resto de credenciales.
//
// DÓNDE VAN LAS INCIDENCIAS. Lo normal es un repositorio aparte y público —por ejemplo
// nightspire/fallos—, con el código en otro sitio: los jugadores pueden leer lo que se ha
// reportado y ver si su fallo ya está, sin que el código quede expuesto. También vale el
// repositorio del servidor si es privado; entonces nadie ve su propio reporte, pero funciona.
//
// DE QUIÉN ES CADA FALLO. Para poder enseñarle a un jugador los suyos hay que saber quién abrió
// cada uno, y aquí no se guarda nada: la lista se lee de GitHub, que es el único sitio donde
// esto vive. Cada incidencia lleva al final una marca que el markdown renderizado no enseña,
// `<!-- ns:cuenta=N -->`, y de ahí se saca. No dice nada que no estuviera ya escrito arriba en
// el bloque de contexto, así que no añade ninguna fuga; solo evita tener que adivinarlo.
//
// No guardar nada tiene dos ventajas que compensan de sobra la llamada extra: los reportes
// siguen ahí después de reiniciar la tienda, y el estado que ve el jugador —abierto o cerrado,
// y las respuestas— es el de verdad y no una copia que se desincroniza.

const LIMITE_DIARIO = 15;         // reportes por cuenta y día: suficiente para quien juega, no para quien spamea
const TITULO_MAX = 120;
const CUERPO_MAX = 4000;
const DIA = 24 * 3600 * 1000;

const PAGINAS = 3;                // hasta 300 incidencias recientes en la lista
const POR_PAGINA = 100;
const CACHE_MS = 120000;          // la lista se refresca cada dos minutos, no en cada visita
const MAX_MIOS = 20;              // cuántos fallos propios se le enseñan al jugador

// La marca de cuenta, y el bloque de contexto visible por si alguna incidencia se abrió antes
// de que la marca existiera.
const RE_MARCA = /<!--\s*ns:cuenta=(\d+)\s*-->/;
const RE_CONTEXTO = /\*\*Cuenta:\*\*\s*#(\d+)/;

/**
 * Quita lo que podría convertir el texto de un jugador en algo más que texto:
 *
 *   - las menciones (@alguien) avisan por correo a esa persona de GitHub, y eso es un vector
 *     de molestias gratuito; se les mete un carácter invisible para que no enganchen.
 *   - los caracteres de control no pintan nada en una incidencia.
 *
 * No se escapa el markdown: que un jugador pueda poner una lista o negrita en su reporte es
 * bueno, y lo peor que puede hacer es que su propio texto se vea raro.
 */
function limpiar(texto, max) {
  return String(texto || "")
    .replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/g, "")
    .replace(/@(?=[A-Za-z0-9_-])/g, "@\u200B")
    .trim()
    .slice(0, max);
}

const cuentaDe = (cuerpo) => {
  const m = RE_MARCA.exec(cuerpo || "") || RE_CONTEXTO.exec(cuerpo || "");
  return m ? Number(m[1]) : 0;
};

// Lo que escribió el jugador, sin el bloque de contexto ni la marca que le añadimos nosotros:
// es lo que hay que enseñarle de vuelta.
const soloElTexto = (cuerpo) => String(cuerpo || "").split("\n\n---\n<sub>")[0].trim();

export function createBugs({ cfg, log }) {
  const repo = (cfg.github?.repo || "").trim();
  const token = (cfg.github?.token || "").trim();
  const etiquetas = (cfg.github?.labels || "").split(",").map((x) => x.trim()).filter(Boolean);
  const enabled = Boolean(repo && token && /^[^/\s]+\/[^/\s]+$/.test(repo));

  if (repo && !enabled) log(`fallos: GITHUB_BUGS_REPO no tiene forma "duenyo/repositorio": ${repo}`);

  // cuenta -> momentos de sus reportes de esta sesión. Es solo un atajo: el tope de verdad se
  // cuenta sobre lo que hay en GitHub, que es lo que sobrevive a un reinicio.
  const recientes = new Map();

  function apuntar(cuenta) {
    const suyos = (recientes.get(cuenta) || []).filter((t) => Date.now() - t < DIA);
    suyos.push(Date.now());
    recientes.set(cuenta, suyos);
  }

  // ------------------------------------------------------------------ GitHub
  async function pedir(ruta) {
    const r = await fetch(`https://api.github.com${ruta}`, {
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "nightspire-support",
      },
      signal: AbortSignal.timeout(15000),
    });
    if (!r.ok) {
      log(`fallos: GET ${ruta} -> ${r.status} ${(await r.text()).slice(0, 200)}`);
      return null;
    }
    return r.json();
  }

  const normaliza = (x) => ({
    numero: x.number,
    titulo: x.title,
    url: x.html_url,
    estado: x.state,                       // "open" | "closed"
    creado: Date.parse(x.created_at) || 0,
    respuestas: Number(x.comments || 0),
    texto: soloElTexto(x.body),
    cuenta: cuentaDe(x.body),
  });

  // La lista completa, cacheada. `cargando` evita que varias visitas a la vez disparen varias
  // tandas de peticiones; si GitHub falla se conserva la lista anterior en vez de vaciarla.
  let cache = { momento: 0, lista: null, cargando: null };

  async function listaDeIncidencias() {
    if (cache.lista && Date.now() - cache.momento < CACHE_MS) return cache.lista;
    if (cache.cargando) return cache.cargando;

    cache.cargando = (async () => {
      const todo = [];
      for (let pagina = 1; pagina <= PAGINAS; pagina++) {
        const trozo = await pedir(`/repos/${repo}/issues?state=all&per_page=${POR_PAGINA}&page=${pagina}&sort=created&direction=desc`);
        if (!trozo) return cache.lista;                     // fallo de red: lo viejo es mejor que nada
        todo.push(...trozo.filter((x) => !x.pull_request).map(normaliza));
        if (trozo.length < POR_PAGINA) break;
      }
      cache.momento = Date.now();
      cache.lista = todo;
      return todo;
    })();

    try { return await cache.cargando; } finally { cache.cargando = null; }
  }

  /** Cuántos reportes le quedan hoy. Cuenta sobre GitHub, no sobre la memoria del proceso. */
  async function quedan(cuenta) {
    const ahora = Date.now();
    const enMemoria = (recientes.get(cuenta) || []).filter((t) => ahora - t < DIA).length;
    const lista = enabled ? await listaDeIncidencias().catch(() => null) : null;
    const enGitHub = lista ? lista.filter((i) => i.cuenta === Number(cuenta) && ahora - i.creado < DIA).length : 0;
    return Math.max(0, LIMITE_DIARIO - Math.max(enMemoria, enGitHub));
  }

  /** Los fallos que ha reportado esta cuenta, del más nuevo al más viejo. */
  async function mios(cuenta) {
    if (!enabled) return { ok: false, lista: [] };
    const lista = await listaDeIncidencias().catch(() => null);
    if (!lista) return { ok: false, lista: [] };
    return { ok: true, lista: lista.filter((i) => i.cuenta === Number(cuenta)).slice(0, MAX_MIOS) };
  }

  /**
   * Un fallo concreto con sus respuestas. Solo lo devuelve si lo abrió esa cuenta: las
   * incidencias son públicas, pero esta página es «tus fallos» y enseñar aquí el de otro sería
   * confuso, no un descubrimiento.
   */
  async function uno(cuenta, numero) {
    if (!enabled || !Number.isInteger(numero) || numero <= 0) return { ok: false };
    const dato = await pedir(`/repos/${repo}/issues/${numero}`).catch(() => null);
    if (!dato || dato.pull_request || cuentaDe(dato.body) !== Number(cuenta)) return { ok: false };
    const comentarios = (await pedir(`/repos/${repo}/issues/${numero}/comments?per_page=50`).catch(() => null)) || [];
    return {
      ok: true,
      fallo: normaliza(dato),
      respuestas: comentarios.map((c) => ({
        autor: c.user ? c.user.login : "-",
        texto: String(c.body || ""),
        cuando: Date.parse(c.created_at) || 0,
      })),
    };
  }

  /**
   * Abre la incidencia. Devuelve {ok:true, numero, url} o {ok:false, error}, sin lanzar: esto
   * lo llama una página web y un fallo al hablar con GitHub no debe tumbarla.
   *
   * `error` es una de: "disabled", "rate", "empty", "github".
   */
  async function reportar({ titulo, cuerpo, cuenta, personaje, reino, lugar, version, lang }) {
    if (!enabled) return { ok: false, error: "disabled" };

    const t = limpiar(titulo, TITULO_MAX);
    const c = limpiar(cuerpo, CUERPO_MAX);
    if (!t || !c) return { ok: false, error: "empty" };
    if ((await quedan(cuenta)) <= 0) return { ok: false, error: "rate" };

    // El contexto va al final y en pequeño: quien lee la incidencia quiere leer primero el
    // fallo. No se incluye el correo de la cuenta —la incidencia puede ser pública—, solo el
    // personaje y el número de cuenta, que es lo que hace falta para buscar en los registros.
    const contexto = [
      `**Personaje:** ${personaje || "-"}`,
      `**Reino:** ${reino || "-"}`,
      `**Donde:** ${lugar || "-"}`,
      `**Cuenta:** #${cuenta}`,
      `**Cliente:** ${version || "7.3.5.26972"}`,
      `**Idioma:** ${lang || "-"}`,
      `**Enviado:** ${new Date().toISOString()}`,
    ].join(" · ");

    const body = `${c}\n\n---\n<sub>${contexto}</sub>\n<sub>Reportado desde el juego.</sub>\n<!-- ns:cuenta=${cuenta} -->`;

    try {
      const r = await fetch(`https://api.github.com/repos/${repo}/issues`, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: "application/vnd.github+json",
          "X-GitHub-Api-Version": "2022-11-28",
          "Content-Type": "application/json",
          "User-Agent": "nightspire-support",
        },
        body: JSON.stringify({ title: t, body, labels: etiquetas.length ? etiquetas : undefined }),
        signal: AbortSignal.timeout(15000),
      });

      const texto = await r.text();
      if (r.status !== 201) {
        log(`fallos: GitHub contestó ${r.status}: ${texto.slice(0, 300)}`);
        return { ok: false, error: "github" };
      }

      const dato = JSON.parse(texto);
      apuntar(cuenta);
      // Al principio de la lista cacheada, para que aparezca en «tus fallos» al momento y no
      // dentro de dos minutos.
      if (cache.lista) cache.lista.unshift(normaliza(dato));
      log(`fallos: incidencia #${dato.number} abierta por la cuenta ${cuenta} (${personaje || "-"})`);
      return { ok: true, numero: dato.number, url: dato.html_url };
    } catch (e) {
      log(`fallos: no se pudo abrir la incidencia: ${e.message}`);
      return { ok: false, error: "github" };
    }
  }

  return { enabled, reportar, quedan, mios, uno, LIMITE_DIARIO };
}
