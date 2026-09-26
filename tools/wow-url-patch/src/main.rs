//! wow-url-patch: reescribe in situ las URLs de checkout y SSO que WowClassic.exe (3.4.3.54261)
//! lleva incrustadas, para que el navegador integrado de la tienda llegue a nuestro servidor.
//!
//! Funciona tanto sobre el ejecutable original de Blizzard como sobre uno ya parcheado antes
//! (por ejemplo con `inna.cl`): busca cadenas ASCII terminadas en NUL que encajen con cada
//! regla y las sustituye respetando el hueco disponible (la cadena mas su relleno de NUL),
//! asi el ejecutable no cambia de tamano ni se mueve nada.
//!
//! Uso:
//!   wow-url-patch <ruta\WowClassic.exe> [--dry-run]
//!       [--checkout URL] [--sso URL] [--host HOST] [--no-backup]
//!
//! Valores por defecto: tienda de NightSpire (tienda.nightspire.gg:8096).

use std::env;
use std::fs;
use std::process;

const DEFAULT_CHECKOUT: &str = "https://tienda.nightspire.gg:8096/shop/simplecheckout/loading";
const DEFAULT_SSO: &str = "https://tienda.nightspire.gg/login/sso?token=%s&ref=%s";
const DEFAULT_HOST: &str = "nightspire.gg/s";
/// Lista blanca de URLs del navegador del checkout (3 copias en el ejecutable, la mas corta de
/// 48 bytes). Sin ella la pagina carga pero el cliente no le inyecta `purchaseRequest`.
const DEFAULT_ALLOW: &str = r"^https?:\/\/([\w.-]+\.)?nightspire\.gg.*$";

struct Rule {
    name: &'static str,
    /// La cadena encaja si contiene TODOS estos fragmentos...
    must_contain: &'static [&'static str],
    /// ...y ninguno de estos.
    must_not_contain: &'static [&'static str],
    replacement: String,
}

struct Options {
    path: String,
    dry_run: bool,
    backup: bool,
    checkout: String,
    sso: String,
    host: String,
    allow: String,
}

fn usage() -> ! {
    eprintln!(
        "uso: wow-url-patch <WowClassic.exe> [--dry-run] [--no-backup] [--checkout URL] [--sso URL] [--host HOST]\n\
         \n  --checkout  URL de la pagina de carga del checkout (defecto {DEFAULT_CHECKOUT})\
         \n  --sso       URL del SSO de soporte, con %s para token y ref (defecto {DEFAULT_SSO})\
         \n  --host      host de soporte, maximo 15 caracteres (defecto {DEFAULT_HOST})\
         \n  --allow     regex de la lista blanca del navegador, maximo 47 caracteres (defecto {DEFAULT_ALLOW})\
         \n  --dry-run   solo muestra lo que cambiaria"
    );
    process::exit(2);
}

fn parse_args() -> Options {
    let mut args = env::args().skip(1);
    let mut opts = Options {
        path: String::new(),
        dry_run: false,
        backup: true,
        checkout: DEFAULT_CHECKOUT.to_string(),
        sso: DEFAULT_SSO.to_string(),
        host: DEFAULT_HOST.to_string(),
        allow: DEFAULT_ALLOW.to_string(),
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dry-run" => opts.dry_run = true,
            "--no-backup" => opts.backup = false,
            "--checkout" => opts.checkout = args.next().unwrap_or_else(|| usage()),
            "--sso" => opts.sso = args.next().unwrap_or_else(|| usage()),
            "--host" => opts.host = args.next().unwrap_or_else(|| usage()),
            "--allow" => opts.allow = args.next().unwrap_or_else(|| usage()),
            "-h" | "--help" => usage(),
            other if opts.path.is_empty() && !other.starts_with("--") => opts.path = other.to_string(),
            _ => usage(),
        }
    }
    if opts.path.is_empty() {
        usage();
    }
    opts
}

/// Cadenas ASCII imprimibles (>= 8 bytes) terminadas en NUL: (inicio, fin exclusivo sin el NUL).
fn ascii_strings(data: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, &b) in data.iter().enumerate() {
        let printable = (0x20..0x7f).contains(&b);
        match (start, printable) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                if b == 0 && i - s >= 8 {
                    out.push((s, i));
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Bytes disponibles desde `start`: la cadena, su NUL y todos los NUL de relleno que la siguen.
fn slot_len(data: &[u8], start: usize, end: usize) -> usize {
    let mut z = end;
    while z < data.len() && data[z] == 0 {
        z += 1;
    }
    z - start
}

fn main() {
    let opts = parse_args();
    if opts.host.len() > 15 {
        eprintln!("--host: maximo 15 caracteres (hueco del ejecutable)");
        process::exit(2);
    }

    let rules = [
        Rule {
            name: "checkout (ref=)",
            must_contain: &["login/sso?ref="],
            must_not_contain: &["%s"],
            replacement: opts.checkout.clone(),
        },
        Rule {
            name: "checkout (NavBar)",
            must_contain: &["blizzard-checkout/NavBar"],
            must_not_contain: &[],
            replacement: opts.checkout.clone(),
        },
        Rule {
            name: "checkout (loading)",
            must_contain: &["blizzard-checkout/loading"],
            must_not_contain: &["login/sso"],
            replacement: opts.checkout.clone(),
        },
        Rule {
            name: "lista blanca",
            must_contain: &["^https?:", "(\\/.*)?$"],
            must_not_contain: &[],
            replacement: opts.allow.clone(),
        },
        Rule {
            name: "SSO soporte",
            must_contain: &["login/sso?token=%s&ref=%s"],
            must_not_contain: &[],
            replacement: opts.sso.clone(),
        },
        Rule {
            name: "host soporte",
            must_contain: &["/s"],
            must_not_contain: &["/", "http"], // se afina abajo: exactamente '<host>/s'
            replacement: opts.host.clone(),
        },
    ];

    let mut data = match fs::read(&opts.path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("no se puede leer {}: {e}", opts.path);
            process::exit(1);
        }
    };
    println!("{} ({} bytes)", opts.path, data.len());

    let mut changes: Vec<(usize, usize, String, String, &str)> = Vec::new(); // (inicio, hueco, antes, despues, regla)
    for (start, end) in ascii_strings(&data) {
        let text = String::from_utf8_lossy(&data[start..end]).into_owned();
        for rule in &rules {
            let matches = if rule.name == "host soporte" {
                // '<dominio>/s' exacto: p.ej. 'www.inna.cl/s' (o el original con /s), sin barras extra
                text.ends_with("/s")
                    && text.matches('/').count() == 1
                    && text.contains('.')
                    && !text.contains(':')
                    && text.len() <= 16
            } else {
                rule.must_contain.iter().all(|m| text.contains(m))
                    && rule.must_not_contain.iter().all(|m| !text.contains(m))
            };
            if !matches {
                continue;
            }
            if text == rule.replacement {
                println!("  0x{start:08x} {:<18} ya parcheada: {text}", rule.name);
                break;
            }
            let slot = slot_len(&data, start, end);
            if rule.replacement.len() + 1 > slot {
                eprintln!(
                    "  0x{start:08x} {:<18} NO CABE: hueco {slot} bytes, hacen falta {} :: {text}",
                    rule.name,
                    rule.replacement.len() + 1
                );
                break;
            }
            changes.push((start, slot, text.clone(), rule.replacement.clone(), rule.name));
            break;
        }
    }

    if changes.is_empty() {
        println!("no se ha encontrado ninguna URL que cambiar");
        return;
    }
    for (start, slot, before, after, name) in &changes {
        println!("  0x{start:08x} {name:<18} [{slot:>3} bytes] {before}\n{:>31}-> {after}", "");
    }
    if opts.dry_run {
        println!("--dry-run: {} cambio(s), nada escrito", changes.len());
        return;
    }

    if opts.backup {
        let bak = format!("{}.bak", opts.path);
        if !std::path::Path::new(&bak).exists() {
            if let Err(e) = fs::copy(&opts.path, &bak) {
                eprintln!("no se pudo crear la copia de seguridad {bak}: {e}");
                process::exit(1);
            }
            println!("copia de seguridad: {bak}");
        }
    }

    for (start, slot, _, after, _) in &changes {
        let bytes = after.as_bytes();
        data[*start..*start + *slot].fill(0);
        data[*start..*start + bytes.len()].copy_from_slice(bytes);
    }
    if let Err(e) = fs::write(&opts.path, &data) {
        eprintln!("no se pudo escribir {}: {e}", opts.path);
        process::exit(1);
    }
    println!("{} cambio(s) escritos en {}", changes.len(), opts.path);
}
