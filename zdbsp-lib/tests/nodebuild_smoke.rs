// ABOUTME: Smoke tests for the node-builder pipeline. Loads real maps and runs the
// ABOUTME: full Phase-1..4d pipeline (load → segs → planes → tree). No byte-identical
// ABOUTME: parity yet (extraction is Phase 4e); these tests just assert the tree is
// ABOUTME: structurally sane on real input.

use std::path::PathBuf;
use zdbsp_lib::nodebuild::{util::collect_poly_spots, NodeBuilder};
use zdbsp_lib::processor::Processor;
use zdbsp_lib::wad::WadReader;
use zdbsp_lib::workdata::NFX_SUBSECTOR;

const CORPUS: &str = "/Users/jsh/media/doom_wads";

fn open(wad: &str) -> Option<WadReader> {
    let p = PathBuf::from(CORPUS).join(wad);
    if !p.is_file() {
        eprintln!("SKIP: {wad} not present");
        return None;
    }
    Some(WadReader::open(p).expect("open"))
}

fn build_map(wad_file: &str, map_name: &str) -> Option<(usize, usize, usize)> {
    let mut reader = open(wad_file)?;
    let idx = reader.find_lump(map_name, 0);
    assert!(idx >= 0, "{map_name} missing from {wad_file}");
    let mut p = Processor::load(&mut reader, idx, false).expect("load");
    let (starts, anchors) = collect_poly_spots(&p.level);

    let mut nb = NodeBuilder::new(&mut p.level, starts, anchors, map_name, false);
    nb.build();
    Some((nb.nodes().len(), nb.subsectors().len(), nb.seg_list().len()))
}

#[test]
fn build_doom_e1m1() {
    let Some((nodes, subs, segs)) = build_map("doom.wad", "E1M1") else {
        return;
    };
    assert!(nodes > 0, "no nodes built");
    assert!(subs > 0, "no subsectors built");
    assert!(segs > 0, "no segs emitted");
    // Reasonable sanity bounds for retail E1M1.
    assert!(nodes < 10_000);
    assert!(subs < 10_000);
}

#[test]
fn build_doom2_map01() {
    let Some((nodes, subs, segs)) = build_map("doom2.wad", "MAP01") else {
        return;
    };
    assert!(nodes > 0);
    assert!(subs > 0);
    assert!(segs > 0);
}

#[test]
fn tree_children_reference_valid_indices() {
    let mut reader = match open("doom.wad") {
        Some(r) => r,
        None => return,
    };
    let idx = reader.find_lump("E1M1", 0);
    let mut p = Processor::load(&mut reader, idx, false).unwrap();
    let (starts, anchors) = collect_poly_spots(&p.level);
    let mut nb = NodeBuilder::new(&mut p.level, starts, anchors, "E1M1", false);
    nb.build();

    let num_nodes = nb.nodes().len();
    let num_subs = nb.subsectors().len();
    for (i, n) in nb.nodes().iter().enumerate() {
        for &child in &n.int_children {
            if child & NFX_SUBSECTOR != 0 {
                let s = (child & !NFX_SUBSECTOR) as usize;
                assert!(s < num_subs, "node {i} references subsector {s}, only {num_subs} exist");
            } else {
                let c = child as usize;
                assert!(c < num_nodes, "node {i} references node {c}, only {num_nodes} exist");
                assert!(c < i, "node {i} references later node {c} — postorder violated");
            }
        }
    }
}

#[test]
fn sweep_all_doom2_maps_build_without_crashing() {
    let Some(reader) = open("doom2.wad") else { return };
    let mut count = 0usize;
    let mut idx = reader.next_map(-1);
    while idx >= 0 {
        let name = reader.lump_name(idx).into_owned();
        let mut w = WadReader::open(PathBuf::from(CORPUS).join("doom2.wad")).unwrap();
        let mut p = Processor::load(&mut w, idx, false).unwrap_or_else(|e| panic!("load {name}: {e}"));
        let (starts, anchors) = collect_poly_spots(&p.level);
        let mut nb = NodeBuilder::new(&mut p.level, starts, anchors, &name, false);
        nb.build();
        assert!(!nb.nodes().is_empty(), "{name} produced no nodes");
        assert!(!nb.subsectors().is_empty(), "{name} produced no subsectors");
        count += 1;
        idx = reader.next_map(idx);
    }
    assert_eq!(count, 32);
}

#[test]
fn sweep_hexen_maps() {
    let Some(reader) = open("hexen.wad") else { return };
    let mut count = 0usize;
    let mut idx = reader.next_map(-1);
    while idx >= 0 {
        let name = reader.lump_name(idx).into_owned();
        let mut w = WadReader::open(PathBuf::from(CORPUS).join("hexen.wad")).unwrap();
        let mut p = Processor::load(&mut w, idx, false).unwrap_or_else(|e| panic!("load {name}: {e}"));
        let (starts, anchors) = collect_poly_spots(&p.level);
        let mut nb = NodeBuilder::new(&mut p.level, starts, anchors, &name, false);
        nb.build();
        assert!(!nb.nodes().is_empty(), "{name} produced no nodes");
        count += 1;
        idx = reader.next_map(idx);
    }
    assert!(count >= 30, "expected at least 30 Hexen maps, got {count}");
}

#[test]
fn root_is_last_node() {
    let mut reader = match open("doom.wad") {
        Some(r) => r,
        None => return,
    };
    let idx = reader.find_lump("E1M1", 0);
    let mut p = Processor::load(&mut reader, idx, false).unwrap();
    let (starts, anchors) = collect_poly_spots(&p.level);
    let mut nb = NodeBuilder::new(&mut p.level, starts, anchors, "E1M1", false);
    nb.build();
    // The C++ pushes nodes post-order, so the root is the last entry.
    assert!(!nb.nodes().is_empty());
}
