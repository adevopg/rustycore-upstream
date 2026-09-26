//! wow-text-dump: copia a un fichero la seccion `.text` de WowClassic.exe (3.4.3.54261) leyendola
//! de la memoria del proceso en ejecucion.
//!
//! El ejecutable de Blizzard lleva la seccion de codigo cifrada en disco y la descifra al arrancar,
//! asi que las funciones del cliente (p. ej. `BNFeaturesEnabled`) solo se pueden desensamblar a
//! partir de la memoria de un cliente ya arrancado. Esta herramienta no escribe nada en el proceso:
//! solo lee y guarda los bytes junto a un `.txt` con la direccion base y el mapa de secciones, que
//! es lo que necesita el desensamblador.
//!
//! Uso (con el WoW abierto, en la pantalla de login o dentro del juego):
//!   wow-text-dump [--pid N] [--out WowClassic.text.bin] [--all]
//!
//! - Windows: `ReadProcessMemory` (kernel32, sin dependencias).
//! - Linux (cliente bajo Wine/Proton): `/proc/<pid>/maps` + `/proc/<pid>/mem`. Si sale
//!   "Permission denied" es la restriccion ptrace de Yama: ejecutalo con `sudo`.

use std::fs;
use std::io::Write;
use std::process;

const PAGE: usize = 0x1000;

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

/// Acceso de solo lectura a la memoria de otro proceso.
trait ProcessMemory {
    /// Lee exactamente `buf.len()` bytes desde `address`; `false` si no se puede.
    fn read_exact_at(&self, address: usize, buf: &mut [u8]) -> bool;
}

/// Lee `[address, address+len)`; las paginas ilegibles se dejan a cero y se cuentan.
fn read_range(mem: &dyn ProcessMemory, address: usize, len: usize) -> (Vec<u8>, usize) {
    let mut out = vec![0u8; len];
    let mut unreadable = 0usize;
    let mut offset = 0usize;
    const CHUNK: usize = 1 << 20;
    while offset < len {
        let want = CHUNK.min(len - offset);
        if mem.read_exact_at(address + offset, &mut out[offset..offset + want]) {
            offset += want;
            continue;
        }
        // Pagina a pagina dentro del trozo que fallo.
        let end = offset + want;
        let mut page_off = offset;
        while page_off < end {
            let page_len = PAGE.min(end - page_off);
            if !mem.read_exact_at(address + page_off, &mut out[page_off..page_off + page_len]) {
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

/// Lo que cada plataforma tiene que resolver: PID, modulo principal y lector de memoria.
struct Target {
    pid: u32,
    path: String,
    base: usize,
    image_size: usize,
    memory: Box<dyn ProcessMemory>,
}

// ------------------------------------------------------------------------------------------
// Windows: Toolhelp32 + ReadProcessMemory
// ------------------------------------------------------------------------------------------
#[cfg(windows)]
mod platform {
    use super::{ProcessMemory, Target};
    use std::ffi::c_void;
    use std::mem;

    type Handle = *mut c_void;
    type Bool = i32;

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const TH32CS_SNAPMODULE: u32 = 0x0000_0008;
    const TH32CS_SNAPMODULE32: u32 = 0x0000_0010;
    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
    const PROCESS_VM_READ: u32 = 0x0010;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const ERROR_BAD_LENGTH: u32 = 24;

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

    fn wide_to_string(wide: &[u16]) -> String {
        let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
        String::from_utf16_lossy(&wide[..end])
    }

    fn last_error() -> u32 {
        unsafe { GetLastError() }
    }

    pub fn find_process(name: &str) -> Option<u32> {
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

    /// Modulo principal (el .exe) del proceso: direccion base, tamano de la imagen y ruta.
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

    struct WindowsProcess(Handle);

    impl ProcessMemory for WindowsProcess {
        fn read_exact_at(&self, address: usize, buf: &mut [u8]) -> bool {
            let mut read = 0usize;
            let ok = unsafe {
                ReadProcessMemory(
                    self.0,
                    address as *const c_void,
                    buf.as_mut_ptr() as *mut c_void,
                    buf.len(),
                    &mut read,
                )
            };
            ok != 0 && read == buf.len()
        }
    }

    impl Drop for WindowsProcess {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub fn open(pid: u32) -> Result<Target, String> {
        let (base, image_size, path) = main_module(pid)?;
        let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
        if handle.is_null() {
            return Err(format!(
                "OpenProcess: error {} (prueba a ejecutar como administrador)",
                last_error()
            ));
        }
        Ok(Target {
            pid,
            path,
            base,
            image_size,
            memory: Box::new(WindowsProcess(handle)),
        })
    }
}

// ------------------------------------------------------------------------------------------
// Linux (Wine / Proton): /proc/<pid>/maps + /proc/<pid>/mem
// ------------------------------------------------------------------------------------------
#[cfg(target_os = "linux")]
mod platform {
    use super::{ProcessMemory, Target};
    use std::fs;
    use std::os::unix::fs::FileExt;

    pub fn find_process(name: &str) -> Option<u32> {
        let mut found = Vec::new();
        for entry in fs::read_dir("/proc").ok()? {
            let entry = entry.ok()?;
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            // Bajo Wine el nombre del proceso es el del .exe; cmdline lleva la ruta Windows.
            let comm = fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
            let cmdline = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            let cmdline = String::from_utf8_lossy(&cmdline).to_ascii_lowercase();
            let first_arg = cmdline.split('\0').next().unwrap_or("").to_string();
            if comm.trim().eq_ignore_ascii_case(name)
                || first_arg.ends_with(&name.to_ascii_lowercase())
            {
                found.push(pid);
            }
        }
        found.sort_unstable();
        found.first().copied()
    }

    struct LinuxProcess(fs::File);

    impl ProcessMemory for LinuxProcess {
        fn read_exact_at(&self, address: usize, buf: &mut [u8]) -> bool {
            self.0.read_exact_at(buf, address as u64).is_ok()
        }
    }

    pub fn open(pid: u32) -> Result<Target, String> {
        let maps = fs::read_to_string(format!("/proc/{pid}/maps"))
            .map_err(|e| format!("/proc/{pid}/maps: {e}"))?;
        // Todas las regiones respaldadas por el .exe; la base es la mas baja y el tamano llega
        // hasta el final de la mas alta (Wine mapea la imagen completa en 0x140000000 o donde
        // la reubique).
        let mut base = usize::MAX;
        let mut end = 0usize;
        let mut path = String::new();
        for line in maps.lines() {
            let mut cols = line.split_whitespace();
            let (Some(range), Some(_perms), Some(_off), Some(_dev), Some(_inode)) =
                (cols.next(), cols.next(), cols.next(), cols.next(), cols.next())
            else {
                continue;
            };
            let name: String = cols.collect::<Vec<_>>().join(" ");
            if !name.to_ascii_lowercase().ends_with("wowclassic.exe") {
                continue;
            }
            let (lo, hi) = range.split_once('-').ok_or("maps: rango invalido")?;
            let lo = usize::from_str_radix(lo, 16).map_err(|e| e.to_string())?;
            let hi = usize::from_str_radix(hi, 16).map_err(|e| e.to_string())?;
            base = base.min(lo);
            end = end.max(hi);
            path = name;
        }
        if base == usize::MAX {
            return Err(format!(
                "el proceso {pid} no tiene WowClassic.exe mapeado (mira /proc/{pid}/maps)"
            ));
        }
        let file = fs::File::open(format!("/proc/{pid}/mem")).map_err(|e| {
            format!("/proc/{pid}/mem: {e} (si es Permission denied, ejecuta con sudo)")
        })?;
        Ok(Target {
            pid,
            path,
            base,
            image_size: end - base,
            memory: Box::new(LinuxProcess(file)),
        })
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use super::Target;
    pub fn find_process(_name: &str) -> Option<u32> {
        None
    }
    pub fn open(_pid: u32) -> Result<Target, String> {
        Err("plataforma no soportada (solo Windows y Linux)".to_string())
    }
}

fn main() {
    let opts = parse_args();
    let pid = match opts.pid.or_else(|| platform::find_process("WowClassic.exe")) {
        Some(pid) => pid,
        None => {
            eprintln!("no se encuentra WowClassic.exe en ejecucion (abre el juego o usa --pid)");
            process::exit(1);
        }
    };
    let target = match platform::open(pid) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("PID {pid}: {e}");
            process::exit(1);
        }
    };
    println!("PID {}: {}", target.pid, target.path);
    println!(
        "  base 0x{:016X}, imagen {} bytes",
        target.base, target.image_size
    );

    let (headers, bad) = read_range(target.memory.as_ref(), target.base, PAGE);
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
        (0usize, target.image_size, "imagen completa")
    } else {
        match sections.iter().find(|s| s.name == ".text") {
            Some(s) => (s.virtual_address, s.virtual_size, ".text"),
            None => {
                eprintln!("no hay seccion .text");
                process::exit(1);
            }
        }
    };

    println!("leyendo {what} ({len} bytes)...");
    let (data, unreadable) = read_range(target.memory.as_ref(), target.base + start, len);
    drop(target.memory);

    let sample_from = (len / 2).saturating_sub(1 << 19);
    let sample = &data[sample_from..(sample_from + (1 << 20)).min(len)];
    let e = entropy(sample);

    if let Err(err) = fs::write(&opts.out, &data) {
        eprintln!("no se pudo escribir {}: {err}", opts.out);
        process::exit(1);
    }
    let meta_path = format!("{}.txt", opts.out);
    let mut meta = String::new();
    meta.push_str(&format!("exe={}\n", target.path));
    meta.push_str(&format!("pid={}\n", target.pid));
    meta.push_str(&format!("image_base=0x{:016X}\n", target.base));
    meta.push_str(&format!("image_size=0x{:X}\n", target.image_size));
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
