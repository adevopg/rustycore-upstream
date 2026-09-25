//! Port of `TrinityCore` `vmap4_assembler` (`src/tools/vmap4_assembler/
//! VMapAssembler.cpp`): converts the raw `Buildings/` extractor output into
//! the runtime `vmaps/` files. Single-threaded, like the C++ tool.
//!
//! Usage: `wow-vmap-assembler [raw data dir] [vmap dest dir]`
//! (defaults `Buildings` and `vmaps`).

// Conversions and float literals intentionally mirror the C++ expressions.
#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::excessive_precision
)]
#![cfg_attr(test, allow(clippy::float_cmp, clippy::too_many_lines))]

mod tile_assembler;

use std::process::ExitCode;

use tile_assembler::TileAssembler;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let program = args.first().map_or("wow-vmap-assembler", String::as_str);

    println!("VMAP assembler");

    let mut src = "Buildings".to_owned();
    let mut dest = "vmaps".to_owned();

    if args.len() > 3 {
        println!("usage: {program} <raw data dir> <vmap dest dir>");
        return ExitCode::from(1);
    }
    if let Some(a) = args.get(1) {
        src.clone_from(a);
    }
    if let Some(a) = args.get(2) {
        dest.clone_from(a);
    }

    println!("using {src} as source directory and writing output to {dest}");

    let mut ta = match TileAssembler::new(&src, &dest) {
        Ok(ta) => ta,
        Err(e) => {
            println!("cannot create {dest}: {e}");
            println!("exit with errors");
            return ExitCode::from(1);
        }
    };

    if !ta.convert_world2() {
        println!("exit with errors");
        return ExitCode::from(1);
    }

    println!("Ok, all done");
    ExitCode::SUCCESS
}
