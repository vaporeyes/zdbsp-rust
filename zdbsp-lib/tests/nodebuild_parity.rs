// ABOUTME: Structural parity check: build a map with the Rust port and with the C++
// ABOUTME: baseline, compare node/subsector/seg/vertex counts. A precondition for
// ABOUTME: byte-identical NODES/SSECTORS/SEGS once the lump writer lands.

use std::path::{Path, PathBuf};
use std::process::Command;
use zdbsp_lib::nodebuild::{util::collect_poly_spots, NodeBuilder};
use zdbsp_lib::processor::Processor;
use zdbsp_lib::wad::WadReader;

const CORPUS: &str = "/Users/jsh/media/doom_wads";
const BASELINE: &str = "/Users/jsh/dev/repos/zdbsp/build/zdbsp";

fn baseline_bin() -> Option<PathBuf> {
    let p = std::env::var("ZDBSP_BASELINE")
        .map(PathBuf::from)
        .unwrap_or(PathBuf::from(BASELINE));
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("SKIP: baseline binary not at {p:?}");
        None
    }
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join("zdbsp-rust-node-parity");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run_baseline(bin: &Path, input: &Path, out: &Path, map: &str) {
    let status = Command::new(bin)
        .arg(format!("-m{map}"))
        .arg("-o")
        .arg(out)
        .arg(input)
        .status()
        .expect("run baseline");
    assert!(status.success());
}

/// Baseline counts read from the classic-format lumps the C++ produces.
struct BaselineCounts {
    nodes: usize,
    subsectors: usize,
    segs: usize,
    vertices: usize,
}

fn baseline_counts(wad_path: &Path, map: &str) -> BaselineCounts {
    let r = WadReader::open(wad_path).unwrap();
    let map_idx = r.find_lump(map, 0);
    let nodes_idx = r.find_map_lump("NODES", map_idx);
    let ssectors_idx = r.find_map_lump("SSECTORS", map_idx);
    let segs_idx = r.find_map_lump("SEGS", map_idx);
    let verts_idx = r.find_map_lump("VERTEXES", map_idx);
    let nodes_sz = r.lump(nodes_idx).unwrap().size as usize;
    let ssec_sz = r.lump(ssectors_idx).unwrap().size as usize;
    let segs_sz = r.lump(segs_idx).unwrap().size as usize;
    let vert_sz = r.lump(verts_idx).unwrap().size as usize;
    BaselineCounts {
        nodes: nodes_sz / 28,        // sizeof(MapNode)
        subsectors: ssec_sz / 4,     // sizeof(MapSubsector)
        segs: segs_sz / 12,          // sizeof(MapSeg)
        vertices: vert_sz / 4,       // sizeof(MapVertex)
    }
}

fn build_rust(wad_path: &Path, map: &str) -> zdbsp_lib::nodebuild::extract::NodeOutput {
    let mut r = WadReader::open(wad_path).unwrap();
    let idx = r.find_lump(map, 0);
    let mut p = Processor::load(&mut r, idx, false).unwrap();
    let (starts, anchors) = collect_poly_spots(&p.level);
    let mut nb = NodeBuilder::new(&mut p.level, starts, anchors, map, false);
    nb.build();
    nb.extract_nodes()
}

fn assert_counts_match(wad_file: &str, map: &str) {
    let Some(bin) = baseline_bin() else { return };
    let input = PathBuf::from(CORPUS).join(wad_file);
    if !input.is_file() {
        return;
    }
    let baseline_out = scratch().join(format!("{wad_file}.{map}.wad"));
    run_baseline(&bin, &input, &baseline_out, map);
    let b = baseline_counts(&baseline_out, map);
    let r = build_rust(&input, map);

    assert_eq!(r.nodes.len(), b.nodes, "{wad_file}/{map}: node count");
    assert_eq!(r.subsectors.len(), b.subsectors, "{wad_file}/{map}: subsector count");
    assert_eq!(r.segs.len(), b.segs, "{wad_file}/{map}: seg count");
    // Vertex counts: C++ writes the full vertex array (originals + node-builder additions).
    assert_eq!(r.vertices.len(), b.vertices, "{wad_file}/{map}: vertex count");
}

#[test]
fn counts_match_doom_e1m1() {
    assert_counts_match("doom.wad", "E1M1");
}

#[test]
fn counts_match_doom2_map01() {
    assert_counts_match("doom2.wad", "MAP01");
}

#[test]
fn counts_match_doom2_map30() {
    assert_counts_match("doom2.wad", "MAP30");
}

#[test]
fn counts_match_every_doom2_map() {
    let Some(_) = baseline_bin() else { return };
    let input = PathBuf::from(CORPUS).join("doom2.wad");
    if !input.is_file() {
        return;
    }
    let reader = WadReader::open(&input).unwrap();
    let mut idx = reader.next_map(-1);
    let mut count = 0usize;
    while idx >= 0 {
        let name = reader.lump_name(idx).into_owned();
        assert_counts_match("doom2.wad", &name);
        count += 1;
        idx = reader.next_map(idx);
    }
    assert_eq!(count, 32);
}

#[test]
fn counts_match_heretic_e1m1() {
    assert_counts_match("heretic.wad", "E1M1");
}
