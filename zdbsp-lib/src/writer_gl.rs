// ABOUTME: GL output lump writers. Port of WriteGLVertices/Segs/SSect/Nodes from
// ABOUTME: processor.cpp:1236-1366. v2 format only — v5 is deferred.

use std::io;

use crate::fixed::FRACBITS;
use crate::level::{MapNodeEx, MapSegGlEx, MapSubsectorEx, WideVertex, NO_INDEX};
use crate::wad::WadWriter;
use crate::writer::NF_SUBSECTOR;

/// GL_VERT v2 magic.
const GL_VERT_MAGIC_V2: [u8; 4] = *b"gNd2";

/// Write GL_VERT: 4 magic bytes followed by `(x, y)` fixed-point pairs for the
/// builder-added vertices only (indices `[num_org_verts, vertices.len())`).
pub fn write_gl_vertices(
    out: &mut WadWriter,
    vertices: &[WideVertex],
    num_org_verts: usize,
) -> io::Result<()> {
    let added = &vertices[num_org_verts.min(vertices.len())..];
    let mut buf = Vec::with_capacity(4 + added.len() * 8);
    buf.extend_from_slice(&GL_VERT_MAGIC_V2);
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
