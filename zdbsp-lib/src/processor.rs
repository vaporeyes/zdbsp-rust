// ABOUTME: Per-map orchestrator. Port of FProcessor's load half from processor.cpp:
// ABOUTME: pulls THINGS/LINEDEFS/SIDEDEFS/VERTEXES/SECTORS out of a WAD and decodes them.

use std::io;

use crate::fixed::from_map_unit;
use crate::level::{
    IntLineDef, IntSector, IntSideDef, IntThing, Level, MapSector, NO_INDEX, NO_MAP_INDEX,
};
use crate::wad::WadReader;

/// On-disk record sizes from doomdata.h. Used both for parsing and for sanity-checking
/// the byte length of each lump before we slice it up.
mod sizes {
    pub const MAP_VERTEX: usize = 4; // i16 x, i16 y
    pub const MAP_SIDEDEF: usize = 30; // i16, i16, [u8;8], [u8;8], [u8;8], u16
    pub const MAP_LINEDEF: usize = 14; // u16 v1, v2, i16 flags, special, tag, u16 sidenum[2]
    pub const MAP_LINEDEF2: usize = 16; // Hexen: u16 v1, v2, i16 flags, u8 special, u8 args[5], u16 sidenum[2]
    pub const MAP_SECTOR: usize = 26; // 5 i16 + 16 bytes of texture names
    pub const MAP_THING: usize = 10; // 5 i16
    pub const MAP_THING2: usize = 20; // Hexen: u16 id, 4 i16, 2 i16, u8 special, u8 args[5]
}

/// Which on-disk format the map header indicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapFormat {
    Doom,
    Hexen,
    Udmf,
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessorError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("required lump {0} missing from map")]
    MissingLump(&'static str),
    #[error("UDMF maps are not yet supported")]
    UdmfNotSupported,
    #[error("lump {name} has size {size} which is not a multiple of {record}")]
    BadLumpSize {
        name: &'static str,
        size: usize,
        record: usize,
    },
}

/// One processed map. After construction it owns a fully-loaded `Level`.
pub struct Processor {
    pub format: MapFormat,
    pub map_lump: i32,
    pub map_name: String,
    pub level: Level,
}

impl Processor {
    /// Load the map whose header lives at `map_lump` in `wad`. Mirrors FProcessor's
    /// constructor: load, then prune unused sides/sectors (unless `no_prune` is set).
    pub fn load(
        wad: &mut WadReader,
        map_lump: i32,
        no_prune: bool,
    ) -> Result<Self, ProcessorError> {
        let map_name = wad.lump_name(map_lump).into_owned();
        let format = if wad.is_udmf(map_lump) {
            MapFormat::Udmf
        } else if wad.map_has_behavior(map_lump) {
            MapFormat::Hexen
        } else {
            MapFormat::Doom
        };

        if format == MapFormat::Udmf {
            return Err(ProcessorError::UdmfNotSupported);
        }

        let extended = format == MapFormat::Hexen;
        let mut level = Level::default();

        load_things(wad, map_lump, extended, &mut level)?;
        load_vertices(wad, map_lump, &mut level)?;
        load_lines(wad, map_lump, extended, &mut level)?;
        load_sides(wad, map_lump, &mut level)?;
        load_sectors(wad, map_lump, &mut level)?;

        let incomplete = level.num_lines() == 0
            || level.num_vertices() == 0
            || level.num_sides() == 0
            || level.num_sectors() == 0;

        if !incomplete {
            level.remove_extra_lines();
            if !no_prune {
                level.remove_extra_sides();
                level.remove_extra_sectors();
            }
            level.find_map_bounds();
        }

        Ok(Self {
            format,
            map_lump,
            map_name,
            level,
        })
    }
}

fn read_lump(
    wad: &mut WadReader,
    map_lump: i32,
    name: &'static str,
    required: bool,
) -> Result<Vec<u8>, ProcessorError> {
    let idx = wad.find_map_lump(name, map_lump);
    if idx < 0 {
        if required {
            return Err(ProcessorError::MissingLump(name));
        }
        return Ok(Vec::new());
    }
    Ok(wad.read_lump(idx)?)
}

fn check_record(name: &'static str, bytes: &[u8], record: usize) -> Result<(), ProcessorError> {
    if !bytes.is_empty() && bytes.len() % record != 0 {
        return Err(ProcessorError::BadLumpSize {
            name,
            size: bytes.len(),
            record,
        });
    }
    Ok(())
}

#[inline]
fn read_i16(buf: &[u8], off: usize) -> i16 {
    i16::from_le_bytes([buf[off], buf[off + 1]])
}

#[inline]
fn read_u16(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

fn widen_sidenum(raw: u16) -> u32 {
    if raw == NO_MAP_INDEX {
        NO_INDEX
    } else {
        raw as u32
    }
}

fn load_things(
    wad: &mut WadReader,
    map_lump: i32,
    extended: bool,
    level: &mut Level,
) -> Result<(), ProcessorError> {
    let bytes = read_lump(wad, map_lump, "THINGS", true)?;
    let record = if extended {
        sizes::MAP_THING2
    } else {
        sizes::MAP_THING
    };
    check_record("THINGS", &bytes, record)?;

    let count = bytes.len() / record;
    level.things.reserve(count);
    for i in 0..count {
        let o = i * record;
        let thing = if extended {
            IntThing {
                thingid: read_u16(&bytes, o),
                x: from_map_unit(read_i16(&bytes, o + 2)),
                y: from_map_unit(read_i16(&bytes, o + 4)),
                z: read_i16(&bytes, o + 6),
                angle: read_i16(&bytes, o + 8),
                kind: read_i16(&bytes, o + 10),
                flags: read_i16(&bytes, o + 12),
                special: bytes[o + 14] as i8,
                args: [
                    bytes[o + 15] as i8,
                    bytes[o + 16] as i8,
                    bytes[o + 17] as i8,
                    bytes[o + 18] as i8,
                    bytes[o + 19] as i8,
                ],
                props: Vec::new(),
            }
        } else {
            IntThing {
                thingid: 0,
                x: from_map_unit(read_i16(&bytes, o)),
                y: from_map_unit(read_i16(&bytes, o + 2)),
                z: 0,
                angle: read_i16(&bytes, o + 4),
                kind: read_i16(&bytes, o + 6),
                flags: read_i16(&bytes, o + 8),
                special: 0,
                args: [0; 5],
                props: Vec::new(),
            }
        };
        level.things.push(thing);
    }
    Ok(())
}

fn load_vertices(
    wad: &mut WadReader,
    map_lump: i32,
    level: &mut Level,
) -> Result<(), ProcessorError> {
    let bytes = read_lump(wad, map_lump, "VERTEXES", true)?;
    check_record("VERTEXES", &bytes, sizes::MAP_VERTEX)?;
    let count = bytes.len() / sizes::MAP_VERTEX;
    level.vertices.reserve(count);
    for i in 0..count {
        let o = i * sizes::MAP_VERTEX;
        level.vertices.push(crate::level::WideVertex {
            x: from_map_unit(read_i16(&bytes, o)),
            y: from_map_unit(read_i16(&bytes, o + 2)),
            index: 0,
        });
    }
    Ok(())
}

fn load_lines(
    wad: &mut WadReader,
    map_lump: i32,
    extended: bool,
    level: &mut Level,
) -> Result<(), ProcessorError> {
    let bytes = read_lump(wad, map_lump, "LINEDEFS", true)?;
    let record = if extended {
        sizes::MAP_LINEDEF2
    } else {
        sizes::MAP_LINEDEF
    };
    check_record("LINEDEFS", &bytes, record)?;

    let count = bytes.len() / record;
    level.lines.reserve(count);
    for i in 0..count {
        let o = i * record;
        let line = if extended {
            IntLineDef {
                v1: read_u16(&bytes, o) as u32,
                v2: read_u16(&bytes, o + 2) as u32,
                flags: read_i16(&bytes, o + 4) as i32,
                special: bytes[o + 6] as i32,
                args: [
                    bytes[o + 7] as i32,
                    bytes[o + 8] as i32,
                    bytes[o + 9] as i32,
                    bytes[o + 10] as i32,
                    bytes[o + 11] as i32,
                ],
                sidenum: [
                    widen_sidenum(read_u16(&bytes, o + 12)),
                    widen_sidenum(read_u16(&bytes, o + 14)),
                ],
                props: Vec::new(),
            }
        } else {
            // Doom format: stash `special` and `tag` in args[0..1] so they aren't lost
            // (matches processor.cpp:204-207).
            let mut line = IntLineDef {
                v1: read_u16(&bytes, o) as u32,
                v2: read_u16(&bytes, o + 2) as u32,
                flags: read_i16(&bytes, o + 4) as i32,
                special: 0,
                args: [0; 5],
                sidenum: [
                    widen_sidenum(read_u16(&bytes, o + 10)),
                    widen_sidenum(read_u16(&bytes, o + 12)),
                ],
                props: Vec::new(),
            };
            line.args[0] = read_i16(&bytes, o + 6) as i32;
            line.args[1] = read_i16(&bytes, o + 8) as i32;
            line
        };
        level.lines.push(line);
    }
    Ok(())
}

fn load_sides(
    wad: &mut WadReader,
    map_lump: i32,
    level: &mut Level,
) -> Result<(), ProcessorError> {
    let bytes = read_lump(wad, map_lump, "SIDEDEFS", true)?;
    check_record("SIDEDEFS", &bytes, sizes::MAP_SIDEDEF)?;
    let count = bytes.len() / sizes::MAP_SIDEDEF;
    level.sides.reserve(count);
    for i in 0..count {
        let o = i * sizes::MAP_SIDEDEF;
        let mut side = IntSideDef::default();
        // Note: textureoffset/rowoffset are copied straight through in C++ without a
        // LittleShort call (processor.cpp:237-238). On Apple/x86 that's a no-op anyway,
        // but on big-endian hosts the C++ would also miss the swap. We swap here so the
        // in-memory representation matches host-byte-order consistently; on little-endian
        // hosts the bytes are identical to the C++ result.
        side.texture_offset = read_i16(&bytes, o);
        side.row_offset = read_i16(&bytes, o + 2);
        side.top_texture.copy_from_slice(&bytes[o + 4..o + 12]);
        side.bottom_texture.copy_from_slice(&bytes[o + 12..o + 20]);
        side.mid_texture.copy_from_slice(&bytes[o + 20..o + 28]);
        let sec = read_u16(&bytes, o + 28);
        side.sector = if sec == NO_MAP_INDEX { NO_INDEX } else { sec as u32 };
        level.sides.push(side);
    }
    Ok(())
}

fn load_sectors(
    wad: &mut WadReader,
    map_lump: i32,
    level: &mut Level,
) -> Result<(), ProcessorError> {
    let bytes = read_lump(wad, map_lump, "SECTORS", true)?;
    check_record("SECTORS", &bytes, sizes::MAP_SECTOR)?;
    let count = bytes.len() / sizes::MAP_SECTOR;
    level.sectors.reserve(count);
    for i in 0..count {
        let o = i * sizes::MAP_SECTOR;
        let mut data = MapSector::default();
        data.floor_height = read_i16(&bytes, o);
        data.ceiling_height = read_i16(&bytes, o + 2);
        data.floor_pic.copy_from_slice(&bytes[o + 4..o + 12]);
        data.ceiling_pic.copy_from_slice(&bytes[o + 12..o + 20]);
        data.light_level = read_i16(&bytes, o + 20);
        data.special = read_i16(&bytes, o + 22);
        data.tag = read_i16(&bytes, o + 24);
        level.sectors.push(IntSector {
            data,
            props: Vec::new(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widen_sidenum_handles_sentinel() {
        assert_eq!(widen_sidenum(0), 0);
        assert_eq!(widen_sidenum(42), 42);
        assert_eq!(widen_sidenum(NO_MAP_INDEX), NO_INDEX);
    }
}
