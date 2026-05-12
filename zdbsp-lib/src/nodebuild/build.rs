// ABOUTME: Port of nodebuild.cpp. The partition loop: BuildTree, CreateNode,
// ABOUTME: CreateSubsector(ForReal), CheckSubsector, SelectSplitter, Heuristic, SplitSegs,
// ABOUTME: ShoveSegBehind, SortSegs. GL-specific helpers (AddIntersection / AddMinisegs /
// ABOUTME: FixSplitSharers / AddMiniseg) are stubbed and panic if invoked with gl_nodes=true.

use crate::fixed::Fixed;
use crate::workdata::{Node, Subsector, NFX_SUBSECTOR};

use super::classify::{classify_line, Side};
use super::{PrivSeg, PrivVert, SplitSharer, USegPtr, NodeBuilder, NO_NODE_INDEX, VERTEX_EPSILON};

const BOX_TOP: usize = 0;
const BOX_BOTTOM: usize = 1;
const BOX_LEFT: usize = 2;
const BOX_RIGHT: usize = 3;

impl<'a> NodeBuilder<'a> {
    /// Top of the partition pipeline. Allocates subsectors after the tree is shaped.
    pub fn build_tree(&mut self) {
        self.hack_seg = NO_NODE_INDEX;
        self.hack_mate = NO_NODE_INDEX;
        // Initial set chains all segs via `next` (already done by group_seg_planes).
        let mut bbox = [0i32; 4];
        let count = self.segs.len() as u32;
        self.create_node(0, count, &mut bbox);
        self.create_subsectors_for_real();
    }

    /// Direct port of CreateNode (nodebuild.cpp:76). Recursive. Returns the index of
    /// the produced node, or a `NFX_SUBSECTOR`-tagged subsector index.
    pub(crate) fn create_node(
        &mut self,
        set: u32,
        count: u32,
        out_bbox: &mut [Fixed; 4],
    ) -> u32 {
        let mut node = Node::default();
        let mut splitseg: u32 = NO_NODE_INDEX;

        let max_segs = self.options.max_segs.max(1);
        let skip = (count as i32) / max_segs;

        let split_decision = {
            // Try `(step=skip, nosplit=true)` first.
            let selstat = self.select_splitter(set, &mut node, &mut splitseg, skip, true);
            if selstat > 0 {
                true
            } else if skip > 0 && self.select_splitter(set, &mut node, &mut splitseg, 1, true) > 0 {
                true
            } else if selstat < 0 {
                // "There were possible splitters but they all split lines we wanted to
                // keep". Retry without the nosplit constraint.
                if self.select_splitter(set, &mut node, &mut splitseg, skip, false) > 0 {
                    true
                } else if skip > 0
                    && self.select_splitter(set, &mut node, &mut splitseg, 1, false) > 0
                {
                    true
                } else {
                    self.check_subsector(set, &mut node, &mut splitseg)
                }
            } else {
                self.check_subsector(set, &mut node, &mut splitseg)
            }
        };

        if split_decision {
            let mut set1: u32 = NO_NODE_INDEX;
            let mut set2: u32 = NO_NODE_INDEX;
            let mut count1: u32 = 0;
            let mut count2: u32 = 0;
            self.split_segs(set, &node, splitseg, &mut set1, &mut set2, &mut count1, &mut count2);

            let mut bbox0 = [0i32; 4];
            let mut bbox1 = [0i32; 4];
            node.int_children[0] = self.create_node(set1, count1, &mut bbox0);
            node.int_children[1] = self.create_node(set2, count2, &mut bbox1);
            node.bbox[0] = bbox0;
            node.bbox[1] = bbox1;

            out_bbox[BOX_TOP] = bbox0[BOX_TOP].max(bbox1[BOX_TOP]);
            out_bbox[BOX_BOTTOM] = bbox0[BOX_BOTTOM].min(bbox1[BOX_BOTTOM]);
            out_bbox[BOX_LEFT] = bbox0[BOX_LEFT].min(bbox1[BOX_LEFT]);
            out_bbox[BOX_RIGHT] = bbox0[BOX_RIGHT].max(bbox1[BOX_RIGHT]);

            let idx = self.nodes.len() as u32;
            self.nodes.push(node);
            idx
        } else {
            NFX_SUBSECTOR | self.create_subsector(set, out_bbox)
        }
    }

    fn create_subsector(&mut self, set: u32, out_bbox: &mut [Fixed; 4]) -> u32 {
        out_bbox[BOX_TOP] = i32::MIN;
        out_bbox[BOX_RIGHT] = i32::MIN;
        out_bbox[BOX_BOTTOM] = i32::MAX;
        out_bbox[BOX_LEFT] = i32::MAX;
        debug_assert!(set != NO_NODE_INDEX);

        let ssnum = self.subsector_sets.len() as u32;
        self.subsector_sets.push(set);

        let mut count = 0u32;
        let mut s = set;
        while s != NO_NODE_INDEX {
            // Inline `add_seg_to_bbox` to keep the borrow on `self.segs` simple.
            let v1 = self.vertices[self.segs[s as usize].v1 as usize];
            let v2 = self.vertices[self.segs[s as usize].v2 as usize];
            for v in [v1, v2] {
                if v.x < out_bbox[BOX_LEFT] {
                    out_bbox[BOX_LEFT] = v.x;
                }
                if v.x > out_bbox[BOX_RIGHT] {
                    out_bbox[BOX_RIGHT] = v.x;
                }
                if v.y < out_bbox[BOX_BOTTOM] {
                    out_bbox[BOX_BOTTOM] = v.y;
                }
                if v.y > out_bbox[BOX_TOP] {
                    out_bbox[BOX_TOP] = v.y;
                }
            }
            s = self.segs[s as usize].next;
            count += 1;
        }
        self.segs_stuffed = self.segs_stuffed.saturating_add(count as i32);
        ssnum
    }

    /// CreateSubsectorsForReal (nodebuild.cpp:177): once the tree is built, walk each
    /// queued subsector set, copy the seg-chain into `seg_list`, and sort it by linedef.
    pub(crate) fn create_subsectors_for_real(&mut self) {
        for i in 0..self.subsector_sets.len() {
            let first_line = self.seg_list.len() as u32;
            let mut s = self.subsector_sets[i];
            while s != NO_NODE_INDEX {
                self.seg_list.push(USegPtr { seg_num: s });
                s = self.segs[s as usize].next;
            }
            let num_lines = self.seg_list.len() as u32 - first_line;

            // Sort by linedef for "special effects". The C++ uses qsort (unstable) here;
            // we use stable `sort_by` so ties retain insertion order. For non-GL output
            // this matches because segs from the same subsector with the same linedef
            // are not generated.
            let slice = &mut self.seg_list[first_line as usize..];
            // Capture the comparator data (segs[]) by value-copy into a local closure.
            // Cannot borrow `self.segs` and `self.seg_list` simultaneously, so snapshot.
            let segs_snap: Vec<(i32, i32, i32)> = slice
                .iter()
                .map(|p| {
                    let s = &self.segs[p.seg_num as usize];
                    (s.linedef, s.frontsector, s.backsector)
                })
                .collect();
            let mut indices: Vec<usize> = (0..slice.len()).collect();
            indices.sort_by(|&a, &b| sort_segs_cmp(segs_snap[a], segs_snap[b]));
            // Apply the permutation back to seg_list.
            let original: Vec<USegPtr> = slice.to_vec();
            for (out_pos, &orig_pos) in indices.iter().enumerate() {
                slice[out_pos] = original[orig_pos];
            }

            self.subsectors.push(Subsector {
                num_lines,
                first_line,
            });
        }
    }

    /// CheckSubsector (nodebuild.cpp:289). Returns true if a splitter was synthesized
    /// (continue down a node), false if the set is a valid subsector.
    pub(crate) fn check_subsector(
        &mut self,
        set: u32,
        node: &mut Node,
        splitseg: &mut u32,
    ) -> bool {
        let mut sec: i32 = -1;
        let mut seg = set;
        let mut multi_sector_seg: u32 = NO_NODE_INDEX;

        loop {
            let s = &self.segs[seg as usize];
            if s.linedef != -1 && s.frontsector != sec {
                if sec == -1 {
                    sec = s.frontsector;
                } else {
                    multi_sector_seg = seg;
                    break;
                }
            }
            let nxt = s.next;
            if nxt == NO_NODE_INDEX {
                break;
            }
            seg = nxt;
        }

        if multi_sector_seg == NO_NODE_INDEX {
            // Valid subsector.
            if self.gl_nodes {
                return self.check_subsector_overlapping_segs(set, node, splitseg);
            }
            return false;
        }

        *splitseg = NO_NODE_INDEX;
        self.shove_seg_behind(set, node, multi_sector_seg, NO_NODE_INDEX)
    }

    /// ShoveSegBehind (nodebuild.cpp:393). Forces `seg` to the back of a synthesized
    /// splitter so the multi-sector subsector can split cleanly.
    fn shove_seg_behind(
        &mut self,
        set: u32,
        node: &mut Node,
        seg: u32,
        mate: u32,
    ) -> bool {
        let pseg = self.segs[seg as usize];
        self.set_node_from_seg(node, &pseg);
        self.hack_seg = seg;
        self.hack_mate = mate;
        if !pseg.plane_front {
            node.x = node.x.wrapping_add(node.dx);
            node.y = node.y.wrapping_add(node.dy);
            node.dx = node.dx.wrapping_neg();
            node.dy = node.dy.wrapping_neg();
        }
        self.heuristic(node, set, false) > 0
    }

    /// SelectSplitter (nodebuild.cpp:415). Returns 1 if a good splitter was found
    /// (writes it into `node` + `splitseg`), 0 for a convex region, -1 if every
    /// candidate splits a "nosplit" seg.
    pub(crate) fn select_splitter(
        &mut self,
        set: u32,
        node: &mut Node,
        splitseg: &mut u32,
        step: i32,
        nosplit: bool,
    ) -> i32 {
        let mut best_value = 0;
        let mut best_seg: u32 = NO_NODE_INDEX;
        let mut nosplitters = false;
        let mut stepleft = 0i32;

        // Clear the plane-checked bitset.
        for byte in &mut self.plane_checked {
            *byte = 0;
        }

        let mut seg = set;
        while seg != NO_NODE_INDEX {
            let pseg = self.segs[seg as usize];
            stepleft -= 1;
            if stepleft <= 0 {
                let plane = pseg.plane_num;
                let l = plane >> 3;
                let r = 1u8 << (plane & 7);
                let already_checked = l >= 0 && (self.plane_checked[l as usize] & r) != 0;
                if !already_checked {
                    if l >= 0 {
                        self.plane_checked[l as usize] |= r;
                    }
                    stepleft = step;
                    self.set_node_from_seg(node, &pseg);
                    let value = self.heuristic(node, set, nosplit);
                    if value > best_value {
                        best_value = value;
                        best_seg = seg;
                    } else if value < 0 {
                        nosplitters = true;
                    }
                }
            }
            seg = pseg.next;
        }

        if best_seg == NO_NODE_INDEX {
            return if nosplitters { -1 } else { 0 };
        }

        *splitseg = best_seg;
        let pseg = self.segs[best_seg as usize];
        self.set_node_from_seg(node, &pseg);
        1
    }

    /// Heuristic (nodebuild.cpp:497). Scores a candidate splitter; higher is better.
    /// Returns -1 when the splitter is forbidden (cuts a nosplit seg, leaves an empty
    /// side, etc.).
    pub(crate) fn heuristic(&mut self, node: &Node, set: u32, honor_no_split: bool) -> i32 {
        let mut score: i32 = 1_000_000;
        let mut segs_in_set: i32 = 0;
        let mut counts = [0i32; 2];
        let mut real_segs = [0i32; 2];
        let mut special_segs = [0i32; 2];
        let mut splitter = false;

        self.touched.clear();
        self.colinear.clear();

        let mut i = set;
        while i != NO_NODE_INDEX {
            let test = self.segs[i as usize];
            let mut sidev = [0i32; 2];
            let side = if self.hack_seg == i {
                sidev[0] = 0;
                sidev[1] = 0;
                Side::Back
            } else {
                let v1: PrivVert = self.vertices[test.v1 as usize];
                let v2: PrivVert = self.vertices[test.v2 as usize];
                let vv1 = crate::workdata::Vertex { x: v1.x, y: v1.y };
                let vv2 = crate::workdata::Vertex { x: v2.x, y: v2.y };
                classify_line(node, &vv1, &vv2, &mut sidev)
            };

            match side {
                Side::Front | Side::Back => {
                    let side_idx = side.as_i32() as usize;
                    // The "abuts but doesn't cross" check.
                    if test.loop_num != 0 && honor_no_split && (sidev[0] == 0 || sidev[1] == 0) {
                        if (sidev[0] | sidev[1]) != 0 {
                            if !self.touched.contains(&test.loop_num) {
                                self.touched.push(test.loop_num);
                            }
                        } else if !self.colinear.contains(&test.loop_num) {
                            self.colinear.push(test.loop_num);
                        }
                    }
                    counts[side_idx] += 1;
                    if test.linedef != -1 {
                        real_segs[side_idx] += 1;
                        if test.frontsector == test.backsector {
                            special_segs[side_idx] += 1;
                        }
                        score = score.wrapping_add(self.options.split_cost);
                    } else {
                        score = score.wrapping_add(self.options.split_cost / 4);
                    }
                }
                Side::Cross => {
                    if test.loop_num != 0 {
                        if honor_no_split {
                            return -1;
                        } else {
                            splitter = true;
                        }
                    }

                    let frac = self.intercept_vector(node, &test);
                    if frac < 0.001 || frac > 0.999 {
                        let v1 = self.vertices[test.v1 as usize];
                        let v2 = self.vertices[test.v2 as usize];
                        let mut x = v1.x as f64;
                        let mut y = v1.y as f64;
                        x += frac * (v2.x as f64 - x);
                        y += frac * (v2.y as f64 - y);
                        if (x - v1.x as f64).abs() < (VERTEX_EPSILON + 1) as f64
                            && (y - v1.y as f64).abs() < (VERTEX_EPSILON + 1) as f64
                        {
                            return -1;
                        }
                        if (x - v2.x as f64).abs() < (VERTEX_EPSILON + 1) as f64
                            && (y - v2.y as f64).abs() < (VERTEX_EPSILON + 1) as f64
                        {
                            return -1;
                        }
                        let frac_pen = if frac > 0.999 { 1.0 - frac } else { frac };
                        let penalty = (1.0 / frac_pen) as i32;
                        score = (score - penalty).max(1);
                    }

                    counts[0] += 1;
                    counts[1] += 1;
                    if test.linedef != -1 {
                        real_segs[0] += 1;
                        real_segs[1] += 1;
                        if test.frontsector == test.backsector {
                            special_segs[0] += 1;
                            special_segs[1] += 1;
                        }
                    }
                }
            }

            segs_in_set += 1;
            i = test.next;
        }

        if counts[0] == 0 || counts[1] == 0 {
            return 0;
        }
        if real_segs[0] == 0 || real_segs[1] == 0 {
            return -1;
        }
        if honor_no_split
            && (special_segs[0] == real_segs[0] || special_segs[1] == real_segs[1])
        {
            return -1;
        }

        // Touched-without-colinear check (polyobject containment safety).
        let touched_len = self.touched.len();
        let colinear_len = self.colinear.len();
        if colinear_len == 0 && touched_len > 0 {
            return -1;
        }
        for &t in &self.touched {
            let mut found = false;
            for &c in &self.colinear {
                if c == t {
                    found = true;
                    break;
                }
            }
            if !found {
                return -1;
            }
        }

        // Axis-aligned bonus.
        if node.dx == 0 || node.dy == 0 {
            if splitter {
                score = score.wrapping_add(segs_in_set * 8);
            } else {
                let pref = self.options.aa_preference.max(1);
                score = score.wrapping_add(segs_in_set / pref);
            }
        }

        score = score.wrapping_add((counts[0] + counts[1]) - (counts[0] - counts[1]).abs());
        score
    }

    /// SplitSegs (nodebuild.cpp:736). Partitions `set` into two seg chains by classifying
    /// every seg against `node`; crossing segs are split at the intersection vertex.
    pub(crate) fn split_segs(
        &mut self,
        mut set: u32,
        node: &Node,
        splitseg: u32,
        outset0: &mut u32,
        outset1: &mut u32,
        count0: &mut u32,
        count1: &mut u32,
    ) {
        *outset0 = NO_NODE_INDEX;
        *outset1 = NO_NODE_INDEX;
        let mut c0 = 0u32;
        let mut c1 = 0u32;

        self.events.delete_all();
        self.split_sharers.clear();

        while set != NO_NODE_INDEX {
            let seg_snap = self.segs[set as usize];
            let next = seg_snap.next;

            let mut sidev = [0i32; 2];
            let (side, hack) = if self.hack_seg == set {
                self.hack_seg = NO_NODE_INDEX;
                (Side::Back, true)
            } else {
                let v1 = self.vertices[seg_snap.v1 as usize];
                let v2 = self.vertices[seg_snap.v2 as usize];
                let vv1 = crate::workdata::Vertex { x: v1.x, y: v1.y };
                let vv2 = crate::workdata::Vertex { x: v2.x, y: v2.y };
                let s = classify_line(node, &vv1, &vv2, &mut sidev);
                (s, false)
            };

            match side {
                Side::Front => {
                    self.segs[set as usize].next = *outset0;
                    *outset0 = set;
                    c0 += 1;
                }
                Side::Back => {
                    self.segs[set as usize].next = *outset1;
                    *outset1 = set;
                    c1 += 1;
                }
                Side::Cross => {
                    let frac = self.intercept_vector(node, &seg_snap);
                    let v1 = self.vertices[seg_snap.v1 as usize];
                    let v2 = self.vertices[seg_snap.v2 as usize];
                    let nx = v1.x as f64 + frac * (v2.x as f64 - v1.x as f64);
                    let ny = v1.y as f64 + frac * (v2.y as f64 - v1.y as f64);
                    let newvert = PrivVert {
                        x: nx as Fixed,
                        y: ny as Fixed,
                        segs: 0,
                        segs2: 0,
                        index: 0,
                    };
                    let vertnum = self.select_vertex_close(newvert);

                    let seg2 = self.split_seg(set, vertnum, sidev[0]);

                    self.segs[seg2 as usize].next = *outset0;
                    *outset0 = seg2;
                    self.segs[set as usize].next = *outset1;
                    *outset1 = set;
                    c0 += 1;
                    c1 += 1;

                    let partner = self.segs[set as usize].partner;
                    if partner != NO_NODE_INDEX {
                        let partner1 = partner;
                        let partner2 = self.split_seg(partner1, vertnum, sidev[1]);
                        self.segs[partner1 as usize].next = partner2;
                        self.segs[partner2 as usize].partner = seg2;
                        self.segs[seg2 as usize].partner = partner2;
                    }

                    if self.gl_nodes {
                        self.add_intersection(node, vertnum);
                    }
                }
            }

            // GL-only: record vertex intersections + split sharers.
            if (matches!(side, Side::Front | Side::Back)) && self.gl_nodes {
                if sidev[0] == 0 {
                    let dist1 = self.add_intersection(node, seg_snap.v1);
                    if sidev[1] == 0 {
                        let dist2 = self.add_intersection(node, seg_snap.v2);
                        self.split_sharers.push(SplitSharer {
                            distance: dist1,
                            seg: set,
                            forward: dist2 > dist1,
                        });
                    }
                } else if sidev[1] == 0 {
                    self.add_intersection(node, seg_snap.v2);
                }
            }

            if hack && self.gl_nodes {
                let seg_v1 = self.segs[set as usize].v1;
                let seg_v2 = self.segs[set as usize].v2;
                let newback = self.add_miniseg(seg_v2, seg_v1, NO_NODE_INDEX, set, splitseg);
                let newfront = if self.hack_mate == NO_NODE_INDEX {
                    let f = self.add_miniseg(seg_v1, seg_v2, newback, set, splitseg);
                    self.segs[f as usize].next = *outset0;
                    *outset0 = f;
                    f
                } else {
                    let f = self.hack_mate;
                    self.segs[f as usize].partner = newback;
                    self.segs[newback as usize].partner = f;
                    f
                };
                let front_sec = self.segs[set as usize].frontsector;
                self.segs[newback as usize].frontsector = front_sec;
                self.segs[newback as usize].backsector = front_sec;
                self.segs[newfront as usize].frontsector = front_sec;
                self.segs[newfront as usize].backsector = front_sec;
                self.segs[newback as usize].next = *outset1;
                *outset1 = newback;
            }

            set = next;
        }

        self.fix_split_sharers();
        if self.gl_nodes {
            self.add_minisegs(node, splitseg, outset0, outset1);
        }
        *count0 = c0;
        *count1 = c1;
    }

    // ---- GL stubs (filled in Phase 5) ---------------------------------------------

    pub(crate) fn check_subsector_overlapping_segs(
        &mut self,
        _set: u32,
        _node: &mut Node,
        _splitseg: &mut u32,
    ) -> bool {
        unimplemented!("GL nodes land in Phase 5; this code path requires gl_nodes=false");
    }

    pub(crate) fn add_intersection(&mut self, _node: &Node, _vertex: i32) -> f64 {
        unimplemented!("GL nodes land in Phase 5; this code path requires gl_nodes=false");
    }

    pub(crate) fn add_minisegs(
        &mut self,
        _node: &Node,
        _splitseg: u32,
        _outset0: &mut u32,
        _outset1: &mut u32,
    ) {
        unimplemented!("GL nodes land in Phase 5; this code path requires gl_nodes=false");
    }

    pub(crate) fn add_miniseg(
        &mut self,
        _v1: i32,
        _v2: i32,
        _partner: u32,
        _seg1: u32,
        _splitseg: u32,
    ) -> u32 {
        unimplemented!("GL nodes land in Phase 5; this code path requires gl_nodes=false");
    }

    /// FixSplitSharers (nodebuild_gl.cpp:64). Body is a no-op when `split_sharers` is
    /// empty, which it always is for non-GL builds; we only panic if someone populates
    /// it ahead of Phase 5.
    pub(crate) fn fix_split_sharers(&mut self) {
        if self.split_sharers.is_empty() {
            return;
        }
        unimplemented!("FixSplitSharers requires the GL helpers from Phase 5");
    }
}

// `SortSegs` comparator from nodebuild.cpp:224, lifted into a free function operating
// on (linedef, frontsector, backsector) tuples so the borrow checker is happy.
fn sort_segs_cmp(x: (i32, i32, i32), y: (i32, i32, i32)) -> std::cmp::Ordering {
    let xtype = if x.0 == -1 {
        2
    } else if x.1 == x.2 {
        1
    } else {
        0
    };
    let ytype = if y.0 == -1 {
        2
    } else if y.1 == y.2 {
        1
    } else {
        0
    };
    if xtype != ytype {
        return xtype.cmp(&ytype);
    }
    if xtype < 2 {
        x.0.cmp(&y.0)
    } else {
        std::cmp::Ordering::Equal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_comparator_groups() {
        // (linedef, front, back). xtype groups: 0 = different sectors, 1 = same, 2 = mini.
        let mini = (-1, 0, 0);
        let same = (5, 1, 1);
        let diff_a = (3, 1, 2);
        let diff_b = (7, 1, 2);
        assert_eq!(sort_segs_cmp(diff_a, same), std::cmp::Ordering::Less);
        assert_eq!(sort_segs_cmp(same, mini), std::cmp::Ordering::Less);
        assert_eq!(sort_segs_cmp(diff_a, diff_b), std::cmp::Ordering::Less);
        // Within type 2, comparator returns Equal regardless of linedef.
        assert_eq!(sort_segs_cmp(mini, mini), std::cmp::Ordering::Equal);
    }
}

// Silence dead_code warnings in helper imports until 4e consumes them.
#[allow(dead_code)]
fn _dummy(_p: PrivSeg, _u: USegPtr, _s: SplitSharer) {}
