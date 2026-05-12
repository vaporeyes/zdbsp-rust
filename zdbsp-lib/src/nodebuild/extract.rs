// ABOUTME: Port of nodebuild_extract.cpp (non-GL paths). Converts the internal BSP
// ABOUTME: into the MapNodeEx / MapSegEx / MapSubsectorEx output arrays consumed by
// ABOUTME: the wad writer. GL extraction (GetGLNodes, CloseSubsector, ...) is Phase 5.

use crate::fixed::FRACBITS;
use crate::level::{MapNodeEx, MapSegEx, MapSubsectorEx, WideVertex, BOX_BOTTOM, BOX_LEFT, BOX_RIGHT, BOX_TOP};
use crate::workdata::NFX_SUBSECTOR;

use super::{PrivSeg, NodeBuilder};

/// Bundled output of the non-GL extraction.
#[derive(Debug, Default)]
pub struct NodeOutput {
    pub vertices: Vec<WideVertex>,
    pub nodes: Vec<MapNodeEx>,
    pub segs: Vec<MapSegEx>,
    pub subsectors: Vec<MapSubsectorEx>,
}

impl<'a> NodeBuilder<'a> {
    /// `GetVertices` (nodebuild_extract.cpp:379). Copies the internal `PrivVert` array
    /// into a fresh `WideVertex` array sized to the final vertex count.
    pub fn extract_vertices(&self) -> Vec<WideVertex> {
        self.vertices
            .iter()
            .map(|v| WideVertex {
                x: v.x,
                y: v.y,
                index: v.index,
            })
            .collect()
    }

    /// `GetNodes` (nodebuild_extract.cpp:392). Strips minisegs, recomputes per-subsector
    /// bounding boxes in map-unit shorts, and produces the three output arrays for the
    /// classic (non-GL) node format.
    pub fn extract_nodes(&self) -> NodeOutput {
        let node_count = self.nodes.len();
        let sub_count = self.subsectors.len();
        let mut nodes = vec![MapNodeEx::default(); node_count];
        let mut subs = vec![MapSubsectorEx::default(); sub_count];
        let mut segs: Vec<MapSegEx> = Vec::with_capacity(self.segs.len());

        // The C++ passes `Nodes.Size() - 1` (the root, since nodes are pushed post-order);
        // when there are zero nodes, that wraps to `(u32)-1` and the recursion's first
        // line short-circuits into "treat as subsector 0".
        let root = if node_count == 0 {
            u32::MAX
        } else {
            (node_count - 1) as u32
        };
        let mut throwaway_bbox = [0i16; 4];
        self.remove_minisegs(&mut nodes, &mut segs, &mut subs, root, &mut throwaway_bbox);

        NodeOutput {
            vertices: self.extract_vertices(),
            nodes,
            segs,
            subsectors: subs,
        }
    }

    fn remove_minisegs(
        &self,
        nodes: &mut [MapNodeEx],
        segs: &mut Vec<MapSegEx>,
        subs: &mut [MapSubsectorEx],
        node: u32,
        bbox: &mut [i16; 4],
    ) -> u32 {
        if node & NFX_SUBSECTOR != 0 {
            let subnum = if node == u32::MAX {
                0
            } else {
                (node & !NFX_SUBSECTOR) as usize
            };
            let numsegs = self.strip_minisegs(segs, subnum, bbox);
            subs[subnum].numlines = numsegs;
            subs[subnum].firstline = segs.len() as u32 - numsegs;
            NFX_SUBSECTOR | (subnum as u32)
        } else {
            let orgnode = self.nodes[node as usize];
            // Recurse into children first; we'll write the parent's fields after.
            let mut bbox0 = [0i16; 4];
            let mut bbox1 = [0i16; 4];
            let child0 = self.remove_minisegs(nodes, segs, subs, orgnode.int_children[0], &mut bbox0);
            let child1 = self.remove_minisegs(nodes, segs, subs, orgnode.int_children[1], &mut bbox1);

            let nn = &mut nodes[node as usize];
            nn.x = orgnode.x;
            nn.y = orgnode.y;
            nn.dx = orgnode.dx;
            nn.dy = orgnode.dy;
            nn.bbox[0] = bbox0;
            nn.bbox[1] = bbox1;
            nn.children[0] = child0;
            nn.children[1] = child1;

            bbox[BOX_TOP] = bbox0[BOX_TOP].max(bbox1[BOX_TOP]);
            bbox[BOX_BOTTOM] = bbox0[BOX_BOTTOM].min(bbox1[BOX_BOTTOM]);
            bbox[BOX_LEFT] = bbox0[BOX_LEFT].min(bbox1[BOX_LEFT]);
            bbox[BOX_RIGHT] = bbox0[BOX_RIGHT].max(bbox1[BOX_RIGHT]);
            node
        }
    }

    fn strip_minisegs(
        &self,
        segs: &mut Vec<MapSegEx>,
        subsector: usize,
        bbox: &mut [i16; 4],
    ) -> u32 {
        bbox[BOX_TOP] = i16::MIN;
        bbox[BOX_BOTTOM] = i16::MAX;
        bbox[BOX_LEFT] = i16::MAX;
        bbox[BOX_RIGHT] = i16::MIN;

        let first = self.subsectors[subsector].first_line as usize;
        let max = first + self.subsectors[subsector].num_lines as usize;
        let mut count: u32 = 0;

        for i in first..max {
            let seg_idx = self.seg_list[i].seg_num as usize;
            let org = &self.segs[seg_idx];

            // SortSegs places minisegs at the end of the subsector, so we can stop on
            // the first one encountered (matches the C++ comment).
            if org.linedef == -1 {
                break;
            }

            self.add_seg_to_short_bbox(bbox, org);

            // The C++ side-determination has two branches followed by a line that
            // unconditionally overwrites the result (extract.cpp:525). That third
            // overwrite is a real bug in the C++ — the sidedef-compression fix-up is
            // immediately discarded. We mirror the bug exactly because byte-identical
            // parity with the C++ baseline is the success criterion.
            let _ = compute_side_corrected(self, org); // dead, kept for documentation
            let ld = &self.level.lines[org.linedef as usize];
            let side = if ld.sidenum[1] as i32 == org.sidedef as i32 {
                1
            } else {
                0
            };

            segs.push(MapSegEx {
                v1: org.v1 as u32,
                v2: org.v2 as u32,
                // angle and offset are >>16 in C++; angle is u32 so the shift is a
                // logical right shift; offset is signed Fixed so the shift is arithmetic.
                angle: (org.angle >> 16) as u16,
                linedef: org.linedef as u16,
                side,
                offset: (org.offset >> FRACBITS) as i16,
            });
            count += 1;
        }
        count
    }

    fn add_seg_to_short_bbox(&self, bbox: &mut [i16; 4], seg: &PrivSeg) {
        let v1 = self.vertices[seg.v1 as usize];
        let v2 = self.vertices[seg.v2 as usize];
        for (x, y) in [(v1.x, v1.y), (v2.x, v2.y)] {
            let sx = (x >> FRACBITS) as i16;
            let sy = (y >> FRACBITS) as i16;
            if sx < bbox[BOX_LEFT] {
                bbox[BOX_LEFT] = sx;
            }
            if sx > bbox[BOX_RIGHT] {
                bbox[BOX_RIGHT] = sx;
            }
            if sy < bbox[BOX_BOTTOM] {
                bbox[BOX_BOTTOM] = sy;
            }
            if sy > bbox[BOX_TOP] {
                bbox[BOX_TOP] = sy;
            }
        }
    }
}

/// What the C++ *would* compute if line 525 of extract.cpp didn't clobber the result.
/// Kept here so the intent is documented; the actual extraction calls the buggy single-
/// branch logic for byte-identical parity.
#[allow(dead_code)]
fn compute_side_corrected(builder: &NodeBuilder, org: &PrivSeg) -> i16 {
    let ld = &builder.level.lines[org.linedef as usize];
    if ld.sidenum[0] == ld.sidenum[1] {
        let lv1 = &builder.level.vertices[ld.v1 as usize];
        let sv1 = &builder.vertices[org.v1 as usize];
        let sv2 = &builder.vertices[org.v2 as usize];
        let d1x = (sv1.x - lv1.x) as f64;
        let d1y = (sv1.y - lv1.y) as f64;
        let d2x = (sv2.x - lv1.x) as f64;
        let d2y = (sv2.y - lv1.y) as f64;
        let dist1 = d1x * d1x + d1y * d1y;
        let dist2 = d2x * d2x + d2y * d2y;
        if dist1 < dist2 {
            0
        } else {
            1
        }
    } else if ld.sidenum[1] as i32 == org.sidedef as i32 {
        1
    } else {
        0
    }
}
