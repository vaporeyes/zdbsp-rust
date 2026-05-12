// ABOUTME: Port of nodebuild_utility.cpp. FVertexMap operations, MakeSegsFromSides,
// ABOUTME: CreateSeg, GroupSegPlanes, FindUsedVertices, MarkLoop, polyobject helpers,
// ABOUTME: InterceptVector, SplitSeg, SetNodeFromSeg.

use crate::fixed::{point_to_angle, Angle, Fixed, FRACBITS};
use crate::level::{
    Level, WideVertex, NO_INDEX, PO_ANCHOR_TYPE, PO_HEX_ANCHOR_TYPE, PO_HEX_SPAWNCRUSH_TYPE,
    PO_HEX_SPAWN_TYPE, PO_SPAWNCRUSH_TYPE, PO_SPAWNHURT_TYPE, PO_SPAWN_TYPE,
};
use crate::workdata::Node;

use super::{
    point_on_side, PolyStart, PrivSeg, PrivVert, SimpleLine, USegPtr, NO_NODE_INDEX,
    VERTEX_EPSILON, VMAP_BLOCK_SHIFT, VMAP_BLOCK_SIZE,
};

const FIXED_MAX: Fixed = Fixed::MAX;
const FIXED_MIN: Fixed = Fixed::MIN;

const ANGLE_180: u32 = 1u32 << 31;
const ANGLE_MAX: u32 = u32::MAX;

const PO_LINE_START: i32 = 1;
const PO_LINE_EXPLICIT: i32 = 5;

const BOX_TOP: usize = 0;
const BOX_BOTTOM: usize = 1;
const BOX_LEFT: usize = 2;
const BOX_RIGHT: usize = 3;

impl<'a> super::NodeBuilder<'a> {
    /// Allocate the vertex spatial hash. Direct port of `FVertexMap::FVertexMap`.
    pub fn init_vertex_map(&mut self, minx: Fixed, miny: Fixed, maxx: Fixed, maxy: Fixed) {
        // The C++ uses doubles for the width/height computation to avoid overflow on
        // very large maps where `maxx - minx + 1` exceeds Fixed range.
        let blocks_wide = (((maxx as f64) - (minx as f64) + 1.0
            + (VMAP_BLOCK_SIZE - 1) as f64)
            / (VMAP_BLOCK_SIZE as f64)) as i32;
        let blocks_tall = (((maxy as f64) - (miny as f64) + 1.0
            + (VMAP_BLOCK_SIZE - 1) as f64)
            / (VMAP_BLOCK_SIZE as f64)) as i32;
        self.vmap_min_x = minx;
        self.vmap_min_y = miny;
        self.vmap_blocks_wide = blocks_wide;
        self.vmap_blocks_tall = blocks_tall;
        self.vmap_max_x = minx + blocks_wide * (VMAP_BLOCK_SIZE as i32) - 1;
        self.vmap_max_y = miny + blocks_tall * (VMAP_BLOCK_SIZE as i32) - 1;
        let cells = (blocks_wide as usize) * (blocks_tall as usize);
        self.vmap_grid = vec![Vec::new(); cells];
    }

    #[inline]
    fn vmap_block(&self, x: Fixed, y: Fixed) -> usize {
        debug_assert!(x >= self.vmap_min_x);
        debug_assert!(y >= self.vmap_min_y);
        debug_assert!(x <= self.vmap_max_x);
        debug_assert!(y <= self.vmap_max_y);
        let bx = ((x.wrapping_sub(self.vmap_min_x)) as u32) >> VMAP_BLOCK_SHIFT;
        let by = ((y.wrapping_sub(self.vmap_min_y)) as u32) >> VMAP_BLOCK_SHIFT;
        (bx as usize) + (by as usize) * (self.vmap_blocks_wide as usize)
    }

    /// Look up an existing vertex with the exact same coords; if none, insert one.
    pub fn select_vertex_exact(&mut self, mut vert: PrivVert) -> i32 {
        let blk = self.vmap_block(vert.x, vert.y);
        for &idx in &self.vmap_grid[blk] {
            let v = &self.vertices[idx as usize];
            if v.x == vert.x && v.y == vert.y {
                return idx;
            }
        }
        self.insert_vertex(&mut vert)
    }

    /// Like `select_vertex_exact` but within VERTEX_EPSILON.
    pub fn select_vertex_close(&mut self, mut vert: PrivVert) -> i32 {
        let blk = self.vmap_block(vert.x, vert.y);
        for &idx in &self.vmap_grid[blk] {
            let v = &self.vertices[idx as usize];
            if (v.x - vert.x).abs() < VERTEX_EPSILON && (v.y - vert.y).abs() < VERTEX_EPSILON {
                return idx;
            }
        }
        self.insert_vertex(&mut vert)
    }

    fn insert_vertex(&mut self, vert: &mut PrivVert) -> i32 {
        vert.segs = NO_NODE_INDEX;
        vert.segs2 = NO_NODE_INDEX;
        let vertnum = self.vertices.len() as i32;
        self.vertices.push(*vert);

        // If the vertex sits near a block boundary, drop it into every block it might
        // be found from by `select_vertex_close`.
        let minx = self.vmap_min_x.max(vert.x - VERTEX_EPSILON);
        let maxx = self.vmap_max_x.min(vert.x + VERTEX_EPSILON);
        let miny = self.vmap_min_y.max(vert.y - VERTEX_EPSILON);
        let maxy = self.vmap_max_y.min(vert.y + VERTEX_EPSILON);
        let blk = [
            self.vmap_block(minx, miny),
            self.vmap_block(maxx, miny),
            self.vmap_block(minx, maxy),
            self.vmap_block(maxx, maxy),
        ];
        // Snapshot pre-push lengths; only push into blocks that haven't already grown
        // (i.e. the four corner blocks may coincide).
        let prev = [
            self.vmap_grid[blk[0]].len(),
            self.vmap_grid[blk[1]].len(),
            self.vmap_grid[blk[2]].len(),
            self.vmap_grid[blk[3]].len(),
        ];
        for i in 0..4 {
            if self.vmap_grid[blk[i]].len() == prev[i] {
                self.vmap_grid[blk[i]].push(vertnum);
            }
        }
        vertnum
    }

    /// Walk every line and copy its referenced vertices through `select_vertex_exact`,
    /// rewriting line `v1`/`v2` to the new vertex indices. Sets `initial_vertices` and
    /// `level.num_org_verts` once done.
    pub fn find_used_vertices(&mut self, oldverts: &[WideVertex]) {
        let max = oldverts.len();
        let mut map: Vec<i32> = vec![-1; max];

        for i in 0..self.level.num_lines() {
            let v1 = self.level.lines[i].v1 as usize;
            let v2 = self.level.lines[i].v2 as usize;

            if map[v1] == -1 {
                let nv = PrivVert {
                    x: oldverts[v1].x,
                    y: oldverts[v1].y,
                    segs: 0,
                    segs2: 0,
                    index: oldverts[v1].index,
                };
                map[v1] = self.select_vertex_exact(nv);
            }
            if map[v2] == -1 {
                let nv = PrivVert {
                    x: oldverts[v2].x,
                    y: oldverts[v2].y,
                    segs: 0,
                    segs2: 0,
                    index: oldverts[v2].index,
                };
                map[v2] = self.select_vertex_exact(nv);
            }

            self.level.lines[i].v1 = map[v1] as u32;
            self.level.lines[i].v2 = map[v2] as u32;
        }
        self.initial_vertices = self.vertices.len();
        // No direct field for NumOrgVerts on our Level yet; tracked via initial_vertices.
    }

    /// For every sidedef, push a corresponding seg. Mirrors MakeSegsFromSides.
    pub fn make_segs_from_sides(&mut self) {
        for i in 0..self.level.num_lines() {
            if self.level.lines[i].sidenum[0] != NO_INDEX {
                self.create_seg(i as i32, 0);
            } else {
                // C++ prints a warning; the build continues regardless.
                eprintln!("Linedef {i} does not have a front side.");
            }

            if self.level.lines[i].sidenum[1] != NO_INDEX {
                let j = self.create_seg(i as i32, 1);
                if self.level.lines[i].sidenum[0] != NO_INDEX {
                    let j = j as usize;
                    self.segs[j - 1].partner = j as u32;
                    self.segs[j].partner = (j as u32) - 1;
                }
            }
        }
    }

    pub fn create_seg(&mut self, linenum: i32, sidenum: i32) -> i32 {
        let line = &self.level.lines[linenum as usize];
        let (v1, v2) = if sidenum == 0 {
            (line.v1 as i32, line.v2 as i32)
        } else {
            (line.v2 as i32, line.v1 as i32)
        };
        let sidedef = line.sidenum[sidenum as usize];
        let backside = line.sidenum[(sidenum ^ 1) as usize];
        let frontsector = self.level.sides[sidedef as usize].sector as i32;
        let backsector = if backside != NO_INDEX {
            self.level.sides[backside as usize].sector as i32
        } else {
            -1
        };

        let vx1 = self.vertices[v1 as usize];
        let vx2 = self.vertices[v2 as usize];

        let mut seg = PrivSeg {
            v1,
            v2,
            sidedef,
            linedef: linenum,
            frontsector,
            backsector,
            next: NO_NODE_INDEX,
            next_for_vert: vx1.segs,
            next_for_vert2: vx2.segs2,
            loop_num: 0,
            partner: NO_NODE_INDEX,
            stored_seg: NO_NODE_INDEX,
            angle: point_to_angle(vx2.x - vx1.x, vx2.y - vx1.y),
            offset: 0,
            plane_num: -1,
            plane_front: false,
            hash_next: NO_NODE_INDEX,
        };
        // C++ sets planenum = DWORD_MAX in CreateSeg, but the field is signed `int`.
        // We use -1, which matches the practical interpretation of "no plane assigned".
        seg.plane_num = -1;

        let segnum = self.segs.len() as i32;
        self.segs.push(seg);
        self.vertices[v1 as usize].segs = segnum as u32;
        self.vertices[v2 as usize].segs2 = segnum as u32;
        segnum
    }

    /// Group colinear segs into the same `plane_num` so SelectSplitter only checks one
    /// representative per line. Direct port of GroupSegPlanes.
    pub fn group_seg_planes(&mut self) {
        const BUCKET_BITS: u32 = 12;
        let num_buckets = 1usize << BUCKET_BITS;
        // Bucket holds the head seg index of each hash chain, or NO_NODE_INDEX.
        let mut buckets: Vec<u32> = vec![NO_NODE_INDEX; num_buckets];

        // Pre-link every seg to its successor via `next`.
        let n = self.segs.len();
        for i in 0..n {
            self.segs[i].next = (i + 1) as u32;
            self.segs[i].hash_next = NO_NODE_INDEX;
        }
        if n > 0 {
            self.segs[n - 1].next = NO_NODE_INDEX;
        }

        let mut planenum: i32 = 0;
        for i in 0..n {
            let v1 = self.segs[i].v1 as usize;
            let v2 = self.segs[i].v2 as usize;
            let x1 = self.vertices[v1].x;
            let y1 = self.vertices[v1].y;
            let x2 = self.vertices[v2].x;
            let y2 = self.vertices[v2].y;
            let mut ang = point_to_angle(x2 - x1, y2 - y1);
            if ang >= 1u32 << 31 {
                ang = ang.wrapping_add(1u32 << 31);
            }
            let bucket = (ang >> (31 - BUCKET_BITS)) as usize;

            // Walk the hash chain looking for a collinear seg.
            let mut check = buckets[bucket];
            let mut found: Option<u32> = None;
            while check != NO_NODE_INDEX {
                let cv1 = self.segs[check as usize].v1 as usize;
                let cv2 = self.segs[check as usize].v2 as usize;
                let cx1 = self.vertices[cv1].x;
                let cy1 = self.vertices[cv1].y;
                let cdx = self.vertices[cv2].x - cx1;
                let cdy = self.vertices[cv2].y - cy1;
                if point_on_side(x1, y1, cx1, cy1, cdx, cdy) == 0
                    && point_on_side(x2, y2, cx1, cy1, cdx, cdy) == 0
                {
                    found = Some(check);
                    break;
                }
                check = self.segs[check as usize].hash_next;
            }

            if let Some(check) = found {
                let plane = self.segs[check as usize].plane_num;
                self.segs[i].plane_num = plane;
                let line = self.planes[plane as usize];
                let plane_front = if line.dx != 0 {
                    (line.dx > 0 && x2 > x1) || (line.dx < 0 && x2 < x1)
                } else {
                    (line.dy > 0 && y2 > y1) || (line.dy < 0 && y2 < y1)
                };
                self.segs[i].plane_front = plane_front;
            } else {
                self.segs[i].hash_next = buckets[bucket];
                buckets[bucket] = i as u32;
                self.segs[i].plane_num = planenum;
                self.segs[i].plane_front = true;
                planenum += 1;
                self.planes.push(SimpleLine {
                    x: x1,
                    y: y1,
                    dx: x2 - x1,
                    dy: y2 - y1,
                });
            }
        }

        // PlaneChecked is a bitset; reserve enough bytes.
        let bytes = (planenum as usize + 7) / 8;
        self.plane_checked.resize(bytes, 0);
    }

    /// `FindPolyContainers` from the C++. Identifies the loop of segs surrounding each
    /// polyobject's origin and stamps them with a per-loop `loop_num`.
    pub fn find_poly_containers(&mut self) {
        let mut loop_num = 1i32;
        let spots = std::mem::take(&mut self.poly_starts);
        let anchors = std::mem::take(&mut self.poly_anchors);

        for spot in &spots {
            let mut bbox = [0i32; 4];
            if !self.get_poly_extents(spot.polynum, &mut bbox) {
                continue;
            }
            let Some(anchor) = anchors.iter().find(|a| a.polynum == spot.polynum) else {
                continue;
            };

            let mid_x = bbox[BOX_LEFT] + (bbox[BOX_RIGHT] - bbox[BOX_LEFT]) / 2;
            let mid_y = bbox[BOX_BOTTOM] + (bbox[BOX_TOP] - bbox[BOX_BOTTOM]) / 2;
            let center_x = mid_x - anchor.x + spot.x;
            let center_y = mid_y - anchor.y + spot.y;

            let mut closest_dist: Fixed = FIXED_MAX;
            let mut closest_seg: u32 = 0;

            for j in 0..self.segs.len() {
                let v1 = self.vertices[self.segs[j].v1 as usize];
                let v2 = self.vertices[self.segs[j].v2 as usize];
                let dy = v2.y - v1.y;
                if dy == 0 {
                    continue;
                }
                if (v1.y < center_y && v2.y < center_y) || (v1.y > center_y && v2.y > center_y) {
                    continue;
                }
                let dx = v2.x - v1.x;
                if point_on_side(center_x, center_y, v1.x, v1.y, dx, dy) <= 0 {
                    // t = (center_y - v1.y) / dy in scale-30, then sx = v1.x + dx*t scale-30.
                    let t = div_scale_30(center_y - v1.y, dy);
                    let sx = v1.x.wrapping_add(mul_scale_30(dx, t));
                    let dist = sx - spot.x;
                    if dist < closest_dist && dist >= 0 {
                        closest_dist = dist;
                        closest_seg = j as u32;
                    }
                }
            }
            if closest_dist != FIXED_MAX {
                loop_num = self.mark_loop(closest_seg, loop_num);
            }
        }

        // Restore the (potentially empty) anchor/spot lists. They're no longer needed
        // for the build itself but keeping them lets us assert on them in tests.
        self.poly_starts = spots;
        self.poly_anchors = anchors;
    }

    fn mark_loop(&mut self, first_seg: u32, loop_num: i32) -> i32 {
        let sec = self.segs[first_seg as usize].frontsector;
        if self.segs[first_seg as usize].loop_num != 0 {
            return loop_num;
        }
        let mut seg = first_seg as i32;
        loop {
            let s1_v2 = self.segs[seg as usize].v2 as usize;
            self.segs[seg as usize].loop_num = loop_num;

            let mut best_seg: u32 = NO_NODE_INDEX;
            let mut try_seg = self.vertices[s1_v2].segs;
            let mut best_ang: Angle = ANGLE_MAX;
            let ang1 = self.segs[seg as usize].angle;

            while try_seg != NO_NODE_INDEX {
                let s2 = &self.segs[try_seg as usize];
                if s2.frontsector == sec {
                    let ang2 = s2.angle.wrapping_add(ANGLE_180);
                    let ang_diff = ang2.wrapping_sub(ang1);
                    if ang_diff < best_ang && ang_diff > 0 {
                        best_ang = ang_diff;
                        best_seg = try_seg;
                    }
                }
                try_seg = s2.next_for_vert;
            }
            seg = best_seg as i32;
            if seg == NO_NODE_INDEX as i32 || self.segs[seg as usize].loop_num != 0 {
                break;
            }
        }
        loop_num + 1
    }

    fn get_poly_extents(&self, polynum: i32, bbox: &mut [Fixed; 4]) -> bool {
        bbox[BOX_LEFT] = FIXED_MAX;
        bbox[BOX_BOTTOM] = FIXED_MAX;
        bbox[BOX_RIGHT] = FIXED_MIN;
        bbox[BOX_TOP] = FIXED_MIN;

        // Phase 1: start-line polyobjects.
        let mut start_seg: Option<usize> = None;
        for (i, seg) in self.segs.iter().enumerate() {
            let line = &self.level.lines[seg.linedef as usize];
            if line.special == PO_LINE_START && line.args[0] == polynum {
                start_seg = Some(i);
                break;
            }
        }

        if let Some(start) = start_seg {
            let vert0 = self.segs[start].v1 as usize;
            let start_x = self.vertices[vert0].x;
            let start_y = self.vertices[vert0].y;
            let mut count = self.segs.len();
            let mut i = start;
            loop {
                self.add_seg_to_bbox(bbox, &self.segs[i]);
                let v2 = self.segs[i].v2 as usize;
                let next = self.vertices[v2].segs;
                count = count.saturating_sub(1);
                if count == 0 || next == NO_NODE_INDEX {
                    break;
                }
                let cur_x = self.vertices[v2].x;
                let cur_y = self.vertices[v2].y;
                if cur_x == start_x && cur_y == start_y {
                    break;
                }
                i = next as usize;
            }
            return true;
        }

        // Phase 2: explicit-line polyobjects.
        let mut found = false;
        for seg in &self.segs {
            let line = &self.level.lines[seg.linedef as usize];
            if line.special == PO_LINE_EXPLICIT && line.args[0] == polynum {
                self.add_seg_to_bbox(bbox, seg);
                found = true;
            }
        }
        found
    }

    fn add_seg_to_bbox(&self, bbox: &mut [Fixed; 4], seg: &PrivSeg) {
        let v1 = self.vertices[seg.v1 as usize];
        let v2 = self.vertices[seg.v2 as usize];
        for v in [v1, v2] {
            if v.x < bbox[BOX_LEFT] {
                bbox[BOX_LEFT] = v.x;
            }
            if v.x > bbox[BOX_RIGHT] {
                bbox[BOX_RIGHT] = v.x;
            }
            if v.y < bbox[BOX_BOTTOM] {
                bbox[BOX_BOTTOM] = v.y;
            }
            if v.y > bbox[BOX_TOP] {
                bbox[BOX_TOP] = v.y;
            }
        }
    }

    /// Copy splitter parameters out of a seg into a `Node`. If the seg has a precomputed
    /// plane, prefer that (avoids reconstructing dx/dy from vertices). Mirrors
    /// SetNodeFromSeg from nodebuild.cpp:901.
    pub fn set_node_from_seg(&self, node: &mut Node, pseg: &PrivSeg) {
        if pseg.plane_num >= 0 {
            let pline = self.planes[pseg.plane_num as usize];
            node.x = pline.x;
            node.y = pline.y;
            node.dx = pline.dx;
            node.dy = pline.dy;
        } else {
            let v1 = self.vertices[pseg.v1 as usize];
            let v2 = self.vertices[pseg.v2 as usize];
            node.x = v1.x;
            node.y = v1.y;
            node.dx = v2.x - v1.x;
            node.dy = v2.y - v1.y;
        }
    }

    /// `InterceptVector` from nodebuild.cpp:1023. Returns the parametric `t` along the
    /// seg at which it crosses the splitter; `t=0` for parallel.
    pub fn intercept_vector(&self, splitter: &Node, seg: &PrivSeg) -> f64 {
        let v2x = self.vertices[seg.v1 as usize].x as f64;
        let v2y = self.vertices[seg.v1 as usize].y as f64;
        let v2dx = self.vertices[seg.v2 as usize].x as f64 - v2x;
        let v2dy = self.vertices[seg.v2 as usize].y as f64 - v2y;
        let v1dx = splitter.dx as f64;
        let v1dy = splitter.dy as f64;
        let den = v1dy * v2dx - v1dx * v2dy;
        if den == 0.0 {
            return 0.0;
        }
        let v1x = splitter.x as f64;
        let v1y = splitter.y as f64;
        let num = (v1x - v2x) * v1dy + (v2y - v1y) * v1dx;
        num / den
    }

    /// `SplitSeg` from nodebuild.cpp:920. Splits `segnum` at `splitvert`, returning the
    /// new seg index. `v1_in_front` mirrors the C++ `v1InFront` parameter.
    pub fn split_seg(&mut self, segnum: u32, splitvert: i32, v1_in_front: i32) -> u32 {
        let newnum = self.segs.len() as u32;
        let mut newseg = self.segs[segnum as usize];

        let dx = (self.vertices[splitvert as usize].x - self.vertices[newseg.v1 as usize].x) as f64;
        let dy = (self.vertices[splitvert as usize].y - self.vertices[newseg.v1 as usize].y) as f64;
        let dist = (dx * dx + dy * dy).sqrt();

        if v1_in_front > 0 {
            newseg.offset = newseg.offset.wrapping_add(dist as Fixed);

            let old_v2 = newseg.v2;
            newseg.v1 = splitvert;
            self.segs[segnum as usize].v2 = splitvert;

            self.remove_seg_from_vert2(segnum, old_v2);

            newseg.next_for_vert = self.vertices[splitvert as usize].segs;
            self.vertices[splitvert as usize].segs = newnum;

            newseg.next_for_vert2 = self.vertices[old_v2 as usize].segs2;
            self.vertices[old_v2 as usize].segs2 = newnum;

            self.segs[segnum as usize].next_for_vert2 = self.vertices[splitvert as usize].segs2;
            self.vertices[splitvert as usize].segs2 = segnum;
        } else {
            self.segs[segnum as usize].offset =
                self.segs[segnum as usize].offset.wrapping_add(dist as Fixed);

            let old_v1 = self.segs[segnum as usize].v1;
            self.segs[segnum as usize].v1 = splitvert;
            newseg.v2 = splitvert;

            self.remove_seg_from_vert1(segnum, old_v1);

            newseg.next_for_vert = self.vertices[old_v1 as usize].segs;
            self.vertices[old_v1 as usize].segs = newnum;

            newseg.next_for_vert2 = self.vertices[splitvert as usize].segs2;
            self.vertices[splitvert as usize].segs2 = newnum;

            self.segs[segnum as usize].next_for_vert = self.vertices[splitvert as usize].segs;
            self.vertices[splitvert as usize].segs = segnum;
        }

        self.segs.push(newseg);
        newnum
    }

    fn remove_seg_from_vert1(&mut self, segnum: u32, vertnum: i32) {
        if self.vertices[vertnum as usize].segs == segnum {
            self.vertices[vertnum as usize].segs = self.segs[segnum as usize].next_for_vert;
            return;
        }
        let mut prev = 0u32;
        let mut curr = self.vertices[vertnum as usize].segs;
        while curr != NO_NODE_INDEX && curr != segnum {
            prev = curr;
            curr = self.segs[curr as usize].next_for_vert;
        }
        if curr == segnum {
            self.segs[prev as usize].next_for_vert = self.segs[curr as usize].next_for_vert;
        }
    }

    fn remove_seg_from_vert2(&mut self, segnum: u32, vertnum: i32) {
        if self.vertices[vertnum as usize].segs2 == segnum {
            self.vertices[vertnum as usize].segs2 = self.segs[segnum as usize].next_for_vert2;
            return;
        }
        let mut prev = 0u32;
        let mut curr = self.vertices[vertnum as usize].segs2;
        while curr != NO_NODE_INDEX && curr != segnum {
            prev = curr;
            curr = self.segs[curr as usize].next_for_vert2;
        }
        if curr == segnum {
            self.segs[prev as usize].next_for_vert2 = self.segs[curr as usize].next_for_vert2;
        }
    }
}

/// Match the C++ `DivScale30(a, b) = (int64)a << 30 / b`. The C++ inline asm is i386-only;
/// on x86_64 the codebase falls back to `(int)(double(a)/double(b) * (1<<30))`. We use
/// i64 math which is mathematically equivalent for blockmap-range inputs (the result
/// rounds toward zero either way once it fits in i32).
#[inline]
fn div_scale_30(a: Fixed, b: Fixed) -> Fixed {
    (((a as i64) << 30) / (b as i64)) as Fixed
}

/// Match the C++ `MulScale30(a, b) = ((int64)a * b) >> 30`.
#[inline]
fn mul_scale_30(a: Fixed, b: Fixed) -> Fixed {
    (((a as i64) * (b as i64)) >> 30) as Fixed
}

/// PolyStart collection from THINGS, called by the Processor pre-build. Mirrors the
/// `GetPolySpots` code in processor.cpp:458; placed here because it's tightly bound to
/// the polyobject thing-type constants.
pub fn collect_poly_spots(level: &Level) -> (Vec<PolyStart>, Vec<PolyStart>) {
    let mut starts = Vec::new();
    let mut anchors = Vec::new();

    // If any thing has the Hexen-format anchor type, we use the Hexen IDs; otherwise
    // the ZDoom-format IDs.
    let is_hexen_set = level.things.iter().any(|t| t.kind == PO_HEX_ANCHOR_TYPE);
    let (spot1, spot2, anchor_id) = if is_hexen_set {
        (PO_HEX_SPAWN_TYPE, PO_HEX_SPAWNCRUSH_TYPE, PO_HEX_ANCHOR_TYPE)
    } else {
        (PO_SPAWN_TYPE, PO_SPAWNCRUSH_TYPE, PO_ANCHOR_TYPE)
    };

    for t in &level.things {
        if t.kind == spot1
            || t.kind == spot2
            || t.kind == PO_SPAWNHURT_TYPE
            || t.kind == anchor_id
        {
            let entry = PolyStart {
                polynum: t.angle as i32,
                x: t.x,
                y: t.y,
            };
            if t.kind == anchor_id {
                anchors.push(entry);
            } else {
                starts.push(entry);
            }
        }
    }
    (starts, anchors)
}

// Helpers below silence unused-by-Phase-4b warnings for items the upcoming phases need.
#[allow(dead_code)]
fn _phase_4d_unused(_p: USegPtr, _f: Fixed, _fb: u32) {
    let _ = FRACBITS;
    let _ = NO_NODE_INDEX;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed::point_to_angle;

    #[test]
    fn point_to_angle_axes() {
        // +x axis is angle 0.
        assert_eq!(point_to_angle(1 << 16, 0), 0);
        // +y axis is angle 2^30 (which then becomes 2^31 after << 1) wait — let's just
        // check it's monotonic across the first quadrant.
        let a0 = point_to_angle(1 << 16, 0);
        let a45 = point_to_angle(1 << 16, 1 << 16);
        let a90 = point_to_angle(0, 1 << 16);
        assert!(a0 < a45);
        assert!(a45 < a90);
    }

    #[test]
    fn div_scale_and_mul_scale_round_trip_approximate() {
        // For small values, MulScale30(DivScale30(x, y), y) ≈ x within rounding.
        let x = 5i32 << 16;
        let y = 3i32 << 16;
        let t = div_scale_30(x, y);
        let back = mul_scale_30(t, y);
        assert!((back - x).abs() < 4);
    }
}
