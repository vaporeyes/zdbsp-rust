// ABOUTME: Node-builder root module. Owns the FNodeBuilder skeleton and the internal
// ABOUTME: types (PrivSeg, PrivVert, etc.) shared across the partition / extract code.
// ABOUTME: Phase 4a wires up the types; algorithms land in 4b-4e.

use crate::fixed::Fixed;
use crate::workdata::{Node, Subsector};

pub mod build;
pub mod classify;
pub mod events;
pub mod extract;
pub mod extract_gl;
pub mod gl;
pub mod types;
pub mod util;

pub use types::*;

/// Tunable build parameters. The C++ uses extern globals (`MaxSegs`, `SplitCost`,
/// `AAPreference`) defined in main.cpp; defaults match those.
#[derive(Debug, Clone, Copy)]
pub struct BuildOptions {
    /// Maximum segs per node — used to throttle SelectSplitter on huge maps.
    pub max_segs: i32,
    /// Score penalty per split when evaluating splitters.
    pub split_cost: i32,
    /// Axis-aligned splitter preference weight.
    pub aa_preference: i32,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            max_segs: 64,
            split_cost: 8,
            aa_preference: 16,
        }
    }
}

/// Points within this distance of a line are considered to lie on the line. Squared
/// against `s_num^2 / (dx^2 + dy^2)` in `point_on_side`. Matches nodebuild.h:262.
pub const SIDE_EPSILON: f64 = 6.5536;

/// Vertices within this fixed-point distance of each other are coalesced. The C++ uses
/// it for "close enough" lookups in the vertex spatial hash.
pub const VERTEX_EPSILON: Fixed = 6;

/// 4 << 32 — threshold below which we fall back to the precise distance test rather
/// than the cheap dot-product sign check. Matches nodebuild.h:279.
const POINT_ON_SIDE_FAR_THRESHOLD: f64 = 17_179_869_184.0;

/// Classify a point against a directed line. Returns:
/// * `< 0` — point is in front of the line.
/// * `= 0` — point is on the line (within `SIDE_EPSILON`).
/// * `> 0` — point is behind the line.
///
/// Direct port of `FNodeBuilder::PointOnSide`. The math is intentionally done in `f64`
/// to match the C++ — the node builder's correctness depends on the exact rounding.
#[inline]
pub fn point_on_side(x: Fixed, y: Fixed, x1: Fixed, y1: Fixed, dx: Fixed, dy: Fixed) -> i32 {
    let d_dx = dx as f64;
    let d_dy = dy as f64;
    let d_x = x as f64;
    let d_y = y as f64;
    let d_x1 = x1 as f64;
    let d_y1 = y1 as f64;

    let s_num = (d_y1 - d_y) * d_dx - (d_x1 - d_x) * d_dy;

    if s_num.abs() < POINT_ON_SIDE_FAR_THRESHOLD {
        let l = d_dx * d_dx + d_dy * d_dy;
        let dist = s_num * s_num / l;
        if dist < SIDE_EPSILON * SIDE_EPSILON {
            return 0;
        }
    }
    if s_num > 0.0 { -1 } else { 1 }
}

/// Driver for the BSP build. Constructed against a loaded `Level`; algorithm methods
/// will be filled in across Phases 4b-4e.
#[allow(dead_code)] // fields populated by upcoming sub-phases
pub struct NodeBuilder<'a> {
    pub(crate) level: &'a mut crate::level::Level,
    pub(crate) map_name: String,
    pub(crate) gl_nodes: bool,

    pub(crate) nodes: Vec<Node>,
    pub(crate) subsectors: Vec<Subsector>,
    pub(crate) subsector_sets: Vec<u32>,
    pub(crate) segs: Vec<PrivSeg>,
    pub(crate) vertices: Vec<PrivVert>,
    pub(crate) seg_list: Vec<USegPtr>,
    pub(crate) plane_checked: Vec<u8>,
    pub(crate) planes: Vec<SimpleLine>,
    pub(crate) initial_vertices: usize,

    /// Loops a splitter touches on a vertex.
    pub(crate) touched: Vec<i32>,
    /// Loops with edges colinear to a splitter.
    pub(crate) colinear: Vec<i32>,
    /// Vertices intersected by the current splitter.
    pub(crate) events: events::EventTree,
    /// Segs collinear with the current splitter.
    pub(crate) split_sharers: Vec<SplitSharer>,

    /// Seg to force to back of the splitter (used by polyobject containment).
    pub(crate) hack_seg: u32,
    /// Seg to use in front of the hack seg.
    pub(crate) hack_mate: u32,

    pub(crate) poly_starts: Vec<PolyStart>,
    pub(crate) poly_anchors: Vec<PolyStart>,

    /// Progress meter state.
    pub(crate) segs_stuffed: i32,
    /// Tunable build options.
    pub options: BuildOptions,

    // Vertex map (FVertexMap). Populated after FindMapBounds via `init_vertex_map`.
    pub(crate) vmap_min_x: Fixed,
    pub(crate) vmap_min_y: Fixed,
    pub(crate) vmap_max_x: Fixed,
    pub(crate) vmap_max_y: Fixed,
    pub(crate) vmap_blocks_wide: i32,
    pub(crate) vmap_blocks_tall: i32,
    pub(crate) vmap_grid: Vec<Vec<i32>>,
}

/// `(BLOCK_SHIFT, BLOCK_SIZE)` from FVertexMap: 8 + FRACBITS = 24, so each block covers
/// 256 map units of fixed-point space.
pub(crate) const VMAP_BLOCK_SHIFT: u32 = 8 + crate::fixed::FRACBITS;
pub(crate) const VMAP_BLOCK_SIZE: i64 = 1i64 << VMAP_BLOCK_SHIFT;

/// Sentinel used inside the node builder for "no seg" / "no vertex" indices, matching
/// the C++ `DWORD_MAX`.
pub const NO_NODE_INDEX: u32 = 0xffffffff;

impl<'a> NodeBuilder<'a> {
    /// Construct a builder bound to `level`. Algorithm methods are no-ops in Phase 4a.
    pub fn new(
        level: &'a mut crate::level::Level,
        poly_starts: Vec<PolyStart>,
        poly_anchors: Vec<PolyStart>,
        map_name: impl Into<String>,
        gl_nodes: bool,
    ) -> Self {
        Self {
            level,
            map_name: map_name.into(),
            gl_nodes,
            nodes: Vec::new(),
            subsectors: Vec::new(),
            subsector_sets: Vec::new(),
            segs: Vec::new(),
            vertices: Vec::new(),
            seg_list: Vec::new(),
            plane_checked: Vec::new(),
            planes: Vec::new(),
            initial_vertices: 0,
            touched: Vec::new(),
            colinear: Vec::new(),
            events: events::EventTree::new(),
            split_sharers: Vec::new(),
            hack_seg: NO_NODE_INDEX,
            hack_mate: NO_NODE_INDEX,
            poly_starts,
            poly_anchors,
            segs_stuffed: 0,
            options: BuildOptions::default(),
            vmap_min_x: 0,
            vmap_min_y: 0,
            vmap_max_x: 0,
            vmap_max_y: 0,
            vmap_blocks_wide: 0,
            vmap_blocks_tall: 0,
            vmap_grid: Vec::new(),
        }
    }

    /// Stub for the public "get final vertex array" entry point. Phase 4e fills this in.
    pub fn get_vertices(&self) -> Vec<crate::level::WideVertex> {
        Vec::new()
    }

    /// Read-only access to the BSP nodes produced by [`build`](Self::build).
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Read-only access to the subsectors produced by [`build`](Self::build).
    pub fn subsectors(&self) -> &[Subsector] {
        &self.subsectors
    }

    /// Flat list of seg references emitted by `create_subsectors_for_real`.
    pub fn seg_list(&self) -> &[USegPtr] {
        &self.seg_list
    }

    /// Internal seg array (with mid-build splits applied).
    pub fn segs(&self) -> &[PrivSeg] {
        &self.segs
    }

    /// Internal vertex array (initial + intersection vertices).
    pub fn priv_vertices(&self) -> &[PrivVert] {
        &self.vertices
    }

    /// Number of original line-graph vertices (i.e. count after `find_used_vertices`
    /// but before any seg splits introduced builder intersections). Needed by the GL
    /// writers to encode high-bit vertex references.
    pub fn initial_vertex_count(&self) -> usize {
        self.initial_vertices
    }

    /// Run the full build pipeline. Mirrors `FNodeBuilder::FNodeBuilder` from
    /// nodebuild.cpp:42 followed by `BuildTree`:
    ///   init_vertex_map → find_used_vertices → make_segs_from_sides →
    ///   find_poly_containers → group_seg_planes → build_tree.
    pub fn build(&mut self) {
        let (minx, miny, maxx, maxy) = (
            self.level.min_x,
            self.level.min_y,
            self.level.max_x,
            self.level.max_y,
        );
        self.init_vertex_map(minx, miny, maxx, maxy);

        // The C++ passes `Level.Vertices` directly through to FindUsedVertices, which
        // both reads from it and writes new indices back into `Level.Lines`. We snapshot
        // the source vertex list to break the borrow with `self.level.lines`.
        let oldverts = self.level.vertices.clone();
        self.find_used_vertices(&oldverts);

        self.make_segs_from_sides();
        self.find_poly_containers();
        self.group_seg_planes();
        self.build_tree();
    }
}

/// Borrowed view of one logical line during seg construction. Mirrors `FSimpleLine`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SimpleLine {
    pub x: Fixed,
    pub y: Fixed,
    pub dx: Fixed,
    pub dy: Fixed,
}

/// `FEventInfo` from nodebuild.h: an intersection record on the current splitter.
#[derive(Debug, Clone, Copy, Default)]
pub struct EventInfo {
    pub vertex: i32,
    pub front_seg: u32,
}

/// Used to share `Vertex` data between the `Vertex` field of the C++ class and bare
/// fixed-point coords. In Rust the distinction is unnecessary, but keeping the name
/// helps when reading the porting log against the original.
pub use crate::workdata::Vertex as SimpleVert;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_on_side_collinear() {
        // Point on the line itself: y - y1 == 0 && x - x1 == 0 ⇒ s_num == 0.
        assert_eq!(point_on_side(0, 0, 0, 0, 1 << 16, 0), 0);
    }

    #[test]
    fn point_on_side_left_right() {
        // Horizontal line y=0 pointing +x. A point above (positive y) lies to the LEFT
        // of the direction of travel, which the routine reports as "behind" (>0) when
        // s_num > 0 ⇒ returns -1. Verify the polarity matches the C++ contract:
        //   < 0  in front,  > 0  behind.
        let x1 = 0;
        let y1 = 0;
        let dx = 1 << 16; // +x
        let dy = 0;
        // Point at (0, +1<<16): y - y1 = +1<<16, x - x1 = 0
        // s_num = (y1 - y) * dx - (x1 - x) * dy = -(1<<16) * (1<<16) < 0 → returns 1
        assert_eq!(point_on_side(0, 1 << 16, x1, y1, dx, dy), 1);
        // Point at (0, -1<<16): mirror → returns -1.
        assert_eq!(point_on_side(0, -(1 << 16), x1, y1, dx, dy), -1);
    }
}
