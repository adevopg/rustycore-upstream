//! wow-text-dump: copia a un fichero la seccion `.text` de WowClassic.exe (3.4.3.54261) leyendola
//! de la memoria del proceso en ejecucion.
//!
//! El ejecutable de Blizzard lleva la seccion de codigo cifrada en disco y la descifra al arrancar,
//! asi que las funciones del cliente (p. ej. `BNFeaturesEnabled`) solo se pueden desensamblar a
//! partir de la memoria de un cliente ya arrancado. Esta herramienta no escribe nada en el proceso:
//! solo lee (`ReadProcessMemory`) y guarda los bytes junto a un `.txt` con la direccion base y el
//! mapa de secciones, que es lo que necesita el desensamblador.
//!
//! Uso (con el WoW abierto, en la pantalla de login o dentro del juego):
//!   wow-text-dump [--pid N] [--out WowClassic.text.bin] [--all]
//!
//! Solo Windows x64. Sin dependencias externas: usa kernel32 directamente.

use std::ffi::c_void;
use std::fs;
use std::io::Write;
use std::mem;
use std::process;

type Handle = *mut c_void;
type Bool = i32;

const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
const TH32CS_SNAPMODULE: u32 = 0x0000_0008;
const TH32CS_SNAPMODULE32: u32 = 0x0000_0010;
const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
const PROCESS_VM_READ: u32 = 0x0010;
const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
const ERROR_BAD_LENGTH: u32 = 24;
const PAGE: usize = 0x1000;

#[repr(C)]
struct ProcessEntry32W {
    dw_size: u32,
    cnt_usage: u32,
    th32_process_id: u32,
    th32_default_heap_id: usize,
    th32_module_id: u32,
    cnt_threads: u32,
    th32_parent_process_id: u32,
    pc_pri_class_base: i32,
    dw_flags: u32,
    sz_exe_file: [u16; 260],
}

#[repr(C)]
struct ModuleEntry32W {
    dw_size: u32,
    th32_module_id: u32,
    th32_process_id: u32,
    glblcnt_usage: u32,
    proccnt_usage: u32,
    mod_base_addr: *mut u8,
    mod_base_size: u32,
    h_module: Handle,
    sz_module: [u16; 256],
    sz_exe_path: [u16; 260],
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
    fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> Bool;
    fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> Bool;
    fn Module32FirstW(snapshot: Handle, entry: *mut ModuleEntry32W) -> Bool;
    fn OpenProcess(desired_access: u32, inherit_handle: Bool, process_id: u32) -> Handle;
    fn ReadProcessMemory(
        process: Handle,
        base_address: *const c_void,
        buffer: *mut c_void,
        size: usize,
        bytes_read: *mut usize,
    ) -> Bool;
    fn CloseHandle(handle: Handle) -> Bool;
    fn GetLastError() -> u32;
}

struct Options {
    pid: Option<u32>,
    out: String,
    all: bool,
}

fn usage() -> ! {
    eprintln!(
        "uso: wow-text-dump [--pid N] [--out FICHERO] [--all]\n\
         \n  --pid   proceso a leer (por defecto busca WowClassic.exe)\
         \n  --out   fichero de salida (por defecto WowClassic.text.bin)\
         \n  --all   vuelca la imagen completa en vez de solo .text"
    );
    process::exit(2);
}

fn parse_args() -> Options {
    let mut opts = Options {
        pid: None,
        out: "WowClassic.text.bin".to_string(),
        all: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pid" => {
                opts.pid = Some(
                    args.next()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or_else(|| usage()),
                )
            }
            "--out" => opts.out = args.next().unwrap_or_else(|| usage()),
            "--all" => opts.all = true,
            _ => usage(),
        }
    }
    opts
}

fn wide_to_string(wide: &[u16]) -> String {
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

fn find_process(name: &str) -> Option<u32> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: ProcessEntry32W = mem::zeroed();
        entry.dw_size = mem::size_of::<ProcessEntry32W>() as u32;
        let mut found = None;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if wide_to_string(&entry.sz_exe_file).eq_ignore_ascii_case(name) {
                    found = Some(entry.th32_process_id);
                    break;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        found
    }
}

/// Modulo principal (el .exe) del proceso: direccion base y tamano de la imagen.
fn main_module(pid: u32) -> Result<(usize, usize, String), String> {
    // Module32First falla con ERROR_BAD_LENGTH mientras el proceso carga DLLs; se reintenta.
    for _ in 0..20 {
        unsafe {
            let snapshot =
                CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
            if snapshot == INVALID_HANDLE_VALUE {
                let err = last_error();
                if err == ERROR_BAD_LENGTH {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                return Err(format!("CreateToolhelp32Snapshot(modulos): error {err}"));
            }
            let mut entry: ModuleEntry32W = mem::zeroed();
            entry.dw_size = mem::size_of::<ModuleEntry32W>() as u32;
            let ok = Module32FirstW(snapshot, &mut entry) != 0;
            let err = last_error();
            CloseHandle(snapshot);
            if ok {
                return Ok((
                    entry.mod_base_addr as usize,
                    entry.mod_base_size as usize,
                    wide_to_string(&entry.sz_exe_path),
                ));
            }
            return Err(format!("Module32FirstW: error {err}"));
        }
    }
    Err("no se pudo enumerar los modulos del proceso".to_string())
}

/// Lee `[address, address+len)`; las paginas ilegibles se dejan a cero y se cuentan.
fn read_range(process: Handle, address: usize, len: usize) -> (Vec<u8>, usize) {
    let mut out = vec![0u8; len];
    let mut unreadable = 0usize;
    let mut offset = 0usize;
    const CHUNK: usize = 1 << 20;
    while offset < len {
        let want = CHUNK.min(len - offset);
        let mut read = 0usize;
        let ok = unsafe {
            ReadProcessMemory(
                process,
                (address + offset) as *const c_void,
                out[offset..].as_mut_ptr() as *mut c_void,
                want,
                &mut read,
            )
        };
        if ok != 0 && read == want {
            offset += want;
            continue;
        }
        // Pagina a pagina dentro del trozo que fallo.
        let mut page_off = offset;
        let end = offset + want;
        while page_off < end {
            let page_len = PAGE.min(end - page_off);
            let mut page_read = 0usize;
            let ok = unsafe {
                ReadProcessMemory(
                    process,
                    (address + page_off) as *const c_void,
                    out[page_off..].as_mut_ptr() as *mut c_void,
                    page_len,
                    &mut page_read,
                )
            };
            if ok == 0 || page_read != page_len {
                unreadable += 1;
            }
            page_off += page_len;
        }
        offset = end;
    }
    (out, unreadable)
}

struct Section {
    name: String,
    virtual_address: usize,
    virtual_size: usize,
}

fn parse_sections(headers: &[u8]) -> Result<Vec<Section>, String> {
    let u16_at = |o: usize| u16::from_le_bytes([headers[o], headers[o + 1]]) as usize;
    let u32_at = |o: usize| {
        u32::from_le_bytes([headers[o], headers[o + 1], headers[o + 2], headers[o + 3]]) as usize
    };
    if headers.len() < 0x40 || &headers[..2] != b"MZ" {
        return Err("la imagen no empieza por MZ".to_string());
    }
    let nt = u32_at(0x3c);
    if nt + 0x18 > headers.len() || &headers[nt..nt + 4] != b"PE\0\0" {
        return Err("cabecera PE no encontrada".to_string());
    }
    let sections = u16_at(nt + 6);
    let optional_size = u16_at(nt + 20);
    let table = nt + 24 + optional_size;
    let mut out = Vec::with_capacity(sections);
    for i in 0..sections {
        let s = table + i * 40;
        if s + 40 > headers.len() {
            return Err("tabla de secciones truncada".to_string());
        }
        let name_end = headers[s..s + 8].iter().position(|&b| b == 0).unwrap_or(8);
        out.push(Section {
            name: String::from_utf8_lossy(&headers[s..s + name_end]).into_owned(),
            virtual_size: u32_at(s + 8),
            virtual_address: u32_at(s + 12),
        });
    }
    Ok(out)
}

/// Entropia de Shannon (bits/byte): codigo x64 normal ronda 6.0-6.5; cifrado, 8.0.
fn entropy(bytes: &[u8]) -> f64 {
    let mut hist = [0u64; 256];
    for &b in bytes {
        hist[b as usize] += 1;
    }
    let n = bytes.len() as f64;
    hist.iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

fn main() {
    let opts = parse_args();
    let pid = match opts.pid.or_else(|| find_process("WowClassic.exe")) {
        Some(pid) => pid,
        None => {
            eprintln!("no se encuentra WowClassic.exe en ejecucion (abre el juego o usa --pid)");
            process::exit(1);
        }
    };
    let (base, image_size, path) = match main_module(pid) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("PID {pid}: {e}");
            process::exit(1);
        }
    };
    println!("PID {pid}: {path}");
    println!("  base 0x{base:016X}, imagen {image_size} bytes");

    let process = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if process.is_null() {
        eprintln!(
            "OpenProcess: error {} (prueba a ejecutar como administrador)",
            last_error()
        );
        process::exit(1);
    }

    let (headers, bad) = read_range(process, base, PAGE);
    if bad != 0 {
        eprintln!("no se pueden leer las cabeceras PE del proceso");
        process::exit(1);
    }
    let sections = match parse_sections(&headers) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };
    for s in &sections {
        println!(
            "  seccion {:<8} VA 0x{:08X} tamano 0x{:08X}",
            s.name, s.virtual_address, s.virtual_size
        );
    }

    let (start, len, what) = if opts.all {
        (0usize, image_size, "imagen completa".to_string())
    } else {
        match sections.iter().find(|s| s.name == ".text") {
            Some(s) => (s.virtual_address, s.virtual_size, ".text".to_string()),
            None => {
                eprintln!("no hay seccion .text");
                process::exit(1);
            }
        }
    };

    println!("leyendo {what} ({len} bytes)...");
    let (data, unreadable) = read_range(process, base + start, len);
    unsafe {
        CloseHandle(process);
    }

    let sample_from = (len / 2).saturating_sub(1 << 19);
    let sample = &data[sample_from..(sample_from + (1 << 20)).min(len)];
    let e = entropy(sample);

    if let Err(err) = fs::write(&opts.out, &data) {
        eprintln!("no se pudo escribir {}: {err}", opts.out);
        process::exit(1);
    }
    let meta_path = format!("{}.txt", opts.out);
    let mut meta = String::new();
    meta.push_str(&format!("exe={path}\n"));
    meta.push_str(&format!("pid={pid}\n"));
    meta.push_str(&format!("image_base=0x{base:016X}\n"));
    meta.push_str(&format!("image_size=0x{image_size:X}\n"));
    meta.push_str(&format!("dump_rva=0x{start:X}\n"));
    meta.push_str(&format!("dump_size=0x{len:X}\n"));
    meta.push_str(&format!("unreadable_pages={unreadable}\n"));
    meta.push_str(&format!("sample_entropy={e:.3}\n"));
    for s in &sections {
        meta.push_str(&format!(
            "section={} rva=0x{:X} size=0x{:X}\n",
            s.name, s.virtual_address, s.virtual_size
        ));
    }
    if let Err(err) = fs::File::create(&meta_path).and_then(|mut f| f.write_all(meta.as_bytes()))
    {
        eprintln!("no se pudo escribir {meta_path}: {err}");
        process::exit(1);
    }

    println!("escrito {} ({} bytes) y {meta_path}", opts.out, data.len());
    if unreadable > 0 {
        println!("  aviso: {unreadable} paginas ilegibles (rellenadas con ceros)");
    }
    println!("  entropia de la muestra central: {e:.2} bits/byte");
    if e > 7.9 {
        println!("  AVISO: parece cifrado todavia; espera a que el juego llegue al login y repite");
    } else {
        println!("  OK: codigo descifrado");
    }
}
