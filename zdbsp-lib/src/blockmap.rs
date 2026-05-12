// ABOUTME: Port of blockmapbuilder.cpp. Builds the Doom BLOCKMAP lump: a 4-word header
// ABOUTME: followed by a per-cell offset table and packed line-list segments.

use crate::fixed::FRACBITS;
use crate::level::{Level, BLOCKBITS, BLOCKSIZE};

/// 32x32 -> 64 -> 32 scaled multiply, mirroring zdbsp.h's `Scale(a, b, c) = a*b/c`.
/// On ARM/x86_64 hosts the C++ uses the double-precision fallback; for blockmap inputs
/// the values stay within 17-18 bits so 64-bit integer math gives the same result.
#[inline]
fn scale(a: i32, b: i32, c: i32) -> i32 {
    ((a as i64) * (b as i64) / (c as i64)) as i32
}

/// Construct the BLOCKMAP lump from `level`. Returns a `Vec<u16>` ready to be written
/// to disk (each value is little-endian when serialized).
pub fn build(level: &Level) -> Vec<u16> {
    let mut blockmap: Vec<u16> = Vec::new();

    if level.num_vertices() == 0 {
        return blockmap;
    }

    // The C++ uses signed ints throughout for blockmap math. Map coords fit comfortably
    // in i32 after the >>FRACBITS shift; we keep i32 to match.
    let minx = level.min_x >> FRACBITS;
    let miny = level.min_y >> FRACBITS;
    let maxx = level.max_x >> FRACBITS;
    let maxy = level.max_y >> FRACBITS;

    let bmapwidth = ((maxx - minx) >> BLOCKBITS) + 1;
    let bmapheight = ((maxy - miny) >> BLOCKBITS) + 1;
    let area = (bmapwidth as usize) * (bmapheight as usize);

    blockmap.push(minx as u16);
    blockmap.push(miny as u16);
    blockmap.push(bmapwidth as u16);
    blockmap.push(bmapheight as u16);

    let mut block_lists: Vec<Vec<u16>> = vec![Vec::new(); area];

    for line_idx in 0..level.lines.len() {
        let line_word = line_idx as u16;
        let v1 = &level.vertices[level.lines[line_idx].v1 as usize];
        let v2 = &level.vertices[level.lines[line_idx].v2 as usize];

        let x1 = v1.x >> FRACBITS;
        let y1 = v1.y >> FRACBITS;
        let x2 = v2.x >> FRACBITS;
        let y2 = v2.y >> FRACBITS;
        let dx = x2 - x1;
        let dy = y2 - y1;
        let mut bx = (x1 - minx) >> BLOCKBITS;
        let mut by = (y1 - miny) >> BLOCKBITS;
        let bx2 = (x2 - minx) >> BLOCKBITS;
        let by2 = (y2 - miny) >> BLOCKBITS;

        // Mirror the four cases from blockmapbuilder.cpp:229. The trick in C++ is pointer
        // arithmetic over BlockLists; we translate to (bx, by) index walks.
        if bx == bx2 && by == by2 {
            // Single block.
            block_lists[(bx + by * bmapwidth) as usize].push(line_word);
        } else if by == by2 {
            // Horizontal line.
            let (lo, hi) = if bx > bx2 { (bx2, bx) } else { (bx, bx2) };
            for cx in lo..=hi {
                block_lists[(cx + by * bmapwidth) as usize].push(line_word);
            }
        } else if bx == bx2 {
            // Vertical line.
            let (lo, hi) = if by > by2 { (by2, by) } else { (by, by2) };
            for cy in lo..=hi {
                block_lists[(bx + cy * bmapwidth) as usize].push(line_word);
            }
        } else {
            // Diagonal line. The C++ loop walks block-by-block along the major axis,
            // pushing every block the line crosses. Preserved here verbatim.
            let xchange: i32 = if dx < 0 { -1 } else { 1 };
            let ychange: i32 = if dy < 0 { -1 } else { 1 };
            let mut adx = dx.abs();
            let ady = dy.abs();

            if adx == ady {
                // 45 degrees: tie-break which axis is "ahead" using the sub-block offsets.
                let mut xb = (x1 - minx) & (BLOCKSIZE - 1);
                let mut yb = (y1 - miny) & (BLOCKSIZE - 1);
                if dx < 0 {
                    xb = BLOCKSIZE - xb;
                }
                if dy < 0 {
                    yb = BLOCKSIZE - yb;
                }
                if xb < yb {
                    adx -= 1;
                }
            }

            if adx >= ady {
                // X-major.
                let yadd = if dy < 0 { -1 } else { BLOCKSIZE };
                loop {
                    let stop =
                        (scale((by << BLOCKBITS) + yadd - (y1 - miny), dx, dy) + (x1 - minx))
                            >> BLOCKBITS;
                    while bx != stop {
                        block_lists[(bx + by * bmapwidth) as usize].push(line_word);
                        bx += xchange;
                    }
                    block_lists[(bx + by * bmapwidth) as usize].push(line_word);
                    by += ychange;
                    if by == by2 {
                        break;
                    }
                }
                while bx != bx2 {
                    block_lists[(bx + by * bmapwidth) as usize].push(line_word);
                    bx += xchange;
                }
                block_lists[(bx + by * bmapwidth) as usize].push(line_word);
            } else {
                // Y-major.
                let xadd = if dx < 0 { -1 } else { BLOCKSIZE };
                loop {
                    let stop =
                        (scale((bx << BLOCKBITS) + xadd - (x1 - minx), dy, dx) + (y1 - miny))
                            >> BLOCKBITS;
                    while by != stop {
                        block_lists[(bx + by * bmapwidth) as usize].push(line_word);
                        by += ychange;
                    }
                    block_lists[(bx + by * bmapwidth) as usize].push(line_word);
                    bx += xchange;
                    if bx == bx2 {
                        break;
                    }
                }
                while by != by2 {
                    block_lists[(bx + by * bmapwidth) as usize].push(line_word);
                    by += ychange;
                }
                block_lists[(bx + by * bmapwidth) as usize].push(line_word);
            }
        }
    }

    // Reserve the offset table (zero-initialized) and then fill via hash-deduped packing.
    blockmap.resize(4 + area, 0);
    create_packed_blockmap(&mut blockmap, &block_lists, area);
    blockmap
}

/// Hash a single block's line list. Reproduces blockmapbuilder.cpp:353-362 verbatim,
/// including the signed-overflow wrap and the final `& 0x7fffffff` mask.
fn block_hash(block: &[u16]) -> u32 {
    let mut hash: i32 = 0;
    for &w in block {
        hash = hash.wrapping_mul(12235).wrapping_add(w as i32);
    }
    (hash & 0x7fffffff) as u32
}

fn create_packed_blockmap(blockmap: &mut Vec<u16>, blocks: &[Vec<u16>], area: usize) {
    const NUM_BUCKETS: usize = 4096;
    const NONE: u16 = 0xffff;
    let mut buckets = [NONE; NUM_BUCKETS];
    let mut hashes = vec![NONE; area];

    for i in 0..area {
        let block = &blocks[i];
        let bucket = (block_hash(block) as usize) % NUM_BUCKETS;
        let mut hashblock = buckets[bucket];
        while hashblock != NONE {
            if blocks_equal(block, &blocks[hashblock as usize]) {
                break;
            }
            hashblock = hashes[hashblock as usize];
        }
        if hashblock != NONE {
            blockmap[4 + i] = blockmap[4 + hashblock as usize];
        } else {
            hashes[i] = buckets[bucket];
            buckets[bucket] = i as u16;
            blockmap[4 + i] = blockmap.len() as u16;
            blockmap.push(0);
            blockmap.extend_from_slice(block);
            blockmap.push(NONE);
        }
    }
}

fn blocks_equal(a: &[u16], b: &[u16]) -> bool {
    a == b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_hash_matches_cpp_zero() {
        assert_eq!(block_hash(&[]), 0);
        assert_eq!(block_hash(&[0]), 0);
    }

    #[test]
    fn block_hash_is_deterministic() {
        // Compare against the C++ formula `hash = hash * 12235 + ar[i]` applied directly.
        let mut expected: i32 = 0;
        for w in [1u16, 2, 3, 4, 5] {
            expected = expected.wrapping_mul(12235).wrapping_add(w as i32);
        }
        let expected = (expected & 0x7fffffff) as u32;
        assert_eq!(block_hash(&[1, 2, 3, 4, 5]), expected);
    }

    #[test]
    fn empty_level_yields_empty_blockmap() {
        let level = Level::default();
        let bm = build(&level);
        assert!(bm.is_empty());
    }
}
