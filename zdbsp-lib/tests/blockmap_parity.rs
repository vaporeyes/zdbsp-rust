// ABOUTME: Byte-identical parity tests for the blockmap builder. Runs the C++ baseline
// ABOUTME: against real maps, then rebuilds the same maps in Rust and compares lump bytes.

use std::path::{Path, PathBuf};
use std::process::Command;
use zdbsp_lib::{blockmap, processor::Processor, wad::WadReader};

const CORPUS: &str = "/Users/jsh/media/doom_wads";
const BASELINE: &str = "/Users/jsh/dev/repos/zdbsp/build/zdbsp";

fn baseline_bin() -> Option<PathBuf> {
    let p = std::env::var("ZDBSP_BASELINE").map(PathBuf::from).ok().unwrap_or(PathBuf::from(BASELINE));
    if p.is_file() { Some(p) } else {
        eprintln!("SKIP: baseline binary not at {p:?}");
        None
    }
}

fn scratch_dir() -> PathBuf {
    let d = std::env::temp_dir().join("zdbsp-rust-blockmap-parity");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run_baseline(baseline: &Path, input: &Path, out: &Path, map: &str) {
    let status = Command::new(baseline)
        .arg(format!("-m{map}"))
        .arg("-o")
        .arg(out)
        .arg(input)
        .status()
        .expect("run baseline");
    assert!(status.success(), "baseline failed for {map}");
}

fn extract_blockmap_bytes(wad_path: &Path, map: &str) -> Vec<u8> {
    let mut reader = WadReader::open(wad_path).expect("open baseline output");
    let map_idx = reader.find_lump(map, 0);
    assert!(map_idx >= 0, "{map} missing from {wad_path:?}");
    let bm_idx = reader.find_map_lump("BLOCKMAP", map_idx);
    assert!(bm_idx >= 0, "BLOCKMAP missing for {map}");
    reader.read_lump(bm_idx).expect("read BLOCKMAP")
}

fn build_rust_blockmap(wad_path: &Path, map: &str) -> Vec<u8> {
    let mut reader = WadReader::open(wad_path).unwrap();
    let map_idx = reader.find_lump(map, 0);
    assert!(map_idx >= 0);
    let p = Processor::load(&mut reader, map_idx, false).expect("load");
    let words = blockmap::build(&p.level);
    let mut bytes = Vec::with_capacity(words.len() * 2);
    for w in words {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    bytes
}

fn assert_blockmap_matches(wad_name: &str, map: &str) {
    let Some(baseline) = baseline_bin() else { return };
    let input = PathBuf::from(CORPUS).join(wad_name);
    if !input.is_file() {
        eprintln!("SKIP: {wad_name} not present");
        return;
    }
    let out = scratch_dir().join(format!("{wad_name}.{map}.baseline.wad"));
    run_baseline(&baseline, &input, &out, map);

    let baseline_bm = extract_blockmap_bytes(&out, map);
    let rust_bm = build_rust_blockmap(&input, map);

    assert_eq!(
        rust_bm.len(),
        baseline_bm.len(),
        "BLOCKMAP length differs for {wad_name}/{map}: rust {} vs baseline {}",
        rust_bm.len(),
        baseline_bm.len()
    );
    if rust_bm != baseline_bm {
        let first_diff = rust_bm.iter().zip(baseline_bm.iter()).position(|(a, b)| a != b).unwrap();
        panic!(
            "BLOCKMAP differs for {wad_name}/{map} starting at byte {first_diff} (of {} bytes)",
            rust_bm.len()
        );
    }
}

#[test]
fn doom_e1m1() {
    assert_blockmap_matches("doom.wad", "E1M1");
}

#[test]
fn doom2_map01() {
    assert_blockmap_matches("doom2.wad", "MAP01");
}

#[test]
fn doom2_map30() {
    assert_blockmap_matches("doom2.wad", "MAP30");
}

#[test]
fn hexen_map01() {
    assert_blockmap_matches("hexen.wad", "MAP01");
}

#[test]
fn heretic_e1m1() {
    assert_blockmap_matches("heretic.wad", "E1M1");
}

#[test]
fn sweep_all_doom2_maps() {
    let Some(_) = baseline_bin() else { return };
    let input = PathBuf::from(CORPUS).join("doom2.wad");
    if !input.is_file() {
        eprintln!("SKIP: doom2.wad not present");
        return;
    }
    let reader = WadReader::open(&input).unwrap();
    let mut idx = reader.next_map(-1);
    let mut count = 0usize;
    while idx >= 0 {
        let name = reader.lump_name(idx).into_owned();
        assert_blockmap_matches("doom2.wad", &name);
        count += 1;
        idx = reader.next_map(idx);
    }
    assert_eq!(count, 32);
}
