// ABOUTME: WAD reader/writer. Port of wad.cpp/wad.h. Doom-format WAD file I/O,
// ABOUTME: lump directory access, and map/GL-node detection. Little-endian on disk.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const LUMP_NAME_LEN: usize = 8;

const MAP_LUMP_NAMES: [&[u8]; 12] = [
    b"THINGS", b"LINEDEFS", b"SIDEDEFS", b"VERTEXES", b"SEGS", b"SSECTORS", b"NODES", b"SECTORS",
    b"REJECT", b"BLOCKMAP", b"BEHAVIOR", b"SCRIPTS",
];

const MAP_LUMP_REQUIRED: [bool; 12] = [
    true,  // THINGS
    true,  // LINEDEFS
    true,  // SIDEDEFS
    true,  // VERTEXES
    false, // SEGS
    false, // SSECTORS
    false, // NODES
    true,  // SECTORS
    false, // REJECT
    false, // BLOCKMAP
    false, // BEHAVIOR
    false, // SCRIPTS
];

const GL_LUMP_NAMES: [&[u8]; 5] = [
    b"GL_VERT", b"GL_SEGS", b"GL_SSECT", b"GL_NODES", b"GL_PVS",
];

#[derive(Debug, thiserror::Error)]
pub enum WadError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("input file is not a wad")]
    NotAWad,
}

/// One entry in a WAD's lump directory. `file_pos` and `size` are kept in host byte order;
/// little-endian conversion happens at the disk boundary.
#[derive(Debug, Clone, Copy)]
pub struct WadLump {
    pub file_pos: i32,
    pub size: i32,
    pub name: [u8; LUMP_NAME_LEN],
}

impl WadLump {
    /// Return the lump name as a `&str`, trimming at the first NUL (or end of the 8 bytes).
    /// Names that contain non-UTF-8 bytes fall back to a lossy view.
    pub fn name_str(&self) -> std::borrow::Cow<'_, str> {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(LUMP_NAME_LEN);
        String::from_utf8_lossy(&self.name[..end])
    }
}

pub struct WadReader {
    file: File,
    is_iwad: bool,
    lumps: Vec<WadLump>,
}

impl WadReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WadError> {
        let mut file = File::open(path)?;

        let mut header = [0u8; 12];
        file.read_exact(&mut header)?;

        // Port note: the C++ magic check uses && between every byte comparison
        // (wad.cpp:73-81), so it only rejects a file when *all four* bytes are wrong.
        // Preserved verbatim for behavioral parity; real WADs are always PWAD/IWAD.
        let magic = &header[0..4];
        if magic[0] != b'P'
            && magic[0] != b'I'
            && magic[1] != b'W'
            && magic[2] != b'A'
            && magic[3] != b'D'
        {
            return Err(WadError::NotAWad);
        }
        let is_iwad = magic[0] == b'I';

        let num_lumps = i32::from_le_bytes(header[4..8].try_into().unwrap());
        let directory = i32::from_le_bytes(header[8..12].try_into().unwrap());

        file.seek(SeekFrom::Start(directory as u64))?;
        let entry_size = 16usize; // 4 + 4 + 8
        let mut dir_bytes = vec![0u8; (num_lumps as usize).saturating_mul(entry_size)];
        file.read_exact(&mut dir_bytes)?;

        let mut lumps = Vec::with_capacity(num_lumps.max(0) as usize);
        for chunk in dir_bytes.chunks_exact(entry_size) {
            let file_pos = i32::from_le_bytes(chunk[0..4].try_into().unwrap());
            let size = i32::from_le_bytes(chunk[4..8].try_into().unwrap());
            let mut name = [0u8; LUMP_NAME_LEN];
            name.copy_from_slice(&chunk[8..16]);
            lumps.push(WadLump { file_pos, size, name });
        }

        Ok(Self { file, is_iwad, lumps })
    }

    pub fn is_iwad(&self) -> bool {
        self.is_iwad
    }

    pub fn num_lumps(&self) -> i32 {
        self.lumps.len() as i32
    }

    pub fn lump(&self, index: i32) -> Option<&WadLump> {
        usize::try_from(index).ok().and_then(|i| self.lumps.get(i))
    }

    pub fn lump_name(&self, index: i32) -> std::borrow::Cow<'_, str> {
        self.lump(index).map(|l| l.name_str()).unwrap_or(std::borrow::Cow::Borrowed(""))
    }

    /// Find the first lump whose name matches `name` case-insensitively, starting at `start`.
    /// Returns -1 (matching the C++ contract) when no match is found.
    pub fn find_lump(&self, name: &str, start: i32) -> i32 {
        let start = start.max(0) as usize;
        for (i, lump) in self.lumps.iter().enumerate().skip(start) {
            if names_eq_ci(&lump.name, name.as_bytes()) {
                return i as i32;
            }
        }
        -1
    }

    /// Find a specific named map lump (THINGS, LINEDEFS, ...) that belongs to the map
    /// header at `map`. Matches the C++ algorithm in `FindMapLump`.
    pub fn find_map_lump(&self, name: &str, map: i32) -> i32 {
        let target = match MAP_LUMP_NAMES.iter().position(|n| names_eq_ci(n, name.as_bytes())) {
            Some(i) => i,
            None => return -1,
        };
        let start = (map + 1) as usize;
        let mut k = 0usize;
        for j in 0..12 {
            let idx = start + k;
            let Some(lump) = self.lumps.get(idx) else {
                return -1;
            };
            if names_eq_ci(&lump.name, MAP_LUMP_NAMES[j]) {
                if target == j {
                    return idx as i32;
                }
                k += 1;
            }
        }
        -1
    }

    pub fn is_udmf(&self, index: i32) -> bool {
        let next = index + 1;
        match self.lump(next) {
            Some(l) => names_eq_ci(&l.name, b"TEXTMAP"),
            None => false,
        }
    }

    pub fn is_map(&self, index: i32) -> bool {
        if self.is_udmf(index) {
            return true;
        }
        let start = (index + 1) as usize;
        let mut k = 0usize;
        for i in 0..12 {
            let Some(lump) = self.lumps.get(start + k) else {
                // Hit end of directory before all required lumps were found.
                return !MAP_LUMP_REQUIRED[i..].iter().any(|&r| r);
            };
            if !names_eq_ci(&lump.name, MAP_LUMP_NAMES[i]) {
                if MAP_LUMP_REQUIRED[i] {
                    return false;
                }
            } else {
                k += 1;
            }
        }
        true
    }

    pub fn is_gl_nodes(&self, index: i32) -> bool {
        let start = match usize::try_from(index) {
            Ok(s) => s,
            Err(_) => return false,
        };
        if start + 4 >= self.lumps.len() {
            return false;
        }
        let header = &self.lumps[start].name;
        if header[0] != b'G' || header[1] != b'L' || header[2] != b'_' {
            return false;
        }
        for i in 0..4 {
            if !names_eq_ci(&self.lumps[start + 1 + i].name, GL_LUMP_NAMES[i]) {
                return false;
            }
        }
        true
    }

    pub fn skip_gl_nodes(&self, index: i32) -> i32 {
        let mut idx = index + 1;
        for i in 0..5 {
            let Some(lump) = self.lump(idx) else { break };
            if !names_eq_ci(&lump.name, GL_LUMP_NAMES[i]) {
                break;
            }
            idx += 1;
        }
        idx
    }

    pub fn find_gl_lump(&self, name: &str, gl_header: i32) -> i32 {
        let start = (gl_header + 1) as usize;
        let target = (0..5).find(|&i| {
            self.lumps
                .get(start + i)
                .map(|l| names_eq_ci(&l.name, name.as_bytes()))
                .unwrap_or(false)
        });
        let Some(target) = target else { return -1 };
        let mut k = 0usize;
        for j in 0..5 {
            let Some(lump) = self.lumps.get(start + k) else {
                return -1;
            };
            if names_eq_ci(&lump.name, GL_LUMP_NAMES[j]) {
                if target == j {
                    return (start + k) as i32;
                }
                k += 1;
            }
        }
        -1
    }

    pub fn map_has_behavior(&self, map: i32) -> bool {
        self.find_map_lump("BEHAVIOR", map) != -1
    }

    /// Find the next map header at or after `start`. Returns -1 if none.
    /// Pass -1 to start from lump 0.
    pub fn next_map(&self, start: i32) -> i32 {
        let mut idx = if start < 0 { 0 } else { start + 1 };
        while (idx as usize) < self.lumps.len() {
            if self.is_map(idx) {
                return idx;
            }
            idx += 1;
        }
        -1
    }

    /// Returns the first lump index *after* the map at `map`.
    pub fn lump_after_map(&self, map: i32) -> i32 {
        if self.is_udmf(map) {
            // UDMF: skip past TEXTMAP, then scan for ENDMAP.
            let mut i = (map + 2) as usize;
            while i < self.lumps.len() && !names_eq_ci(&self.lumps[i].name, b"ENDMAP") {
                i += 1;
            }
            return (i + 1) as i32;
        }

        let start = (map + 1) as usize;
        let mut k = 0usize;
        for j in 0..12 {
            let Some(lump) = self.lumps.get(start + k) else { break };
            if !names_eq_ci(&lump.name, MAP_LUMP_NAMES[j]) {
                if MAP_LUMP_REQUIRED[j] {
                    break;
                }
            } else {
                k += 1;
            }
        }
        (start + k) as i32
    }

    /// Read the entire contents of a lump into a fresh `Vec<u8>`.
    pub fn read_lump(&mut self, index: i32) -> io::Result<Vec<u8>> {
        let (pos, size) = match self.lump(index) {
            Some(l) => (l.file_pos, l.size),
            None => return Ok(Vec::new()),
        };
        let len = size.max(0) as usize;
        self.file.seek(SeekFrom::Start(pos as u64))?;
        let mut buf = vec![0u8; len];
        self.file.read_exact(&mut buf)?;
        Ok(buf)
    }
}

/// Build a lump directory while streaming lump data straight to disk. Mirrors the C++
/// `FWadWriter` byte-for-byte: header magic written up front, eight placeholder bytes for
/// `num_lumps` + `directory`, lump data appended sequentially, directory at EOF, then a
/// seek-back-and-patch of those eight bytes.
pub struct WadWriter {
    file: File,
    lumps: Vec<WadLump>,
    closed: bool,
}

impl WadWriter {
    pub fn create(path: impl AsRef<Path>, iwad: bool) -> io::Result<Self> {
        let mut file = File::create(path)?;
        let magic = if iwad { b'I' } else { b'P' };
        // C++ writes a 12-byte WadHeader where only the magic is initialized; the other
        // 8 bytes are uninitialized stack memory that gets overwritten in Close(). We
        // explicitly zero them so the in-progress file is deterministic (the final file
        // is identical either way once Close() runs).
        let header = [magic, b'W', b'A', b'D', 0, 0, 0, 0, 0, 0, 0, 0];
        file.write_all(&header)?;
        Ok(Self { file, lumps: Vec::new(), closed: false })
    }

    fn pos(&mut self) -> io::Result<i32> {
        Ok(self.file.stream_position()? as i32)
    }

    pub fn create_label(&mut self, name: &str) -> io::Result<()> {
        let file_pos = self.pos()?;
        self.lumps.push(WadLump { file_pos, size: 0, name: pack_name(name) });
        Ok(())
    }

    pub fn write_lump(&mut self, name: &str, data: &[u8]) -> io::Result<()> {
        let file_pos = self.pos()?;
        self.lumps.push(WadLump { file_pos, size: data.len() as i32, name: pack_name(name) });
        self.file.write_all(data)
    }

    pub fn copy_lump(&mut self, reader: &mut WadReader, lump: i32) -> io::Result<()> {
        let name = reader.lump(lump).map(|l| l.name).unwrap_or([0u8; LUMP_NAME_LEN]);
        let data = reader.read_lump(lump)?;
        if data.is_empty() && reader.lump(lump).is_none() {
            return Ok(());
        }
        let file_pos = self.pos()?;
        self.lumps.push(WadLump { file_pos, size: data.len() as i32, name });
        self.file.write_all(&data)
    }

    pub fn start_lump(&mut self, name: &str) -> io::Result<()> {
        self.create_label(name)
    }

    pub fn add_to_lump(&mut self, data: &[u8]) -> io::Result<()> {
        self.file.write_all(data)?;
        if let Some(last) = self.lumps.last_mut() {
            last.size += data.len() as i32;
        }
        Ok(())
    }

    pub fn write_u8(&mut self, val: u8) -> io::Result<()> {
        self.add_to_lump(&[val])
    }

    pub fn write_u16(&mut self, val: u16) -> io::Result<()> {
        self.add_to_lump(&val.to_le_bytes())
    }

    pub fn write_i16(&mut self, val: i16) -> io::Result<()> {
        self.add_to_lump(&val.to_le_bytes())
    }

    pub fn write_u32(&mut self, val: u32) -> io::Result<()> {
        self.add_to_lump(&val.to_le_bytes())
    }

    pub fn write_i32(&mut self, val: i32) -> io::Result<()> {
        self.add_to_lump(&val.to_le_bytes())
    }

    pub fn close(&mut self) -> io::Result<()> {
        if self.closed {
            return Ok(());
        }
        let dir_pos = self.pos()?;
        for lump in &self.lumps {
            self.file.write_all(&lump.file_pos.to_le_bytes())?;
            self.file.write_all(&lump.size.to_le_bytes())?;
            self.file.write_all(&lump.name)?;
        }
        let header_patch = {
            let mut h = [0u8; 8];
            h[0..4].copy_from_slice(&(self.lumps.len() as i32).to_le_bytes());
            h[4..8].copy_from_slice(&dir_pos.to_le_bytes());
            h
        };
        self.file.seek(SeekFrom::Start(4))?;
        self.file.write_all(&header_patch)?;
        self.file.flush()?;
        self.closed = true;
        Ok(())
    }
}

impl Drop for WadWriter {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn pack_name(name: &str) -> [u8; LUMP_NAME_LEN] {
    // Match `strncpy(dst, src, 8)`: copy up to 8 bytes, zero-pad the rest.
    let mut out = [0u8; LUMP_NAME_LEN];
    let src = name.as_bytes();
    let n = src.len().min(LUMP_NAME_LEN);
    out[..n].copy_from_slice(&src[..n]);
    out
}

/// Case-insensitive comparison of two lump-name byte slices, matching `strnicmp(a, b, 8)`:
/// comparison stops at the first NUL in either operand or after 8 bytes, ASCII case is
/// folded, and shorter operands are treated as if zero-padded.
fn names_eq_ci(a: &[u8], b: &[u8]) -> bool {
    for i in 0..LUMP_NAME_LEN {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x == 0 && y == 0 {
            return true;
        }
        if x.eq_ignore_ascii_case(&y) {
            continue;
        }
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_eq_ci_basic() {
        let name: [u8; 8] = *b"THINGS\0\0";
        assert!(names_eq_ci(&name, b"THINGS"));
        assert!(names_eq_ci(&name, b"things"));
        assert!(names_eq_ci(&name, b"Things"));
        assert!(!names_eq_ci(&name, b"THINGZ"));
        assert!(!names_eq_ci(&name, b"THING"));
        let full: [u8; 8] = *b"BEHAVIOR";
        assert!(names_eq_ci(&full, b"BEHAVIOR"));
        // strnicmp only inspects 8 bytes; anything beyond is ignored, matching C++.
        assert!(names_eq_ci(&full, b"BEHAVIOR1"));
    }

    #[test]
    fn pack_name_zero_pads() {
        assert_eq!(pack_name("THINGS"), *b"THINGS\0\0");
        assert_eq!(pack_name(""), [0u8; 8]);
        assert_eq!(pack_name("LONGENOUGH"), *b"LONGENOU");
    }

    #[test]
    fn write_then_read_minimal_wad() {
        let dir = std::env::temp_dir().join("zdbsp-rust-wad-min");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("min.wad");

        {
            let mut w = WadWriter::create(&path, false).unwrap();
            w.write_lump("HELLO", b"hi there").unwrap();
            w.create_label("MAP01").unwrap();
            w.write_lump("THINGS", &[]).unwrap();
            w.close().unwrap();
        }

        let mut r = WadReader::open(&path).unwrap();
        assert!(!r.is_iwad());
        assert_eq!(r.num_lumps(), 3);
        assert_eq!(r.lump_name(0), "HELLO");
        assert_eq!(r.read_lump(0).unwrap(), b"hi there");
        assert_eq!(r.lump_name(1), "MAP01");
        assert_eq!(r.lump_name(2), "THINGS");
        assert_eq!(r.find_lump("MAP01", 0), 1);
        assert_eq!(r.find_lump("nope", 0), -1);
    }
}
