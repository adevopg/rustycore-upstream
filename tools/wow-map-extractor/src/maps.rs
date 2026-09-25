//! Map tile extraction: port of `ExtractMaps`, `IsDeepWaterIgnored` and the
//! `ConvertADT(fileName|fileDataId, ...)` overloads from
//! `src/tools/map_extractor/System.cpp`, plus `ChunkedFile::loadFile` (loadlib.cpp).
//!
//! Output: `maps/{mapId:04}_{y:02}_{x:02}.map` for every WDT tile with `MAIN` flag 1 and
//! `maps/{mapId:04}.tilelist` ("MAPS", `MapVersionMagic`, build, then the
//! `std::bitset<4096>::to_string()` of converted tiles — bit `y * 64 + x`, most
//! significant bit first).

use std::io::Write;
use std::path::Path;

use crate::adt::{WDT_MAP_SIZE, maid_root_adt, main_flag, mphd_flags};
use crate::casc::{CASC_LOCALE_ALL_WOW, Casc, FileRead, FileRef, OpenFlags};
use crate::convert::{AdtConverter, MAP_MAGIC, MAP_VERSION_MAGIC, TileInfo};
use crate::fsutil::create_dir;
use crate::loadlib::ChunkedFile;
use crate::tables::{CppFatal, LiquidTables, MapEntry, read_map_dbc};

/// `IsDeepWaterIgnored(mapId, x, y)` — called with (`y`, `x`) of the WDT loop.
pub(crate) fn is_deep_water_ignored(map_id: u32, x: u32, y: u32) -> bool {
    if map_id == 0 {
        // Vashj'ir grids completely ignore fatigue
        return ((39..=40).contains(&x) && (24..=26).contains(&y))
            || ((41..=46).contains(&x) && (18..=26).contains(&y));
    }
    if map_id == 1 {
        // Thousand Needles
        return x == 43 && (y == 39 || y == 40);
    }
    false
}

/// `ChunkedFile::loadFile` (both overloads): `description` is printed on
/// `Error loading %s` (the file name, or `Map <name> grid [x,y]`).
pub(crate) fn load_chunked_file(
    casc: &Casc,
    file: FileRef<'_>,
    description: &str,
    log: bool,
) -> Option<ChunkedFile> {
    let FileRead::Data(bytes) = casc.read(
        file,
        CASC_LOCALE_ALL_WOW,
        OpenFlags {
            print_errors: log,
            zerofill_encrypted: false,
        },
    ) else {
        return None;
    };
    let chunked = ChunkedFile::from_bytes(bytes);
    if chunked.is_none() {
        println!("Error loading {description}");
    }
    chunked
}

/// `std::bitset<WDT_MAP_SIZE * WDT_MAP_SIZE>::to_string()`.
pub(crate) fn tiles_to_string(existing_tiles: &[bool]) -> Vec<u8> {
    existing_tiles
        .iter()
        .rev()
        .map(|&b| if b { b'1' } else { b'0' })
        .collect()
}

/// The `.tilelist` file content.
pub(crate) fn tile_list_bytes(build: u32, existing_tiles: &[bool]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + existing_tiles.len());
    out.extend_from_slice(&MAP_MAGIC);
    out.extend_from_slice(&MAP_VERSION_MAGIC.to_le_bytes());
    out.extend_from_slice(&build.to_le_bytes());
    out.extend_from_slice(&tiles_to_string(existing_tiles));
    out
}

/// Options shared by every tile.
pub(crate) struct MapExtractOptions<'a> {
    pub(crate) output_path: &'a Path,
    pub(crate) build: u32,
    pub(crate) allow_float_to_int: bool,
    pub(crate) print_progress: bool,
}

/// `ConvertADT(ChunkedFile&, ...)` plus the output file write.
fn convert_and_write(
    converter: &mut AdtConverter,
    adt: &ChunkedFile,
    tables: &LiquidTables,
    tile: &TileInfo<'_>,
    output_path: &str,
) -> Result<bool, CppFatal> {
    let bytes = converter.convert(adt, tables, tile)?;
    let written = std::fs::File::create(output_path).and_then(|mut f| f.write_all(&bytes));
    if written.is_err() {
        println!("Can't create the output file '{output_path}'");
        return Ok(false);
    }
    Ok(true)
}

/// `ExtractMaps(build)`.
pub(crate) fn extract_maps(casc: &Casc, opts: &MapExtractOptions<'_>) -> anyhow::Result<()> {
    println!("Extracting maps...");

    let map_ids: Vec<MapEntry> = read_map_dbc(casc)?;
    let tables = LiquidTables::read(casc)?;

    create_dir(&opts.output_path.join("maps"))?;

    let out = opts.output_path.display().to_string();
    let mut converter = AdtConverter::default();

    println!("Convert map files");
    for (z, map) in map_ids.iter().enumerate() {
        println!(
            "Extract {} ({}/{})                  ",
            map.name,
            z + 1,
            map_ids.len()
        );
        // Loadup map grid data
        let mut existing_tiles = vec![false; WDT_MAP_SIZE * WDT_MAP_SIZE];
        let file_name = format!("World\\Maps\\{0}\\{0}.wdt", map.directory);
        if let Some(wdt) = load_chunked_file(casc, FileRef::Name(&file_name), &file_name, false) {
            let mphd = wdt.get_chunk("MPHD");
            let tile_flags = wdt.get_chunk("MAIN");
            let tile_file_ids = wdt.get_chunk("MAID");
            for y in 0..WDT_MAP_SIZE {
                for x in 0..WDT_MAP_SIZE {
                    // C++ dereferences MAIN unconditionally (crash when missing).
                    let Some(main) = tile_flags else {
                        anyhow::bail!("{file_name}: WDT has no (unique) MAIN chunk");
                    };
                    if main_flag(&wdt.data, main, y, x) & 0x1 == 0 {
                        continue;
                    }

                    let output_file_name = format!("{out}/maps/{:04}_{y:02}_{x:02}.map", map.id);
                    let tile = TileInfo {
                        map_name: &map.name,
                        gx: y as u32,
                        gy: x as u32,
                        build: opts.build,
                        ignore_deep_water: is_deep_water_ignored(map.id, y as u32, x as u32),
                        allow_float_to_int: opts.allow_float_to_int,
                    };
                    let adt = if let Some(mphd) = mphd
                        && mphd_flags(&wdt.data, mphd) & 0x200 != 0
                    {
                        let Some(file_ids) = tile_file_ids else {
                            anyhow::bail!(
                                "{file_name}: WDT has MPHD flag 0x200 but no (unique) MAID chunk"
                            );
                        };
                        let file_data_id = maid_root_adt(&wdt.data, file_ids, y, x);
                        let description = format!("Map {} grid [{y},{x}]", map.name);
                        load_chunked_file(casc, FileRef::Id(file_data_id), &description, true)
                    } else {
                        let storage_path =
                            format!("World\\Maps\\{0}\\{0}_{x}_{y}.adt", map.directory);
                        load_chunked_file(casc, FileRef::Name(&storage_path), &storage_path, true)
                    };
                    existing_tiles[y * WDT_MAP_SIZE + x] = match adt {
                        Some(adt) => convert_and_write(
                            &mut converter,
                            &adt,
                            &tables,
                            &tile,
                            &output_file_name,
                        )?,
                        None => false,
                    };
                }

                // draw progress bar
                if opts.print_progress {
                    print!(
                        "Processing........................{}%\r",
                        (100 * (y + 1)) / WDT_MAP_SIZE
                    );
                }
            }
        }

        let tile_list = format!("{out}/maps/{:04}.tilelist", map.id);
        // fopen failure is silently ignored by the C++
        let _ = std::fs::write(tile_list, tile_list_bytes(opts.build, &existing_tiles));
    }

    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_water_grids() {
        assert!(is_deep_water_ignored(0, 39, 24));
        assert!(is_deep_water_ignored(0, 46, 18));
        assert!(!is_deep_water_ignored(0, 39, 23));
        assert!(!is_deep_water_ignored(0, 47, 20));
        assert!(is_deep_water_ignored(1, 43, 39));
        assert!(is_deep_water_ignored(1, 43, 40));
        assert!(!is_deep_water_ignored(1, 43, 41));
        assert!(!is_deep_water_ignored(530, 43, 39));
    }

    #[test]
    fn tile_list_is_msb_first_bitset_string() {
        let mut tiles = vec![false; 4096];
        tiles[0] = true; // y 0, x 0 -> last character
        tiles[64 + 2] = true; // y 1, x 2
        let bytes = tile_list_bytes(54261, &tiles);
        assert_eq!(&bytes[0..4], b"MAPS");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 10);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 54261);
        let s = &bytes[12..];
        assert_eq!(s.len(), 4096);
        assert_eq!(s[4095], b'1');
        assert_eq!(s[4095 - 66], b'1');
        let ones: Vec<usize> = (0..s.len()).filter(|&i| s[i] == b'1').collect();
        assert_eq!(ones, vec![4095 - 66, 4095]);
        assert!(s.iter().all(|&c| c == b'0' || c == b'1'));
    }
}
