// ABOUTME: Internal node-builder working types. Port of workdata.h. These are the
// ABOUTME: in-memory shapes the partition loop manipulates before extraction to the
// ABOUTME: doomdata.h output forms (MapNode, MapSubsector, MapSeg, etc.).

use crate::fixed::Fixed;

/// A 2D vertex in 16.16 fixed-point world space.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Vertex {
    pub x: Fixed,
    pub y: Fixed,
}

/// A BSP partition node during construction. `bbox` is `[front, back]` × `[top, bottom,
/// left, right]`. `int_children` indices point either at another node or, with the
/// `NFX_SUBSECTOR` bit set, at a subsector.
#[derive(Debug, Clone, Copy, Default)]
pub struct Node {
    pub x: Fixed,
    pub y: Fixed,
    pub dx: Fixed,
    pub dy: Fixed,
    pub bbox: [[Fixed; 4]; 2],
    pub int_children: [u32; 2],
}

/// A leaf in the BSP: contiguous run of segs starting at `first_line`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Subsector {
    pub num_lines: u32,
    pub first_line: u32,
}

/// Subsector flag bit on a node's `int_children[]` entry.
pub const NFX_SUBSECTOR: u32 = 0x80000000;
