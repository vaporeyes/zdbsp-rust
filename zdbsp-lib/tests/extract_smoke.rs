// ABOUTME: Smoke tests for the node-builder extraction (Phase 4e). Asserts the
// ABOUTME: extracted MapNodeEx/MapSegEx/MapSubsectorEx arrays are well-formed.

use std::path::PathBuf;
use zdbsp_lib::level::OUT_NFX_SUBSECTOR;
use zdbsp_lib::nodebuild::{util::collect_poly_spots, NodeBuilder};
use zdbsp_lib::processor::Processor;
use zdbsp_lib::wad::WadReader;

const CORPUS: &str = "/Users/jsh/media/doom_wads";

fn build(wad_file: &str, map: &str) -> Option<zdbsp_lib::nodebuild::extract::NodeOutput> {
    let path = PathBuf::from(CORPUS).join(wad_file);
    if !path.is_file() {
        eprintln!("SKIP: {wad_file} not present");
        return None;
    }
    let mut reader = WadReader::open(&path).unwrap();
    let idx = reader.find_lump(map, 0);
    assert!(idx >= 0);
    let mut p = Processor::load(&mut reader, idx, false).unwrap();
    let (starts, anchors) = collect_poly_spots(&p.level);
    let mut nb = NodeBuilder::new(&mut p.level, starts, anchors, map, false);
    nb.build();
    Some(nb.extract_nodes())
}

#[test]
fn extract_doom_e1m1_is_well_formed() {
    let Some(out) = build("doom.wad", "E1M1") else { return };
    assert!(!out.nodes.is_empty());
    assert!(!out.subsectors.is_empty());
    assert!(!out.segs.is_empty());
    assert!(!out.vertices.is_empty());

    let nn = out.nodes.len();
    let ns = out.subsectors.len();
    for (i, n) in out.nodes.iter().enumerate() {
        for &child in &n.children {
            if child & OUT_NFX_SUBSECTOR != 0 {
                let s = (child & !OUT_NFX_SUBSECTOR) as usize;
                assert!(s < ns, "node {i} child references subsector {s} >= {ns}");
            } else {
                let c = child as usize;
                assert!(c < nn, "node {i} child references node {c} >= {nn}");
                assert!(c < i, "node {i} references later node {c}");
            }
        }
        // bboxes should be sane.
        for j in 0..2 {
            assert!(n.bbox[j][0] >= n.bbox[j][1], "node {i} bbox{j}: top < bottom");
            assert!(n.bbox[j][3] >= n.bbox[j][2], "node {i} bbox{j}: right < left");
        }
    }

    // Every subsector should account for at least one seg.
    let total_seg_refs: u32 = out.subsectors.iter().map(|s| s.numlines).sum();
    assert_eq!(total_seg_refs as usize, out.segs.len());
    for (i, s) in out.subsectors.iter().enumerate() {
        assert!(s.numlines > 0, "subsector {i} has no segs");
        assert!(
            (s.firstline + s.numlines) as usize <= out.segs.len(),
            "subsector {i} segs spill past array"
        );
    }
}

#[test]
fn extract_every_doom2_map() {
    let path = PathBuf::from(CORPUS).join("doom2.wad");
    if !path.is_file() {
        return;
    }
    let reader = WadReader::open(&path).unwrap();
    let mut idx = reader.next_map(-1);
    let mut count = 0usize;
    while idx >= 0 {
        let name = reader.lump_name(idx).into_owned();
        let mut w = WadReader::open(&path).unwrap();
        let mut p = Processor::load(&mut w, idx, false).unwrap();
        let (starts, anchors) = collect_poly_spots(&p.level);
        let mut nb = NodeBuilder::new(&mut p.level, starts, anchors, &name, false);
        nb.build();
        let out = nb.extract_nodes();
        assert!(!out.nodes.is_empty(), "{name}: no nodes");
        assert!(!out.segs.is_empty(), "{name}: no segs");
        count += 1;
        idx = reader.next_map(idx);
    }
    assert_eq!(count, 32);
}
