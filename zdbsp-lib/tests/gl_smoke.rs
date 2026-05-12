// ABOUTME: GL node builder smoke tests. Runs the full pipeline with `gl_nodes=true`
// ABOUTME: and asserts the GL output arrays are well-formed. Byte-level parity against
// ABOUTME: the C++ baseline awaits the GL writer in Phase 5c.

use std::path::PathBuf;
use zdbsp_lib::nodebuild::{util::collect_poly_spots, NodeBuilder};
use zdbsp_lib::processor::Processor;
use zdbsp_lib::wad::WadReader;

const CORPUS: &str = "/Users/jsh/media/doom_wads";

fn build_gl(wad: &str, map: &str) -> Option<zdbsp_lib::nodebuild::extract_gl::GlNodeOutput> {
    let p = PathBuf::from(CORPUS).join(wad);
    if !p.is_file() {
        return None;
    }
    let mut r = WadReader::open(&p).unwrap();
    let idx = r.find_lump(map, 0);
    let mut proc = Processor::load(&mut r, idx, false).unwrap();
    let (starts, anchors) = collect_poly_spots(&proc.level);
    let mut nb = NodeBuilder::new(&mut proc.level, starts, anchors, map, true);
    nb.build();
    Some(nb.extract_gl())
}

#[test]
fn gl_build_doom_e1m1() {
    let Some(out) = build_gl("doom.wad", "E1M1") else {
        return;
    };
    assert!(!out.nodes.is_empty(), "GL nodes empty");
    assert!(!out.subsectors.is_empty(), "GL subsectors empty");
    assert!(!out.segs.is_empty(), "GL segs empty");
    assert!(!out.vertices.is_empty(), "GL vertices empty");

    // Subsector seg ranges cover the seg array.
    let total: u32 = out.subsectors.iter().map(|s| s.numlines).sum();
    assert_eq!(total as usize, out.segs.len());
    for (i, s) in out.subsectors.iter().enumerate() {
        assert!(s.numlines > 0, "GL subsector {i} has no segs");
        assert!(
            (s.firstline + s.numlines) as usize <= out.segs.len(),
            "GL subsector {i} segs spill"
        );
    }
}

#[test]
fn gl_build_doom2_map01() {
    let Some(out) = build_gl("doom2.wad", "MAP01") else {
        return;
    };
    assert!(!out.nodes.is_empty());
    assert!(!out.segs.is_empty());
}

#[test]
fn gl_build_hexen_map01_with_polyobjects() {
    // Hexen MAP01 has polyobjects, which exercise the AddMinisegs / CheckLoopStart /
    // CheckLoopEnd paths.
    let Some(out) = build_gl("hexen.wad", "MAP01") else {
        return;
    };
    assert!(!out.nodes.is_empty());
    assert!(!out.segs.is_empty());
}

#[test]
fn gl_sweep_doom2() {
    let p = PathBuf::from(CORPUS).join("doom2.wad");
    if !p.is_file() {
        return;
    }
    let reader = WadReader::open(&p).unwrap();
    let mut idx = reader.next_map(-1);
    let mut count = 0usize;
    while idx >= 0 {
        let name = reader.lump_name(idx).into_owned();
        let out = build_gl("doom2.wad", &name).unwrap();
        assert!(!out.nodes.is_empty(), "{name}: empty GL nodes");
        count += 1;
        idx = reader.next_map(idx);
    }
    assert_eq!(count, 32);
}
