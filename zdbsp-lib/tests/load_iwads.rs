// ABOUTME: Map-loader sanity tests against the real IWAD corpus. Cross-checks parsed
// ABOUTME: counts against raw lump sizes and verifies the pruning passes.

use std::path::PathBuf;
use zdbsp_lib::processor::{MapFormat, Processor};
use zdbsp_lib::wad::WadReader;

const CORPUS: &str = "/Users/jsh/media/doom_wads";

fn open_wad(name: &str) -> Option<WadReader> {
    let p = PathBuf::from(CORPUS).join(name);
    if !p.is_file() {
        eprintln!("SKIP: {name} not present");
        return None;
    }
    Some(WadReader::open(p).expect("open wad"))
}

#[test]
fn loader_counts_match_raw_lump_sizes_doom() {
    let Some(mut wad) = open_wad("doom.wad") else { return };
    let first = wad.next_map(-1);
    assert!(first >= 0);

    // Raw byte sizes for the map header at `first`.
    let vert_idx = wad.find_map_lump("VERTEXES", first);
    let line_idx = wad.find_map_lump("LINEDEFS", first);
    let side_idx = wad.find_map_lump("SIDEDEFS", first);
    let sect_idx = wad.find_map_lump("SECTORS", first);
    let thg_idx = wad.find_map_lump("THINGS", first);
    let vert_size = wad.lump(vert_idx).unwrap().size as usize;
    let line_size = wad.lump(line_idx).unwrap().size as usize;
    let side_size = wad.lump(side_idx).unwrap().size as usize;
    let sect_size = wad.lump(sect_idx).unwrap().size as usize;
    let thg_size = wad.lump(thg_idx).unwrap().size as usize;

    // no_prune so the post-load counts reflect the raw lumps.
    let p = Processor::load(&mut wad, first, true).expect("load");
    assert_eq!(p.format, MapFormat::Doom);
    assert_eq!(p.map_name, "E1M1");
    assert_eq!(p.level.num_vertices(), vert_size / 4);
    // LINEDEFS may shed zero-length lines via remove_extra_lines (which still runs even
    // with no_prune); E1M1 in retail Doom has none.
    assert_eq!(p.level.num_lines(), line_size / 14);
    assert_eq!(p.level.num_sides(), side_size / 30);
    assert_eq!(p.level.num_sectors(), sect_size / 26);
    assert_eq!(p.level.num_things(), thg_size / 10);
}

#[test]
fn loader_counts_match_raw_lump_sizes_hexen() {
    let Some(mut wad) = open_wad("hexen.wad") else { return };
    let first = wad.next_map(-1);
    assert!(first >= 0);

    let line_idx = wad.find_map_lump("LINEDEFS", first);
    let thg_idx = wad.find_map_lump("THINGS", first);
    let line_size = wad.lump(line_idx).unwrap().size as usize;
    let thg_size = wad.lump(thg_idx).unwrap().size as usize;

    let p = Processor::load(&mut wad, first, true).expect("load");
    assert_eq!(p.format, MapFormat::Hexen);
    // Hexen records are wider on disk.
    assert_eq!(p.level.num_lines(), line_size / 16);
    assert_eq!(p.level.num_things(), thg_size / 20);
}

#[test]
fn map_bounds_are_sane() {
    let Some(mut wad) = open_wad("doom.wad") else { return };
    let first = wad.next_map(-1);
    let p = Processor::load(&mut wad, first, true).expect("load");
    assert!(p.level.min_x < p.level.max_x);
    assert!(p.level.min_y < p.level.max_y);
    // E1M1 fits comfortably inside ~16384 map units in every direction.
    assert!(p.level.min_x > -(16384 << 16));
    assert!(p.level.max_x < (16384 << 16));
}

#[test]
fn pruning_does_not_grow_counts() {
    let Some(mut wad) = open_wad("doom2.wad") else { return };
    let first = wad.next_map(-1);

    let mut wad2 = WadReader::open(PathBuf::from(CORPUS).join("doom2.wad")).unwrap();
    let with_prune = Processor::load(&mut wad, first, false).expect("with prune");
    let no_prune = Processor::load(&mut wad2, first, true).expect("no prune");

    assert!(with_prune.level.num_sides() <= no_prune.level.num_sides());
    assert!(with_prune.level.num_sectors() <= no_prune.level.num_sectors());
    // Lines are pruned (zero-length removal) regardless of no_prune; same count either way.
    assert_eq!(with_prune.level.num_lines(), no_prune.level.num_lines());
}

#[test]
fn load_every_map_in_doom2() {
    let Some(wad) = open_wad("doom2.wad") else { return };
    let mut count = 0usize;
    let mut idx = wad.next_map(-1);
    while idx >= 0 {
        let name = wad.lump_name(idx).into_owned();
        // Re-open per iteration; Processor::load borrows wad mutably.
        let mut w = WadReader::open(PathBuf::from(CORPUS).join("doom2.wad")).unwrap();
        let p = Processor::load(&mut w, idx, false)
            .unwrap_or_else(|e| panic!("loading {name}: {e}"));
        assert!(p.level.num_lines() > 0, "{name} has no lines");
        assert!(p.level.num_sectors() > 0, "{name} has no sectors");
        count += 1;
        idx = wad.next_map(idx);
    }
    assert_eq!(count, 32);
}
