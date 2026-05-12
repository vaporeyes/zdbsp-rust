// ABOUTME: Internal data types used inside FNodeBuilder. Direct port of the nested
// ABOUTME: structs in nodebuild.h: FPrivSeg, FPrivVert, USegPtr, FSplitSharer, FPolyStart.

use crate::fixed::Fixed;

/// One half-edge in the BSP build. Indices reference the builder's internal `segs`,
/// `vertices`, and `planes` arrays. `next`, `next_for_vert`, and `next_for_vert2` are
/// intrusive linked-list pointers (each `u32` is an index, with `NO_NODE_INDEX` as the
/// null sentinel).
#[derive(Debug, Clone, Copy)]
pub struct PrivSeg {
    pub v1: i32,
    pub v2: i32,
    pub sidedef: u32,
    pub linedef: i32,
    pub frontsector: i32,
    pub backsector: i32,
    pub next: u32,
    pub next_for_vert: u32,
    pub next_for_vert2: u32,
    /// Loop number for split avoidance (0 means splitting is okay).
    pub loop_num: i32,
    /// Seg on the back side.
    pub partner: u32,
    /// Seg number in the GL_SEGS lump.
    pub stored_seg: u32,
    pub angle: u32,
    pub offset: Fixed,
    pub plane_num: i32,
    pub plane_front: bool,
    /// Index in the per-plane hash chain (replaces the raw `FPrivSeg *hashnext` pointer
    /// in C++). `NO_NODE_INDEX` ends the chain.
    pub hash_next: u32,
}

impl Default for PrivSeg {
    fn default() -> Self {
        Self {
            v1: 0,
            v2: 0,
            sidedef: 0,
            linedef: 0,
            frontsector: 0,
            backsector: -1,
            next: super::NO_NODE_INDEX,
            next_for_vert: super::NO_NODE_INDEX,
            next_for_vert2: super::NO_NODE_INDEX,
            loop_num: 0,
            partner: super::NO_NODE_INDEX,
            stored_seg: super::NO_NODE_INDEX,
            angle: 0,
            offset: 0,
            plane_num: -1,
            plane_front: false,
            hash_next: super::NO_NODE_INDEX,
        }
    }
}

/// One vertex in the build, plus the two doubly-linked-list heads for the segs that use
/// it as their v1 / v2 endpoint. Equality is geometric (matches C++ `operator==`).
#[derive(Debug, Clone, Copy, Default)]
pub struct PrivVert {
    pub x: Fixed,
    pub y: Fixed,
    /// Head of the list of segs that have this vertex as `v1`.
    pub segs: u32,
    /// Head of the list of segs that have this vertex as `v2`.
    pub segs2: u32,
    pub index: i32,
}

impl PartialEq for PrivVert {
    fn eq(&self, other: &Self) -> bool {
        self.x == other.x && self.y == other.y
    }
}
impl Eq for PrivVert {}

/// `USegPtr` from the C++: in C++ this is a union of `DWORD SegNum` and `FPrivSeg *`.
/// We use the seg-index form throughout — the pointer form was only used to walk the
/// `segs` array, which is identical to indexing it.
#[derive(Debug, Clone, Copy, Default)]
pub struct USegPtr {
    pub seg_num: u32,
}

/// Records a seg collinear with the current splitter so that the split loop can decide
/// later whether the seg lies in front of or behind that splitter.
#[derive(Debug, Clone, Copy)]
pub struct SplitSharer {
    pub distance: f64,
    pub seg: u32,
    pub forward: bool,
}

impl Default for SplitSharer {
    fn default() -> Self {
        Self {
            distance: 0.0,
            seg: super::NO_NODE_INDEX,
            forward: false,
        }
    }
}

/// Polyobject anchor or spawn point passed in from the processor.
#[derive(Debug, Clone, Copy, Default)]
pub struct PolyStart {
    pub polynum: i32,
    pub x: Fixed,
    pub y: Fixed,
}
