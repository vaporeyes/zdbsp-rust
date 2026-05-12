// ABOUTME: In-memory representation of a loaded Doom/Hexen map. Port of FLevel and the
// ABOUTME: `Int*` structs in doomdata.h, plus the pruning passes from processor.cpp.

use crate::fixed::Fixed;

/// Sentinel: 0xFFFF on disk, meaning "no side / no vertex / no sector".
pub const NO_MAP_INDEX: u16 = 0xffff;

/// Sentinel: 0xFFFFFFFF, the widened version used in the in-memory `Int*` structs.
pub const NO_INDEX: u32 = 0xffffffff;

/// Blockmap geometry constants (doomdata.h:269-272).
pub const BLOCKSIZE: i32 = 128;
pub const BLOCKBITS: u32 = 7;
pub const BLOCKFRACBITS: u32 = crate::fixed::FRACBITS + 7;
pub const BLOCKFRACSIZE: i32 = BLOCKSIZE << crate::fixed::FRACBITS;

/// Hexen polyobject thing types (used during PolySpot collection in Phase 4+).
pub const PO_HEX_ANCHOR_TYPE: i16 = 3000;
pub const PO_HEX_SPAWN_TYPE: i16 = 3001;
pub const PO_HEX_SPAWNCRUSH_TYPE: i16 = 3002;
pub const PO_ANCHOR_TYPE: i16 = 9300;
pub const PO_SPAWN_TYPE: i16 = 9301;
pub const PO_SPAWNCRUSH_TYPE: i16 = 9302;
pub const PO_SPAWNHURT_TYPE: i16 = 9303;

/// One UDMF key/value pair on a map element. Populated only for UDMF maps; left empty
/// for binary-format loads.
#[derive(Debug, Clone)]
pub struct UdmfKey {
    pub key: String,
    pub value: String,
}

/// Wide-precision vertex: 16.16 fixed-point coords with an index field used by the
/// node builder during vertex deduplication.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WideVertex {
    pub x: Fixed,
    pub y: Fixed,
    pub index: i32,
}

/// In-memory side definition. Texture name fields are kept as raw 8-byte arrays,
/// matching `char[8]` on disk (NUL-padded, no terminator on full-width names).
#[derive(Debug, Clone)]
pub struct IntSideDef {
    pub texture_offset: i16,
    pub row_offset: i16,
    pub top_texture: [u8; 8],
    pub bottom_texture: [u8; 8],
    pub mid_texture: [u8; 8],
    /// Sector index; `NO_INDEX` means "no sector" (widened from `NO_MAP_INDEX`).
    pub sector: u32,
    pub props: Vec<UdmfKey>,
}

impl Default for IntSideDef {
    fn default() -> Self {
        Self {
            texture_offset: 0,
            row_offset: 0,
            top_texture: [0; 8],
            bottom_texture: [0; 8],
            mid_texture: [0; 8],
            sector: NO_INDEX,
            props: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct IntLineDef {
    pub v1: u32,
    pub v2: u32,
    pub flags: i32,
    pub special: i32,
    pub args: [i32; 5],
    /// `[front, back]`; either may be `NO_INDEX` (widened from `NO_MAP_INDEX`).
    pub sidenum: [u32; 2],
    pub props: Vec<UdmfKey>,
}

impl Default for IntLineDef {
    fn default() -> Self {
        Self {
            v1: 0,
            v2: 0,
            flags: 0,
            special: 0,
            args: [0; 5],
            sidenum: [NO_INDEX, NO_INDEX],
            props: Vec::new(),
        }
    }
}

/// Raw 26-byte SECTOR record. The node builder doesn't inspect any field, so the data
/// is kept verbatim for round-tripping.
#[derive(Debug, Clone, Copy)]
pub struct MapSector {
    pub floor_height: i16,
    pub ceiling_height: i16,
    pub floor_pic: [u8; 8],
    pub ceiling_pic: [u8; 8],
    pub light_level: i16,
    pub special: i16,
    pub tag: i16,
}

impl Default for MapSector {
    fn default() -> Self {
        Self {
            floor_height: 0,
            ceiling_height: 0,
            floor_pic: [0; 8],
            ceiling_pic: [0; 8],
            light_level: 0,
            special: 0,
            tag: 0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct IntSector {
    pub data: MapSector,
    pub props: Vec<UdmfKey>,
}

#[derive(Debug, Clone, Default)]
pub struct IntThing {
    pub thingid: u16,
    pub x: Fixed,
    pub y: Fixed,
    pub z: i16,
    pub angle: i16,
    pub kind: i16,
    pub flags: i16,
    pub special: i8,
    pub args: [i8; 5],
    pub props: Vec<UdmfKey>,
}

#[derive(Debug, Clone, Default)]
pub struct IntVertex {
    pub props: Vec<UdmfKey>,
}

/// Extended subsector (32-bit indices). Output of the node builder; consumed by the
/// compressed-node writers. Mirrors `MapSubsectorEx` in doomdata.h.
#[derive(Debug, Clone, Copy, Default)]
pub struct MapSubsectorEx {
    pub numlines: u32,
    pub firstline: u32,
}

/// Extended seg with 32-bit vertex indices. Output of the non-GL extraction path.
/// Mirrors `MapSegEx` in doomdata.h.
#[derive(Debug, Clone, Copy, Default)]
pub struct MapSegEx {
    pub v1: u32,
    pub v2: u32,
    pub angle: u16,
    pub linedef: u16,
    pub side: i16,
    pub offset: i16,
}

/// Extended BSP node with full-precision fixed-point splitter coords. Bounding boxes
/// are stored in `short` (map units) here because that's what the wad writer emits
/// for the classic NODES lump format. Mirrors `MapNodeEx` in doomdata.h.
#[derive(Debug, Clone, Copy, Default)]
pub struct MapNodeEx {
    pub x: Fixed,
    pub y: Fixed,
    pub dx: Fixed,
    pub dy: Fixed,
    pub bbox: [[i16; 4]; 2],
    pub children: [u32; 2],
}

/// Extended GL seg, 32-bit indices. Mirrors `MapSegGLEx` in doomdata.h.
#[derive(Debug, Clone, Copy, Default)]
pub struct MapSegGlEx {
    pub v1: u32,
    pub v2: u32,
    pub linedef: u32,
    pub side: u16,
    pub partner: u32,
}

/// Subsector flag bit on a child reference. Same value as `NFX_SUBSECTOR` in workdata.
pub const OUT_NFX_SUBSECTOR: u32 = 0x80000000;

/// BBox indices used throughout the node builder and output structures.
pub const BOX_TOP: usize = 0;
pub const BOX_BOTTOM: usize = 1;
pub const BOX_LEFT: usize = 2;
pub const BOX_RIGHT: usize = 3;

/// Loaded map. Owns the geometry vectors and (later) the node-builder output.
#[derive(Debug, Default)]
pub struct Level {
    pub vertices: Vec<WideVertex>,
    pub vertex_props: Vec<IntVertex>,
    pub sides: Vec<IntSideDef>,
    pub lines: Vec<IntLineDef>,
    pub sectors: Vec<IntSector>,
    pub things: Vec<IntThing>,
    /// Original sector count before pruning (used later by the reject builder).
    pub num_org_sectors: i32,
    /// `pruned_index -> original_index` mapping, populated only when sectors were pruned.
    pub org_sector_map: Vec<u32>,
    pub min_x: Fixed,
    pub min_y: Fixed,
    pub max_x: Fixed,
    pub max_y: Fixed,
    pub props: Vec<UdmfKey>,
}

impl Level {
    pub fn num_lines(&self) -> usize {
        self.lines.len()
    }
    pub fn num_sides(&self) -> usize {
        self.sides.len()
    }
    pub fn num_sectors(&self) -> usize {
        self.sectors.len()
    }
    pub fn num_things(&self) -> usize {
        self.things.len()
    }
    pub fn num_vertices(&self) -> usize {
        self.vertices.len()
    }

    /// Compute axis-aligned bounding box over the vertex set.
    pub fn find_map_bounds(&mut self) {
        if self.vertices.is_empty() {
            self.min_x = 0;
            self.min_y = 0;
            self.max_x = 0;
            self.max_y = 0;
            return;
        }
        let v0 = self.vertices[0];
        let (mut minx, mut miny) = (v0.x, v0.y);
        let (mut maxx, mut maxy) = (v0.x, v0.y);
        for v in &self.vertices[1..] {
            // Note: the C++ code uses `else if`, so a single vertex can update either
            // min or max but not both. Identical here for behavior parity.
            if v.x < minx {
                minx = v.x;
            } else if v.x > maxx {
                maxx = v.x;
            }
            if v.y < miny {
                miny = v.y;
            } else if v.y > maxy {
                maxy = v.y;
            }
        }
        self.min_x = minx;
        self.min_y = miny;
        self.max_x = maxx;
        self.max_y = maxy;
    }

    /// Remove zero-length linedefs. Collision tests against them would divide by zero,
    /// and ZDoom strips them itself anyway.
    pub fn remove_extra_lines(&mut self) -> usize {
        let before = self.lines.len();
        let verts = &self.vertices;
        self.lines.retain(|l| {
            let v1 = verts.get(l.v1 as usize);
            let v2 = verts.get(l.v2 as usize);
            match (v1, v2) {
                (Some(a), Some(b)) => a.x != b.x || a.y != b.y,
                _ => false,
            }
        });
        before - self.lines.len()
    }

    /// Remove sides that no line references. Renumbers `sidenum[]` on the surviving lines.
    pub fn remove_extra_sides(&mut self) -> usize {
        let n = self.sides.len();
        if n == 0 {
            return 0;
        }
        let mut used = vec![0u8; n];
        for line in &self.lines {
            for &s in &line.sidenum {
                if s != NO_INDEX {
                    if let Some(slot) = used.get_mut(s as usize) {
                        *slot = 1;
                    }
                }
            }
        }
        let mut remap = vec![NO_INDEX; n];
        let mut new_n = 0usize;
        for i in 0..n {
            if used[i] != 0 {
                if i != new_n {
                    self.sides.swap(i, new_n);
                }
                remap[i] = new_n as u32;
                new_n += 1;
            }
        }
        let removed = n - new_n;
        if removed > 0 {
            self.sides.truncate(new_n);
            for line in &mut self.lines {
                for s in &mut line.sidenum {
                    if *s != NO_INDEX {
                        *s = remap[*s as usize];
                    }
                }
            }
        }
        removed
    }

    /// Remove sectors that no side references. Renumbers `sector` on surviving sides and
    /// records a reverse map (`org_sector_map`) used later by reject-table fixups.
    pub fn remove_extra_sectors(&mut self) -> usize {
        let n = self.sectors.len();
        self.num_org_sectors = n as i32;
        if n == 0 {
            return 0;
        }
        let mut used = vec![0u8; n];
        for side in &self.sides {
            if side.sector != NO_INDEX {
                if let Some(slot) = used.get_mut(side.sector as usize) {
                    *slot = 1;
                }
            }
        }
        let mut remap = vec![NO_INDEX; n];
        let mut new_n = 0usize;
        for i in 0..n {
            if used[i] != 0 {
                if i != new_n {
                    self.sectors.swap(i, new_n);
                }
                remap[i] = new_n as u32;
                new_n += 1;
            }
        }
        let removed = n - new_n;
        if removed > 0 {
            for side in &mut self.sides {
                if side.sector != NO_INDEX {
                    side.sector = remap[side.sector as usize];
                }
            }
            self.org_sector_map = vec![0u32; new_n];
            for (orig, &mapped) in remap.iter().enumerate() {
                if mapped != NO_INDEX {
                    self.org_sector_map[mapped as usize] = orig as u32;
                }
            }
            self.sectors.truncate(new_n);
        }
        removed
    }
}
