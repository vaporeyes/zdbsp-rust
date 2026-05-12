// ABOUTME: GL output lump writers. Port of WriteGLVertices/Segs/SSect/Nodes from
// ABOUTME: processor.cpp:1236-1366. v2 format only — v5 is deferred.

use std::io;

use crate::fixed::FRACBITS;
use crate::level::{MapNodeEx, MapSegGlEx, MapSubsectorEx, WideVertex, NO_INDEX};
use crate::wad::WadWriter;
use crate::writer::NF_SUBSECTOR;

/// GL_VERT v2 magic.
const GL_VERT_MAGIC_V2: [u8; 4] = *b"gNd2";
/// GL_VERT v5 magic.
const GL_VERT_MAGIC_V5: [u8; 4] = *b"gNd5";

/// Write GL_VERT: 4 magic bytes followed by `(x, y)` fixed-point pairs for the
/// builder-added vertices only (indices `[num_org_verts, vertices.len())`).
/// The `v5` flag swaps the magic from "gNd2" to "gNd5"; record layout is identical.
pub fn write_gl_vertices(
    out: &mut WadWriter,
    vertices: &[WideVertex],
    num_org_verts: usize,
    v5: bool,
) -> io::Result<()> {
    let added = &vertices[num_org_verts.min(vertices.len())..];
    let mut buf = Vec::with_capacity(4 + added.len() * 8);
    buf.extend_from_slice(if v5 { &GL_VERT_MAGIC_V5 } else { &GL_VERT_MAGIC_V2 });
    for v in added {
        buf.extend_from_slice(&v.x.to_le_bytes());
        buf.extend_from_slice(&v.y.to_le_bytes());
    }
    out.write_lump("GL_VERT", &buf)
}

/// Encode a v2 GL vertex reference: WORD-sized, with the high bit (0x8000) set when
/// the index refers to a builder-added vertex (which lives in GL_VERT) rather than the
/// regular VERTEXES lump.
#[inline]
fn encode_gl_v(index: u32, num_org_verts: u32) -> u16 {
    if index < num_org_verts {
        index as u16
    } else {
        0x8000 | ((index - num_org_verts) as u16)
    }
}

/// Write GL_SEGS in v2 format: 10 bytes per record (v1, v2, linedef, side, partner).
pub fn write_gl_segs(
    out: &mut WadWriter,
    segs: &[MapSegGlEx],
    num_org_verts: u32,
) -> io::Result<()> {
    let mut buf = Vec::with_capacity(segs.len() * 10);
    for s in segs {
        buf.extend_from_slice(&encode_gl_v(s.v1, num_org_verts).to_le_bytes());
        buf.extend_from_slice(&encode_gl_v(s.v2, num_org_verts).to_le_bytes());
        // linedef is u32 in MapSegGlEx; truncate to u16 for v2 output. NO_INDEX
        // (0xFFFFFFFF) → 0xFFFF, which is the v2 "miniseg" sentinel.
        let ld = if s.linedef == NO_INDEX {
            0xffff
        } else {
            s.linedef as u16
        };
        buf.extend_from_slice(&ld.to_le_bytes());
        buf.extend_from_slice(&s.side.to_le_bytes());
        buf.extend_from_slice(&(s.partner as u16).to_le_bytes());
    }
    out.write_lump("GL_SEGS", &buf)
}

/// Write GL_SSECT in v2 format (4 bytes per record: numlines, firstline). Identical
/// layout to the regular SSECTORS lump.
pub fn write_gl_ssect(out: &mut WadWriter, subs: &[MapSubsectorEx]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(subs.len() * 4);
    for s in subs {
        buf.extend_from_slice(&(s.numlines as u16).to_le_bytes());
        buf.extend_from_slice(&(s.firstline as u16).to_le_bytes());
    }
    out.write_lump("GL_SSECT", &buf)
}

/// Write GL_SEGS in v5 format. Layout matches `MapSegGLEx` with its natural alignment:
///   u32 v1, u32 v2 (high bit 0x80000000 = builder-added vertex offset)  — 8 bytes
///   u32 linedef (zero-extended from the 16-bit value the C++ writes)    — 4 bytes
///   u16 side, 2 bytes of padding, u32 partner                           — 8 bytes
/// Total = 20 bytes per record. The C++ relies on `new MapSegGLEx[]` happening to
/// land on a zero page so the alignment padding ends up as zero; we emit zeros
/// explicitly to keep the output deterministic and byte-identical.
pub fn write_gl_segs_v5(
    out: &mut WadWriter,
    segs: &[MapSegGlEx],
    num_org_verts: u32,
) -> io::Result<()> {
    let mut buf = Vec::with_capacity(segs.len() * 20);
    for s in segs {
        let v1 = if s.v1 < num_org_verts {
            s.v1
        } else {
            0x80000000 | (s.v1 - num_org_verts)
        };
        let v2 = if s.v2 < num_org_verts {
            s.v2
        } else {
            0x80000000 | (s.v2 - num_org_verts)
        };
        buf.extend_from_slice(&v1.to_le_bytes());
        buf.extend_from_slice(&v2.to_le_bytes());
        // C++: `LittleShort(linedef)` assigned to a DWORD slot zero-extends to 4 bytes.
        buf.extend_from_slice(&(s.linedef as u16 as u32).to_le_bytes());
        buf.extend_from_slice(&s.side.to_le_bytes());
        buf.extend_from_slice(&[0, 0]); // alignment padding before `partner`
        buf.extend_from_slice(&s.partner.to_le_bytes());
    }
    out.write_lump("GL_SEGS", &buf)
}

/// Write GL_SSECT in v5 format (8 bytes per record: u32 numlines, u32 firstline).
pub fn write_gl_ssect_v5(out: &mut WadWriter, subs: &[MapSubsectorEx]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(subs.len() * 8);
    for s in subs {
        buf.extend_from_slice(&s.numlines.to_le_bytes());
        buf.extend_from_slice(&s.firstline.to_le_bytes());
    }
    out.write_lump("GL_SSECT", &buf)
}

/// Write GL_NODES in v5 format. Per-record layout matches `MapNodeExO` (32 bytes):
///   i16 x, y, dx, dy      (8 bytes)
///   i16 bbox[2][4]        (16 bytes)
///   u32 children[2]       (8 bytes)
///
/// **Bug-mirror**: the C++ `WriteNodes5` allocates `MapNodeExO[count * sizeof(MapNodeEx)]`
/// but only `WriteLump`s `count * sizeof(MapNodeEx) = count * 40` bytes — leaving
/// `count * 8` uninitialized trailing bytes per lump. On Apple Silicon those bytes
/// are reliably zero (fresh-page heap), so the output is observable as 32 valid bytes
/// plus 8 zero bytes per record. We emit them explicitly so the result is deterministic.
pub fn write_gl_nodes_v5(out: &mut WadWriter, nodes: &[MapNodeEx]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(nodes.len() * 40);
    for n in nodes {
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
    // Trailing "garbage" — see doc comment above.
    buf.extend(std::iter::repeat(0u8).take(nodes.len() * 8));
    out.write_lump("GL_NODES", &buf)
}

/// Write GL_NODES in v2 format (28 bytes per record; same shape as regular NODES).
pub fn write_gl_nodes(out: &mut WadWriter, nodes: &[MapNodeEx]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(nodes.len() * 28);
    for n in nodes {
        buf.extend_from_slice(&((n.x >> FRACBITS) as i16).to_le_bytes());
        buf.extend_from_slice(&((n.y >> FRACBITS) as i16).to_le_bytes());
        buf.extend_from_slice(&((n.dx >> FRACBITS) as i16).to_le_bytes());
        buf.extend_from_slice(&((n.dy >> FRACBITS) as i16).to_le_bytes());
        for j in 0..2 {
            for k in 0..4 {
                buf.extend_from_slice(&n.bbox[j][k].to_le_bytes());
            }
        }
        for j in 0..2 {
            let child = n.children[j];
            let word = if child & 0x80000000 != 0 {
                ((child & 0x7fffffff) as u16) | NF_SUBSECTOR
            } else {
                child as u16
            };
            buf.extend_from_slice(&word.to_le_bytes());
        }
    }
    out.write_lump("GL_NODES", &buf)
}
