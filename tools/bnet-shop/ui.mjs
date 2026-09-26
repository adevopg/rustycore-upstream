/*
 * Diseño compartido de la tienda, el soporte, el RAF y el autenticador.
 *
 * Es el mismo lenguaje visual que www.nightspire.gg: los tokens de color están copiados tal
 * cual del styles.css de la web, en oklch, porque el navegador del juego es un Chromium 151
 * y los entiende de forma nativa. Así el color coincide exactamente en vez de "parecerse".
 *
 * Los nombres de clase son los que ya usaban las páginas (card, msg, row, btn, box, tabs…):
 * aquí solo cambia el aspecto. De ese modo el rediseño no toca ni una línea de la lógica de
 * cobro, que es la parte que no conviene mover.
 */

export const CSS = `
:root{
  color-scheme: dark;
  --radius: 0.375rem;
  --background: oklch(0.15 0.018 150);
  --foreground: oklch(0.93 0.02 110);
  --card: oklch(0.2 0.022 150);
  --muted: oklch(0.26 0.02 150);
  --muted-foreground: oklch(0.7 0.03 130);
  --border: oklch(0.34 0.04 145);
  --input: oklch(0.3 0.03 150);
  --fel: oklch(0.82 0.24 138);
  --gold: oklch(0.82 0.14 85);
  --destructive: oklch(0.58 0.22 25);
  --font-display: "Cinzel", "Trajan Pro", Georgia, serif;
  --font-body: "Inter", "Segoe UI", system-ui, Arial, sans-serif;
}

*{box-sizing:border-box}

/*
  El marco del juego es ancho y bajo, así que el fondo no puede ser negro plano: se llena
  con dos halos fel, un rescoldo dorado abajo y una rejilla muy tenue que da textura sin
  competir con el contenido. Todo con gradientes, sin imágenes: son cero peticiones y el
  navegador del juego las pinta sin coste.
*/
body{
  margin:0;
  min-height:100vh;
  background:
    radial-gradient(900px 500px at 50% -12%, oklch(0.34 0.11 145 / .32), transparent 68%),
    radial-gradient(700px 420px at 8% 104%, oklch(0.3 0.09 150 / .22), transparent 70%),
    radial-gradient(620px 380px at 96% 96%, oklch(0.35 0.08 85 / .14), transparent 72%),
    repeating-linear-gradient(115deg, oklch(0.5 0.06 140 / .028) 0 2px, transparent 2px 92px),
    var(--background);
  background-attachment:fixed;
  color:var(--foreground);
  font:15px/1.5 var(--font-body);
  display:flex;
  align-items:flex-start;
  justify-content:center;
  padding:14px 16px 18px;
}

/* ---- Tarjeta: el "panel" de la web ---------------------------------------- */
.card,.box{
  width:100%;
  max-width:520px;
  margin:0 auto;
  background:linear-gradient(160deg, oklch(0.22 0.03 150 / .92), oklch(0.16 0.02 150 / .92));
  border:1px solid oklch(0.4 0.06 140 / .5);
  border-radius:var(--radius);
  box-shadow:inset 0 1px 0 oklch(0.85 0.2 140 / .08), 0 12px 40px oklch(0 0 0 / .5);
  padding:24px 24px 20px;
}
.box{max-width:none;margin:0 0 16px;padding:18px}
.wide{max-width:860px}

h1,h2,h3{
  font-family:var(--font-display);
  font-weight:700;
  letter-spacing:.04em;
  margin:0 0 16px;
  color:var(--gold);
  text-shadow:0 0 14px oklch(0.82 0.14 85 / .35);
}
h1{font-size:21px}
h2{font-size:17px}
h3{font-size:15px}

a{color:var(--fel);text-decoration:none}
a:hover{text-decoration:underline}

/* ---- Mensajes -------------------------------------------------------------- */
.msg{
  display:flex;align-items:center;gap:10px;
  padding:12px 14px;margin:12px 0;
  border-radius:var(--radius);
  border:1px solid var(--border);
  background:oklch(0.24 0.025 150 / .6);
  color:var(--muted-foreground);
}
.msg.ok{
  border-color:oklch(0.72 0.2 140 / .55);
  background:oklch(0.3 0.1 145 / .28);
  color:oklch(0.92 0.12 135);
}
.msg.bad{
  border-color:oklch(0.58 0.22 25 / .6);
  background:oklch(0.32 0.12 25 / .25);
  color:oklch(0.88 0.11 30);
}
.msg.pending{
  border-color:oklch(0.8 0.14 85 / .5);
  background:oklch(0.32 0.09 85 / .22);
  color:oklch(0.9 0.12 88);
}

/* ---- Producto y filas ------------------------------------------------------ */
.prod{
  display:flex;justify-content:space-between;gap:14px;
  padding:14px 0;margin-bottom:18px;
  border-top:1px solid var(--border);
  border-bottom:1px solid var(--border);
}
.prod .n{font-family:var(--font-display);font-size:15px;font-weight:700;color:var(--foreground)}
.prod .d{margin-top:4px;font-size:13px;color:var(--muted-foreground)}
.prod .p{
  font-family:var(--font-display);font-size:20px;font-weight:700;
  color:var(--gold);text-shadow:0 0 14px oklch(0.82 0.14 85 / .35);white-space:nowrap;
}

.row{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-top:16px;flex-wrap:wrap}
.right{margin-left:auto}
.head{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:14px}
.who{font-size:13px;color:var(--muted-foreground)}
.k{font-size:12px;text-transform:uppercase;letter-spacing:.14em;color:var(--muted-foreground)}

.foot{margin-top:14px;font-size:12px;color:var(--muted-foreground)}

/* ---- Botones: btn-legion de la web ---------------------------------------- */
.btn,button,input[type=submit]{
  display:inline-flex;align-items:center;justify-content:center;gap:.5rem;
  padding:.7rem 1.5rem;
  font-family:var(--font-display);font-weight:700;
  letter-spacing:.12em;text-transform:uppercase;font-size:.78rem;
  color:oklch(0.97 0.03 120);
  border-radius:.3rem;
  border:1px solid oklch(0.72 0.2 140 / .75);
  background:linear-gradient(180deg, oklch(0.4 0.14 143), oklch(0.22 0.08 145));
  box-shadow:inset 0 1px 0 oklch(0.9 0.22 140 / .35),
             inset 0 -10px 22px oklch(0 0 0 / .5),
             0 0 22px oklch(0.7 0.22 140 / .35);
  transition:all .22s ease;
  cursor:pointer;
}
.btn:hover,button:hover{filter:brightness(1.2);box-shadow:inset 0 1px 0 oklch(0.95 0.22 140 / .45), 0 0 34px oklch(0.78 0.24 138 / .6)}
.btn:active,button:active{transform:translateY(1px)}
.btn:disabled,button:disabled{opacity:.5;cursor:not-allowed;filter:none;box-shadow:none}

/* Secundario: el btn-legion-ghost */
.sec,.btn.sec,button.sec{
  border-color:oklch(0.5 0.05 140 / .7);
  background:linear-gradient(180deg, oklch(0.26 0.02 150), oklch(0.18 0.02 150));
  box-shadow:inset 0 1px 0 oklch(0.8 0.05 140 / .12);
}
.sec:hover,button.sec:hover{filter:brightness(1.25);box-shadow:inset 0 1px 0 oklch(0.8 0.05 140 / .18)}

/* ---- Campos ---------------------------------------------------------------- */
input,textarea,select{
  width:100%;
  padding:.65rem .8rem;
  font:14px/1.4 var(--font-body);
  color:var(--foreground);
  background:oklch(0.3 0.03 150 / .45);
  border:1px solid var(--border);
  border-radius:var(--radius);
  outline:none;
}
input:focus,textarea:focus,select:focus{border-color:oklch(0.72 0.2 140 / .8);box-shadow:0 0 0 3px oklch(0.7 0.22 140 / .18)}
textarea{min-height:120px;resize:vertical}
label{display:block;margin:12px 0 6px;font-size:12px;text-transform:uppercase;letter-spacing:.14em;color:var(--muted-foreground)}

/* ---- Pestañas --------------------------------------------------------------- */
.tabs{display:flex;gap:8px;margin-bottom:16px;flex-wrap:wrap}
.tab{
  padding:.5rem 1rem;font-size:.75rem;font-family:var(--font-display);
  text-transform:uppercase;letter-spacing:.12em;
  border:1px solid oklch(0.5 0.05 140 / .7);border-radius:.3rem;
  background:linear-gradient(180deg, oklch(0.26 0.02 150), oklch(0.18 0.02 150));
  color:var(--muted-foreground);cursor:pointer;
}
.tab.active{
  border-color:oklch(0.72 0.2 140 / .75);
  background:linear-gradient(180deg, oklch(0.4 0.14 143), oklch(0.22 0.08 145));
  color:oklch(0.97 0.03 120);
}

/* ---- Tickets y etiquetas ---------------------------------------------------- */
.tk{
  display:block;padding:12px 14px;margin-bottom:10px;
  border:1px solid var(--border);border-radius:var(--radius);
  background:oklch(0.24 0.025 150 / .5);
  transition:border-color .2s ease, transform .2s ease;
}
.tk:hover{border-color:oklch(0.7 0.18 140 / .7);transform:translateY(-2px);text-decoration:none}
.tk.nohover:hover{transform:none;border-color:var(--border)}

.badge{
  display:inline-block;padding:.2rem .6rem;
  font-size:11px;text-transform:uppercase;letter-spacing:.12em;
  border-radius:999px;border:1px solid var(--border);
  background:oklch(0.28 0.03 150);color:var(--muted-foreground);
}
.badge.ok{border-color:oklch(0.72 0.2 140 / .6);color:oklch(0.9 0.14 135);background:oklch(0.3 0.1 145 / .3)}
.badge.bad{border-color:oklch(0.58 0.22 25 / .6);color:oklch(0.88 0.11 30);background:oklch(0.32 0.12 25 / .25)}
.badge.pending{border-color:oklch(0.8 0.14 85 / .55);color:oklch(0.9 0.12 88);background:oklch(0.32 0.09 85 / .22)}

/* ---- Cargando ---------------------------------------------------------------- */
.spin{
  width:16px;height:16px;flex:none;
  border:2px solid oklch(0.5 0.05 140 / .45);
  border-top-color:var(--fel);
  border-radius:50%;
  animation:spin .8s linear infinite;
}
@keyframes spin{to{transform:rotate(360deg)}}

/* El checkout de SumUp se pinta en un iframe propio: se le deja sitio y respiro. */
#pay,#sumup-card{margin-top:8px}

/* ---- Checkout a dos columnas ------------------------------------------------
   El marco del juego da mucho ancho y poca altura. Apilado, el formulario de la
   tarjeta caía por debajo del borde y había que desplazar para pagar, que es la
   peor pantalla donde obligar a buscar. Repartido en dos, el producto ocupa el
   hueco negro de la izquierda y la tarjeta entra entera.
   Por debajo de 700px vuelve a una sola columna, para el launcher o una ventana
   pequeña. -------------------------------------------------------------------- */
@media (min-width:700px){
  .card.checkout{
    display:grid;
    grid-template-columns:minmax(230px,0.95fr) minmax(320px,1.15fr);
    column-gap:26px;
    align-items:start;
    max-width:860px;
  }
  .card.checkout>h1{grid-column:1 / -1;grid-row:1}
  .card.checkout>.prod{grid-column:1;grid-row:2;flex-direction:column;align-items:flex-start;gap:10px;border-top:0;padding-top:0;margin-bottom:0;border-bottom:0}
  .card.checkout>.prod .p{font-size:26px}
  .card.checkout>.notas{grid-column:1;grid-row:3}
  .card.checkout>#pay{grid-column:2;grid-row:2 / span 2;margin-top:0}
  .card.checkout>#status{grid-column:2;grid-row:4}
  .card.checkout>.row{grid-column:1 / -1;grid-row:5;margin-top:12px;justify-content:flex-end}
}

/* Las notas llenan el hueco que queda bajo el precio: lo que antes era un vacío negro
   ahora dice lo que el jugador quiere saber justo antes de pagar. */
.notas{margin-top:14px;padding-top:14px;border-top:1px solid oklch(0.4 0.06 140 / .35)}
.notas p{margin:0 0 8px;font-size:12.5px;line-height:1.5;color:var(--muted-foreground);position:relative;padding-left:16px}
.notas p:last-child{margin-bottom:0}
.notas p::before{content:"";position:absolute;left:0;top:.5em;width:6px;height:6px;border-radius:50%;background:var(--fel);box-shadow:0 0 8px oklch(0.82 0.24 138 / .7)}

/* Descripción del producto: se limita a unas líneas para que no empuje el pago
   fuera de la pantalla. El texto completo sigue en la tienda. */
.prod .d{
  display:-webkit-box;
  -webkit-line-clamp:5;
  -webkit-box-orient:vertical;
  overflow:hidden;
}

/* Aire recortado donde no aporta: en el marco del juego cada píxel de alto cuenta. */
.card.checkout{padding:18px 20px 16px}
.card.checkout>h1{margin-bottom:12px;font-size:19px}
`;

/*
 * Tema claro para las paginas que se abren DENTRO del juego (carga, checkout, pagado, error):
 * el navegador integrado del cliente pinta la pagina sobre un marco negro, y un fondo oscuro
 * encima se ve como un agujero. Blizzard usa ahi una pagina blanca, limpia, con el producto a
 * la izquierda y el pago a la derecha. Solo sobrescribe colores y brillos: la maquetacion y las
 * clases son las mismas, asi que la logica de cobro no cambia. Se activa con `opts.tema = "claro"`.
 */
export const CSS_CLARO = `
:root{
  color-scheme: light;
  --background:#f4f5f7;
  --foreground:#1b1d21;
  --card:#ffffff;
  --muted:#eceef2;
  --muted-foreground:#5c6370;
  --border:#d9dde3;
  --input:#ffffff;
  --fel:#0a6ed1;
  --gold:#1b1d21;
  --destructive:#c8342a;
  --font-display:"Inter","Segoe UI",system-ui,Arial,sans-serif;
}
body{background:var(--background);background-attachment:scroll;color:var(--foreground)}
.card,.box{background:var(--card);border:1px solid var(--border);box-shadow:0 6px 24px rgba(20,30,50,.08)}
h1,h2,h3{color:var(--foreground);text-shadow:none;letter-spacing:0;font-weight:700}
.card.checkout>h1{font-size:20px}
a{color:var(--fel)}
.msg{background:#f7f8fa;border-color:var(--border);color:var(--muted-foreground)}
.msg.ok{background:#e9f7ee;border-color:#9fd8b3;color:#1f6b3a}
.msg.bad{background:#fdecea;border-color:#f3b4ae;color:#9b2c22}
.msg.pending{background:#fff7e6;border-color:#f2d59a;color:#7a5200}
.prod{border-color:var(--border)}
.prod .n{color:var(--foreground);font-size:16px}
.prod .d{color:var(--muted-foreground)}
.prod .p{color:var(--foreground);text-shadow:none}
.notas{border-top-color:var(--border)}
.notas p{color:var(--muted-foreground)}
.notas p::before{background:var(--fel);box-shadow:none}
.btn,button,input[type=submit]{
  color:#fff;border:1px solid #0a6ed1;
  background:linear-gradient(180deg,#2b8bea,#0a6ed1);
  box-shadow:0 1px 2px rgba(0,0,0,.15);
  text-transform:none;letter-spacing:0;font-size:14px;font-weight:600;
}
.btn:hover,button:hover{filter:brightness(1.06);box-shadow:0 2px 6px rgba(10,110,209,.3)}
.sec,.btn.sec,button.sec{color:var(--foreground);border-color:var(--border);background:#fff;box-shadow:none}
.sec:hover,button.sec:hover{filter:none;background:#f3f4f6;box-shadow:none}
input,textarea,select{background:#fff;color:var(--foreground);border-color:var(--border)}
input:focus,textarea:focus,select:focus{border-color:var(--fel);box-shadow:0 0 0 3px rgba(10,110,209,.15)}
label,.k,.who,.foot{color:var(--muted-foreground)}
.spin{border-color:#d9dde3;border-top-color:var(--fel)}
.tk{background:#fff;border-color:var(--border)}
.badge{background:#f3f4f6;color:var(--muted-foreground);border-color:var(--border)}
`;

/** Cabecera de marca, igual que la del sitio. */
const marca = `<div style="width:100%;max-width:520px;margin:0 auto 14px;text-align:center">
  <span style="font-family:var(--font-display);font-size:18px;font-weight:700;letter-spacing:.22em;color:var(--fel);text-shadow:0 0 18px oklch(0.82 0.24 138 / .55)">NIGHTSPIRE</span><span style="font-size:11px;font-weight:600;letter-spacing:.3em;color:var(--gold);margin-left:6px">.GG</span>
</div>`;

const FUENTES = `<link rel="preconnect" href="https://fonts.googleapis.com"><link rel="preconnect" href="https://fonts.gstatic.com" crossorigin><link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Cinzel:wght@500;700&family=Inter:wght@400;500;600;700&display=swap">`;

/**
 * Envoltorio de página.
 *
 * El contenedor va siempre a 860px y es la propia tarjeta quien decide su ancho: `.card`
 * son 520 y `.card.wide` 860, que es lo que ya pedían las pantallas de soporte. Así no hace
 * falta que cada módulo avise de lo ancho que es lo suyo.
 *
 * `opts.marca` a false quita la cabecera, para el navbar del checkout, que va empotrado
 * dentro del marco del juego y no debe repetir la marca.
 */
export function page(esc, title, body, opts = {}) {
  const cabecera = opts.marca === false ? "" : marca;
  const tema = opts.tema === "claro" ? CSS_CLARO : "";
  return `<!doctype html><html lang="es"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${esc(title)}</title>${FUENTES}<style>${CSS}${tema}</style></head><body><div style="width:100%;max-width:860px">${cabecera}${body}</div></body></html>`;
}
