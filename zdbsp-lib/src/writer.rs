// ABOUTME: Map writer. Port of the output half of processor.cpp for classic (non-GL,
// ABOUTME: non-compressed) WAD output. Emits VERTEXES, SEGS, SSECTORS, NODES,
// ABOUTME: BLOCKMAP, REJECT and re-emits LINEDEFS/SIDEDEFS/SECTORS with pruning applied.

use std::io;

use crate::blockmap;
use crate::fixed::FRACBITS;
use crate::level::{Level, MapNodeEx, MapSubsectorEx, NO_INDEX};
use crate::nodebuild::extract::NodeOutput;
use crate::processor::MapFormat;
use crate::wad::{WadReader, WadWriter};

/// `NF_SUBSECTOR` flag used in the classic 16-bit NODES lump child references. Matches
/// `doomdata.h:160`.
pub const NF_SUBSECTOR: u16 = 0x8000;

/// What to do with the REJECT lump. Mirrors `ERejectMode` from zdbsp.h.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RejectMode {
    /// Leave the input wad's REJECT unchanged (with sector remapping fixups if pruning
    /// changed the count).
    #[default]
    DontTouch,
    /// Emit a zero-length REJECT lump.
    Create0,
    /// Emit a REJECT filled with zero bytes (every sector visible from every other).
    CreateZeroes,
}

/// What to do with the BLOCKMAP lump. Mirrors `EBlockmapMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlockmapMode {
    #[default]
    Rebuild,
    Create0,
}

/// Per-map writer options.
#[derive(Debug, Clone, Copy)]
pub struct WriterOptions {
    pub blockmap_mode: BlockmapMode,
    pub reject_mode: RejectMode,
    pub build_nodes: bool,
}

impl Default for WriterOptions {
    fn default() -> Self {
        Self {
            blockmap_mode: BlockmapMode::Rebuild,
            reject_mode: RejectMode::DontTouch,
            build_nodes: true,
        }
    }
}

/// Emit a fully-built map to `out`. Mirrors `FProcessor::Write` from processor.cpp:511 for
/// the classic (non-GL, non-compressed) path.
///
/// * `wad` — the source wad; used to copy through THINGS, BEHAVIOR, SCRIPTS, and (when
///   `build_nodes` is false) SEGS/SSECTORS/NODES.
/// * `map_lump` — index of the map header in `wad`.
/// * `level` — fully loaded + pruned map.
/// * `nodes` — extracted node-builder output for this map.
/// * `format` — Doom or Hexen (UDMF is not supported in Phase 5b).
pub fn write_map(
    out: &mut WadWriter,
    wad: &mut WadReader,
    map_lump: i32,
    level: &Level,
    nodes: &NodeOutput,
    format: MapFormat,
    opts: WriterOptions,
) -> io::Result<()> {
    let extended = matches!(format, MapFormat::Hexen);

    // Handle the "empty map" case from processor.cpp:513. Copies the original lumps
    // and emits empty labels for SEGS/SSECTORS/NODES/REJECT/BLOCKMAP.
    if level.num_lines() == 0
        || level.num_sides() == 0
        || level.num_sectors() == 0
        || level.num_vertices() == 0
    {
        out.copy_lump(wad, map_lump)?;
        copy_map_lump(out, wad, map_lump, "THINGS")?;
        copy_map_lump(out, wad, map_lump, "LINEDEFS")?;
        copy_map_lump(out, wad, map_lump, "SIDEDEFS")?;
        copy_map_lump(out, wad, map_lump, "VERTEXES")?;
        out.create_label("SEGS")?;
        out.create_label("SSECTORS")?;
        out.create_label("NODES")?;
        copy_map_lump(out, wad, map_lump, "SECTORS")?;
        out.create_label("REJECT")?;
        out.create_label("BLOCKMAP")?;
        if extended {
            copy_map_lump(out, wad, map_lump, "BEHAVIOR")?;
            copy_map_lump(out, wad, map_lump, "SCRIPTS")?;
        }
        return Ok(());
    }

    // Header label (the map name).
    out.copy_lump(wad, map_lump)?;
    copy_map_lump(out, wad, map_lump, "THINGS")?;
    write_lines(out, level, extended)?;
    write_sides(out, level)?;
    write_vertices(out, &nodes.vertices)?;

    if opts.build_nodes {
        write_segs(out, &nodes.segs)?;
        write_ssectors(out, &nodes.subsectors)?;
        write_nodes(out, &nodes.nodes)?;
    } else {
        copy_map_lump(out, wad, map_lump, "SEGS")?;
        copy_map_lump(out, wad, map_lump, "SSECTORS")?;
        copy_map_lump(out, wad, map_lump, "NODES")?;
    }

    write_sectors(out, level)?;
    write_reject(out, wad, map_lump, level, opts.reject_mode)?;
    write_blockmap(out, level, opts.blockmap_mode)?;

    if extended {
        copy_map_lump(out, wad, map_lump, "BEHAVIOR")?;
        copy_map_lump_optional(out, wad, map_lump, "SCRIPTS")?;
    }

    Ok(())
}

fn copy_map_lump(out: &mut WadWriter, wad: &mut WadReader, map: i32, name: &str) -> io::Result<()> {
    let idx = wad.find_map_lump(name, map);
    if idx >= 0 {
        out.copy_lump(wad, idx)?;
    } else {
        out.create_label(name)?;
    }
    Ok(())
}

fn copy_map_lump_optional(
    out: &mut WadWriter,
    wad: &mut WadReader,
    map: i32,
    name: &str,
) -> io::Result<()> {
    let idx = wad.find_map_lump(name, map);
    if idx >= 0 {
        out.copy_lump(wad, idx)?;
    }
    Ok(())
}

/// VERTEXES: `i16(x>>16), i16(y>>16)` pairs.
fn write_vertices(out: &mut WadWriter, verts: &[crate::level::WideVertex]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(verts.len() * 4);
    for v in verts {
        buf.extend_from_slice(&((v.x >> FRACBITS) as i16).to_le_bytes());
        buf.extend_from_slice(&((v.y >> FRACBITS) as i16).to_le_bytes());
    }
    out.write_lump("VERTEXES", &buf)
}

/// LINEDEFS: 14 bytes per record (Doom) or 16 bytes (Hexen). Re-emit so widened
/// sidenum/v1/v2 fit back into u16.
fn write_lines(out: &mut WadWriter, level: &Level, extended: bool) -> io::Result<()> {
    let record = if extended { 16 } else { 14 };
    let mut buf = Vec::with_capacity(level.lines.len() * record);
    for line in &level.lines {
        if extended {
            buf.extend_from_slice(&(line.v1 as u16).to_le_bytes());
            buf.extend_from_slice(&(line.v2 as u16).to_le_bytes());
            buf.extend_from_slice(&(line.flags as i16).to_le_bytes());
            buf.push(line.special as u8);
            for a in &line.args {
                buf.push(*a as u8);
            }
            buf.extend_from_slice(&(line.sidenum[0] as u16).to_le_bytes());
            buf.extend_from_slice(&(line.sidenum[1] as u16).to_le_bytes());
        } else {
            buf.extend_from_slice(&(line.v1 as u16).to_le_bytes());
            buf.extend_from_slice(&(line.v2 as u16).to_le_bytes());
            buf.extend_from_slice(&(line.flags as i16).to_le_bytes());
            // Doom format stashed `special` and `tag` in args[0..1] at load time
            // (processor.cpp:205-207). Restore them here.
            buf.extend_from_slice(&(line.args[0] as i16).to_le_bytes());
            buf.extend_from_slice(&(line.args[1] as i16).to_le_bytes());
            buf.extend_from_slice(&(line.sidenum[0] as u16).to_le_bytes());
            buf.extend_from_slice(&(line.sidenum[1] as u16).to_le_bytes());
        }
    }
    out.write_lump("LINEDEFS", &buf)
}

/// SIDEDEFS: 30 bytes per record. Re-emit after pruning.
fn write_sides(out: &mut WadWriter, level: &Level) -> io::Result<()> {
    let mut buf = Vec::with_capacity(level.sides.len() * 30);
    for s in &level.sides {
        buf.extend_from_slice(&s.texture_offset.to_le_bytes());
        buf.extend_from_slice(&s.row_offset.to_le_bytes());
        buf.extend_from_slice(&s.top_texture);
        buf.extend_from_slice(&s.bottom_texture);
        buf.extend_from_slice(&s.mid_texture);
        buf.extend_from_slice(&(s.sector as u16).to_le_bytes());
    }
    out.write_lump("SIDEDEFS", &buf)
}

/// SECTORS: 26 bytes per record. Re-emit after pruning.
fn write_sectors(out: &mut WadWriter, level: &Level) -> io::Result<()> {
    let mut buf = Vec::with_capacity(level.sectors.len() * 26);
    for s in &level.sectors {
        buf.extend_from_slice(&s.data.floor_height.to_le_bytes());
        buf.extend_from_slice(&s.data.ceiling_height.to_le_bytes());
        buf.extend_from_slice(&s.data.floor_pic);
        buf.extend_from_slice(&s.data.ceiling_pic);
        buf.extend_from_slice(&s.data.light_level.to_le_bytes());
        buf.extend_from_slice(&s.data.special.to_le_bytes());
        buf.extend_from_slice(&s.data.tag.to_le_bytes());
    }
    out.write_lump("SECTORS", &buf)
}

/// SEGS: 12 bytes per record (classic format). Truncates u32 fields to u16.
fn write_segs(out: &mut WadWriter, segs: &[crate::level::MapSegEx]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(segs.len() * 12);
    for s in segs {
        buf.extend_from_slice(&(s.v1 as u16).to_le_bytes());
        buf.extend_from_slice(&(s.v2 as u16).to_le_bytes());
        buf.extend_from_slice(&s.angle.to_le_bytes());
        buf.extend_from_slice(&s.linedef.to_le_bytes());
        buf.extend_from_slice(&s.side.to_le_bytes());
        buf.extend_from_slice(&s.offset.to_le_bytes());
    }
    out.write_lump("SEGS", &buf)
}

/// SSECTORS: 4 bytes per record (numlines u16, firstline u16).
fn write_ssectors(out: &mut WadWriter, subs: &[MapSubsectorEx]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(subs.len() * 4);
    for s in subs {
        buf.extend_from_slice(&(s.numlines as u16).to_le_bytes());
        buf.extend_from_slice(&(s.firstline as u16).to_le_bytes());
    }
    out.write_lump("SSECTORS", &buf)
}

/// NODES: 28 bytes per record. Truncates dx/dy/x/y from Fixed to i16, bbox already in
/// i16, children mask `NFX_SUBSECTOR` → `NF_SUBSECTOR` (0x8000).
fn write_nodes(out: &mut WadWriter, nodes: &[MapNodeEx]) -> io::Result<()> {
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
            // The C++ does `child - (NFX_SUBSECTOR + NF_SUBSECTOR)` which simplifies to
            // `(child & ~NFX_SUBSECTOR) | NF_SUBSECTOR`. Same bits.
            let word: u16 = if child & 0x80000000 != 0 {
                ((child & 0x7fffffff) as u16) | NF_SUBSECTOR
            } else {
                child as u16
            };
            buf.extend_from_slice(&word.to_le_bytes());
        }
    }
    out.write_lump("NODES", &buf)
}

fn write_blockmap(out: &mut WadWriter, level: &Level, mode: BlockmapMode) -> io::Result<()> {
    if mode == BlockmapMode::Create0 {
        return out.create_label("BLOCKMAP");
    }
    let words = blockmap::build(level);
    let mut buf = Vec::with_capacity(words.len() * 2);
    for w in words {
        buf.extend_from_slice(&w.to_le_bytes());
    }
    out.write_lump("BLOCKMAP", &buf)
}

fn write_reject(
    out: &mut WadWriter,
    wad: &mut WadReader,
    map_lump: i32,
    level: &Level,
    mode: RejectMode,
) -> io::Result<()> {
    let reject_size = (level.num_sectors() * level.num_sectors() + 7) / 8;
    match mode {
        RejectMode::Create0 => out.create_label("REJECT"),
        RejectMode::CreateZeroes => out.write_lump("REJECT", &vec![0u8; reject_size]),
        RejectMode::DontTouch => {
            let idx = wad.find_map_lump("REJECT", map_lump);
            if idx < 0 {
                return out.create_label("REJECT");
            }
            let raw = wad.read_lump(idx)?;
            let org = level.num_org_sectors as usize;
            let expected = (org * org + 7) / 8;
            if raw.len() != expected {
                return out.create_label("REJECT");
            }
            if level.num_org_sectors as usize != level.num_sectors() {
                let fixed = fix_reject(level, &raw);
                out.write_lump("REJECT", &fixed)
            } else {
                out.write_lump("REJECT", &raw)
            }
        }
    }
}

/// `FixReject` from processor.cpp:836. Remaps a REJECT lump after sector pruning.
///
/// Note: the C++ uses `NumSectors()` (the post-prune count) for BOTH `pnum` and
/// `opnum` (processor.cpp:850-851). The geometrically-correct formula would use
/// `NumOrgSectors` for `opnum`, but we preserve the C++'s computation verbatim
/// for byte-identical output. This shifts which bits get carried over from the
/// source REJECT but produces a well-formed (if slightly different) result.
fn fix_reject(level: &Level, oldreject: &[u8]) -> Vec<u8> {
    let n = level.num_sectors();
    let new_size = (n * n + 7) / 8;
    let mut new_reject = vec![0u8; new_size];
    for y in 0..n {
        let oy = level.org_sector_map[y] as usize;
        for x in 0..n {
            let ox = level.org_sector_map[x] as usize;
            let pnum = y * n + x;
            let opnum = oy * n + ox; // intentional: matches C++ bug
            if (opnum >> 3) < oldreject.len() && oldreject[opnum >> 3] & (1u8 << (opnum & 7)) != 0
            {
                new_reject[pnum >> 3] |= 1u8 << (pnum & 7);
            }
        }
    }
    new_reject
}

// Silence unused-warnings for sentinels referenced only by Phase 5c output.
#[allow(dead_code)]
fn _used_in_5c() {
    let _ = NO_INDEX;
}
