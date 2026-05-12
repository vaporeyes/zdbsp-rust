// ABOUTME: ZNODES/ZGLN/XNOD/XGLN writers. Port of processor.cpp:1368-1645. ZNODES
// ABOUTME: are zlib-compressed; XNOD are the same payload, uncompressed. We use the
// ABOUTME: flate2 crate with the real-zlib backend to match the C++ deflate stream byte-for-byte.

use std::io::{self, Write};

use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::fixed::FRACBITS;
use crate::level::{MapNodeEx, MapSegEx, MapSegGlEx, MapSubsectorEx, WideVertex, NO_INDEX};
use crate::wad::WadWriter;

/// Serialize the BSP payload for a non-GL build into a `Vec<u8>` (uncompressed).
/// Mirrors the order in WriteBSPZ / WriteBSPX:
///   - vertices: u32 orgverts, u32 newverts, then newverts × (i32 x, i32 y)
///   - subsectors: u32 count, then count × u32 numlines
///   - segs: u32 count, then count × (u32 v1, u32 v2, u16 linedef, u8 side)
///   - nodes: u32 count, then count × node record
fn serialize_bsp_payload(
    vertices: &[WideVertex],
    subsectors: &[MapSubsectorEx],
    segs: &[MapSegEx],
    nodes: &[MapNodeEx],
    num_org_verts: u32,
) -> Vec<u8> {
    let added = &vertices[num_org_verts.min(vertices.len() as u32) as usize..];
    let mut buf = Vec::new();
    buf.extend_from_slice(&num_org_verts.to_le_bytes());
    buf.extend_from_slice(&(added.len() as u32).to_le_bytes());
    for v in added {
        buf.extend_from_slice(&v.x.to_le_bytes());
        buf.extend_from_slice(&v.y.to_le_bytes());
    }
    buf.extend_from_slice(&(subsectors.len() as u32).to_le_bytes());
    for s in subsectors {
        buf.extend_from_slice(&s.numlines.to_le_bytes());
    }
    buf.extend_from_slice(&(segs.len() as u32).to_le_bytes());
    for s in segs {
        buf.extend_from_slice(&s.v1.to_le_bytes());
        buf.extend_from_slice(&s.v2.to_le_bytes());
        buf.extend_from_slice(&(s.linedef as u16).to_le_bytes());
        buf.push(s.side as u8);
    }
    buf.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
    for n in nodes {
        // nodever < 3: emit splitter coords as SWORD (i16 after >>16). v3 fractional
        // splitters are deferred until we support them.
        buf.extend_from_slice(&((n.x >> FRACBITS) as i16).to_le_bytes());
        buf.extend_from_slice(&((n.y >> FRACBITS) as i16).to_le_bytes());
        buf.extend_from_slice(&((n.dx >> FRACBITS) as i16).to_le_bytes());
        buf.extend_from_slice(&((n.dy >> FRACBITS) as i16).to_le_bytes());
        for j in 0..2 {
            for k in 0..4 {
                buf.extend_from_slice(&n.bbox[j][k].to_le_bytes());
            }
        }
        buf.extend_from_slice(&n.children[0].to_le_bytes());
        buf.extend_from_slice(&n.children[1].to_le_bytes());
    }
    buf
}

/// Same shape as `serialize_bsp_payload` but for GL output. The C++ `nodever` is
/// determined from the line count and presence of fractional splitters:
/// * 1 (`ZGLN`) — fewer than 65535 linedefs, integer splitters
/// * 2 (`ZGL2`) — 65535+ linedefs
/// * 3 (`ZGL3`) — fractional splitters (any line count)
fn serialize_gl_bsp_payload(
    vertices: &[WideVertex],
    subsectors: &[MapSubsectorEx],
    segs: &[MapSegGlEx],
    nodes: &[MapNodeEx],
    num_org_verts: u32,
    nodever: u32,
) -> Vec<u8> {
    let added = &vertices[num_org_verts.min(vertices.len() as u32) as usize..];
    let mut buf = Vec::new();
    buf.extend_from_slice(&num_org_verts.to_le_bytes());
    buf.extend_from_slice(&(added.len() as u32).to_le_bytes());
    for v in added {
        buf.extend_from_slice(&v.x.to_le_bytes());
        buf.extend_from_slice(&v.y.to_le_bytes());
    }
    buf.extend_from_slice(&(subsectors.len() as u32).to_le_bytes());
    for s in subsectors {
        buf.extend_from_slice(&s.numlines.to_le_bytes());
    }
    buf.extend_from_slice(&(segs.len() as u32).to_le_bytes());
    if nodever < 2 {
        for s in segs {
            buf.extend_from_slice(&s.v1.to_le_bytes());
            buf.extend_from_slice(&s.partner.to_le_bytes());
            buf.extend_from_slice(&(s.linedef as u16).to_le_bytes());
            buf.push(s.side as u8);
        }
    } else {
        for s in segs {
            buf.extend_from_slice(&s.v1.to_le_bytes());
            buf.extend_from_slice(&s.partner.to_le_bytes());
            buf.extend_from_slice(&s.linedef.to_le_bytes());
            buf.push(s.side as u8);
        }
    }
    buf.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
    for n in nodes {
        if nodever < 3 {
            buf.extend_from_slice(&((n.x >> FRACBITS) as i16).to_le_bytes());
            buf.extend_from_slice(&((n.y >> FRACBITS) as i16).to_le_bytes());
            buf.extend_from_slice(&((n.dx >> FRACBITS) as i16).to_le_bytes());
            buf.extend_from_slice(&((n.dy >> FRACBITS) as i16).to_le_bytes());
        } else {
            buf.extend_from_slice(&n.x.to_le_bytes());
            buf.extend_from_slice(&n.y.to_le_bytes());
            buf.extend_from_slice(&n.dx.to_le_bytes());
            buf.extend_from_slice(&n.dy.to_le_bytes());
        }
        for j in 0..2 {
            for k in 0..4 {
                buf.extend_from_slice(&n.bbox[j][k].to_le_bytes());
            }
        }
        buf.extend_from_slice(&n.children[0].to_le_bytes());
        buf.extend_from_slice(&n.children[1].to_le_bytes());
    }
    buf
}

fn deflate_max(payload: &[u8]) -> io::Result<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(9));
    enc.write_all(payload)?;
    enc.finish()
}

/// Write a "ZNOD"-prefixed compressed NODES lump. Mirrors WriteBSPZ.
pub fn write_bspz(
    out: &mut WadWriter,
    lump: &str,
    vertices: &[WideVertex],
    subsectors: &[MapSubsectorEx],
    segs: &[MapSegEx],
    nodes: &[MapNodeEx],
    num_org_verts: u32,
) -> io::Result<()> {
    out.start_lump(lump)?;
    out.add_to_lump(b"ZNOD")?;
    let payload = serialize_bsp_payload(vertices, subsectors, segs, nodes, num_org_verts);
    let compressed = deflate_max(&payload)?;
    out.add_to_lump(&compressed)
}

/// Write a compressed GL block. Tag is one of "ZGLN", "ZGL2", "ZGL3" depending on
/// line count / fractional splitter presence.
pub fn write_gl_bspz(
    out: &mut WadWriter,
    lump: &str,
    vertices: &[WideVertex],
    subsectors: &[MapSubsectorEx],
    segs: &[MapSegGlEx],
    nodes: &[MapNodeEx],
    num_org_verts: u32,
    num_lines: usize,
) -> io::Result<()> {
    let frac = nodes.iter().any(|n| {
        (n.x | n.y | n.dx | n.dy) & 0x0000_FFFF != 0
    });
    let (tag, nodever): (&[u8; 4], u32) = if frac {
        (b"ZGL3", 3)
    } else if num_lines < 65535 {
        (b"ZGLN", 1)
    } else {
        (b"ZGL2", 2)
    };
    out.start_lump(lump)?;
    out.add_to_lump(tag)?;
    let payload = serialize_gl_bsp_payload(vertices, subsectors, segs, nodes, num_org_verts, nodever);
    let compressed = deflate_max(&payload)?;
    out.add_to_lump(&compressed)
}

/// Write a "XNOD"-prefixed uncompressed extended NODES lump (mirrors WriteBSPX).
pub fn write_bspx(
    out: &mut WadWriter,
    lump: &str,
    vertices: &[WideVertex],
    subsectors: &[MapSubsectorEx],
    segs: &[MapSegEx],
    nodes: &[MapNodeEx],
    num_org_verts: u32,
) -> io::Result<()> {
    out.start_lump(lump)?;
    out.add_to_lump(b"XNOD")?;
    let payload = serialize_bsp_payload(vertices, subsectors, segs, nodes, num_org_verts);
    out.add_to_lump(&payload)
}

/// Write an extended uncompressed GL block (XGLN/XGL2/XGL3 magic; same body as ZGL*).
pub fn write_gl_bspx(
    out: &mut WadWriter,
    lump: &str,
    vertices: &[WideVertex],
    subsectors: &[MapSubsectorEx],
    segs: &[MapSegGlEx],
    nodes: &[MapNodeEx],
    num_org_verts: u32,
    num_lines: usize,
) -> io::Result<()> {
    let frac = nodes.iter().any(|n| {
        (n.x | n.y | n.dx | n.dy) & 0x0000_FFFF != 0
    });
    let (tag, nodever): (&[u8; 4], u32) = if frac {
        (b"XGL3", 3)
    } else if num_lines < 65535 {
        (b"XGLN", 1)
    } else {
        (b"XGL2", 2)
    };
    out.start_lump(lump)?;
    out.add_to_lump(tag)?;
    let payload = serialize_gl_bsp_payload(vertices, subsectors, segs, nodes, num_org_verts, nodever);
    out.add_to_lump(&payload)
}

// silence: NO_INDEX may be unused if all callers stop using it
#[allow(dead_code)]
fn _no_idx() -> u32 {
    NO_INDEX
}
