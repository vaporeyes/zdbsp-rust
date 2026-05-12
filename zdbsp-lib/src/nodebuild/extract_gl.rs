// ABOUTME: Port of nodebuild_extract.cpp's GL paths: GetGLNodes, CloseSubsector,
// ABOUTME: OutputDegenerateSubsector, PushGLSeg, PushConnectingGLSeg. Produces the
// ABOUTME: MapNodeEx / MapSegGlEx / MapSubsectorEx arrays for GL output.

use crate::fixed::{point_to_angle, Angle, Fixed, FRACBITS};
use crate::level::{MapNodeEx, MapSegGlEx, MapSubsectorEx, WideVertex, NO_INDEX};

use super::{NodeBuilder, PrivSeg, NO_NODE_INDEX};

const ANGLE_MAX: Angle = u32::MAX;

/// Bundled GL extraction output.
#[derive(Debug, Default)]
pub struct GlNodeOutput {
    pub vertices: Vec<WideVertex>,
    pub nodes: Vec<MapNodeEx>,
    pub segs: Vec<MapSegGlEx>,
    pub subsectors: Vec<MapSubsectorEx>,
}

impl<'a> NodeBuilder<'a> {
    /// `GetGLNodes` (nodebuild_extract.cpp:36). Builds the GL output by closing each
    /// subsector (sorting + connecting segs into a clockwise polygon and emitting
    /// minisegs as needed) and remapping partner indices.
    pub fn extract_gl(&mut self) -> GlNodeOutput {
        let node_count = self.nodes.len();
        let mut nodes = vec![MapNodeEx::default(); node_count];
        for i in 0..node_count {
            let org = self.nodes[i];
            let mut newnode = MapNodeEx::default();
            newnode.x = org.x;
            newnode.y = org.y;
            newnode.dx = org.dx;
            newnode.dy = org.dy;
            for j in 0..2 {
                for k in 0..4 {
                    newnode.bbox[j][k] = (org.bbox[j][k] >> FRACBITS) as i16;
                }
                newnode.children[j] = org.int_children[j];
            }
            nodes[i] = newnode;
        }

        let sub_count = self.subsectors.len();
        let mut subs = vec![MapSubsectorEx::default(); sub_count];
        let mut segs: Vec<MapSegGlEx> = Vec::with_capacity(self.segs.len() * 5 / 4);
        for i in 0..sub_count {
            let numsegs = self.close_subsector(i as i32, &mut segs);
            subs[i].numlines = numsegs;
            subs[i].firstline = segs.len() as u32 - numsegs;
        }

        // Fix up partner indices: at this point each seg's `partner` field references
        // a `PrivSeg` index; we want a GL-seg index.
        for s in &mut segs {
            if s.partner != NO_NODE_INDEX {
                s.partner = self.segs[s.partner as usize].stored_seg;
            }
        }

        GlNodeOutput {
            vertices: self.extract_vertices(),
            nodes,
            segs,
            subsectors: subs,
        }
    }

    /// `CloseSubsector` (nodebuild_extract.cpp:89). Walks the subsector's segs in
    /// clockwise angular order around the subsector centroid, inserting minisegs as
    /// needed to form a closed polygon. Degenerate (collinear) subsectors take a
    /// three-stage forward/backward/forward sweep instead.
    fn close_subsector(&mut self, subsector: i32, segs: &mut Vec<MapSegGlEx>) -> u32 {
        let sub = self.subsectors[subsector as usize];
        let first = sub.first_line as usize;
        let max = first + sub.num_lines as usize;
        let mut count: u32;

        let mut accum_x: f64 = 0.0;
        let mut accum_y: f64 = 0.0;
        let first_plane = self.segs[self.seg_list[first].seg_num as usize].plane_num;
        let mut diff_planes = false;

        for i in first..max {
            let seg = self.segs[self.seg_list[i].seg_num as usize];
            let v1 = self.vertices[seg.v1 as usize];
            let v2 = self.vertices[seg.v2 as usize];
            accum_x += v1.x as f64 + v2.x as f64;
            accum_y += v1.y as f64 + v2.y as f64;
            if first_plane != seg.plane_num {
                diff_planes = true;
            }
        }
        let n = (max - first) as f64;
        let midx: Fixed = (accum_x / n / 2.0) as Fixed;
        let midy: Fixed = (accum_y / n / 2.0) as Fixed;

        let first_seg = self.segs[self.seg_list[first].seg_num as usize];
        let mut prev_idx = self.seg_list[first].seg_num;
        let mut prev = first_seg;
        let mut prev_angle = point_to_angle(
            self.vertices[prev.v1 as usize].x - midx,
            self.vertices[prev.v1 as usize].y - midy,
        );
        let stored = self.push_gl_seg(segs, &prev);
        self.segs[prev_idx as usize].stored_seg = stored;
        count = 1;
        let first_vert = prev.v1;

        if diff_planes {
            // "Well-behaved" subsector — angle-sort the segs.
            //
            // Faithful port of nodebuild_extract.cpp:155-201. The C++ tracks `seg` as
            // a live pointer assigned each j-iteration and, on j-loop exit, either
            //   (a) `seg == bestseg` if we broke out via `seg->v1 == prev->v2`, or
            //   (b) `seg == Segs[SegList[max-1]]` if we fell through normally, after
            //       which `if (bestseg != NULL) seg = bestseg` overrides.
            // The storedseg write target is whatever `seg` was after that override.
            for _i in (first + 1)..max {
                let mut best_diff: Angle = ANGLE_MAX;
                let mut best_seg_idx: u32 = NO_NODE_INDEX;
                let mut last_j_seg_idx: u32 = NO_NODE_INDEX; // matches `seg` at j-loop exit
                let mut broke_early = false;
                for j in first..max {
                    let cand_idx = self.seg_list[j].seg_num;
                    last_j_seg_idx = cand_idx;
                    let cand = self.segs[cand_idx as usize];
                    let cv1 = self.vertices[cand.v1 as usize];
                    let ang = point_to_angle(cv1.x - midx, cv1.y - midy);
                    let diff = prev_angle.wrapping_sub(ang);
                    if cand.v1 == prev.v2 {
                        best_diff = diff;
                        best_seg_idx = cand_idx;
                        broke_early = true;
                        break;
                    }
                    if diff < best_diff && diff > 0 {
                        best_diff = diff;
                        best_seg_idx = cand_idx;
                    }
                }
                // After j-loop: `seg_idx` == `bestseg` if we broke early, otherwise
                // last-j; then `if (bestseg != NULL) seg = bestseg` override applies.
                let seg_idx = if broke_early {
                    best_seg_idx
                } else if best_seg_idx != NO_NODE_INDEX {
                    best_seg_idx
                } else {
                    last_j_seg_idx
                };
                if seg_idx == NO_NODE_INDEX {
                    continue;
                }
                let seg_chosen = self.segs[seg_idx as usize];
                if prev.v2 != seg_chosen.v1 {
                    self.push_connecting_gl_seg(segs, prev.v2, seg_chosen.v1);
                    count += 1;
                }
                prev_angle = prev_angle.wrapping_sub(best_diff);
                let stored = self.push_gl_seg(segs, &seg_chosen);
                self.segs[seg_idx as usize].stored_seg = stored;
                count += 1;
                prev = seg_chosen;
                if seg_chosen.v2 == first_vert {
                    break;
                }
            }
        } else {
            // Degenerate subsector — three-stage forward/backward/forward sweep.
            count += self.output_degenerate_subsector(segs, subsector, true, 0.0, &mut prev, &mut prev_idx);
            count += self.output_degenerate_subsector(
                segs,
                subsector,
                false,
                f64::MAX,
                &mut prev,
                &mut prev_idx,
            );
            count += self.output_degenerate_subsector(
                segs,
                subsector,
                true,
                -f64::MAX,
                &mut prev,
                &mut prev_idx,
            );
        }

        if prev.v2 != first_vert {
            self.push_connecting_gl_seg(segs, prev.v2, first_vert);
            count += 1;
        }
        count
    }

    /// `OutputDegenerateSubsector` (nodebuild_extract.cpp:253). Forward/backward sweep
    /// through segs lying on the same plane as the start seg, picking the one with the
    /// best dot product against the lead direction.
    fn output_degenerate_subsector(
        &mut self,
        segs: &mut Vec<MapSegGlEx>,
        subsector: i32,
        forward: bool,
        mut lastdot: f64,
        prev: &mut PrivSeg,
        prev_idx: &mut u32,
    ) -> u32 {
        let sub = self.subsectors[subsector as usize];
        let first = sub.first_line as usize;
        let max = first + sub.num_lines as usize;
        let mut count: u32 = 0;

        let lead = self.segs[self.seg_list[first].seg_num as usize];
        let lv1 = self.vertices[lead.v1 as usize];
        let lv2 = self.vertices[lead.v2 as usize];
        let x1 = lv1.x as f64;
        let y1 = lv1.y as f64;
        let dx = lv2.x as f64 - x1;
        let dy = lv2.y as f64 - y1;
        let want_side = lead.plane_front ^ !forward;

        for _i in (first + 1)..max {
            // C++ uses `static const double bestinit[2] = { -DBL_MAX, DBL_MAX };
            // double bestdot = bestinit[bForward];` — i.e. bForward indexes as a `bool→int`:
            // `false→0→-MAX`, `true→1→+MAX`. We had this inverted previously, which made
            // the forward sweep's `dot < bestdot` test never succeed and silently no-op
            // the entire degenerate-subsector forward path.
            let init: f64 = if forward { f64::MAX } else { -f64::MAX };
            let mut best_dot = init;
            let mut best_idx: u32 = NO_NODE_INDEX;
            let mut last_cand_idx: u32 = NO_NODE_INDEX;
            for j in (first + 1)..max {
                let cand_idx = self.seg_list[j].seg_num;
                last_cand_idx = cand_idx;
                let cand = self.segs[cand_idx as usize];
                if cand.plane_front != want_side {
                    continue;
                }
                let cv1 = self.vertices[cand.v1 as usize];
                let dx2 = cv1.x as f64 - x1;
                let dy2 = cv1.y as f64 - y1;
                let dot = dx * dx2 + dy * dy2;
                if forward {
                    if dot < best_dot && dot > lastdot {
                        best_dot = dot;
                        best_idx = cand_idx;
                    }
                } else if dot > best_dot && dot < lastdot {
                    best_dot = dot;
                    best_idx = cand_idx;
                }
            }
            if best_idx != NO_NODE_INDEX {
                let best_seg = self.segs[best_idx as usize];
                if prev.v2 != best_seg.v1 {
                    self.push_connecting_gl_seg(segs, prev.v2, best_seg.v1);
                    count += 1;
                }
                let stored = self.push_gl_seg(segs, &best_seg);
                // Mirror C++ bug: storedseg written to last j-iteration's seg.
                self.segs[last_cand_idx as usize].stored_seg = stored;
                count += 1;
                *prev = best_seg;
                *prev_idx = best_idx;
                lastdot = best_dot;
            }
        }
        count
    }

    /// `PushGLSeg` (nodebuild_extract.cpp:320). Build a `MapSegGlEx` for `seg`,
    /// determining side via sidedef ID (with a sidedef-compression fallback).
    fn push_gl_seg(&self, segs: &mut Vec<MapSegGlEx>, seg: &PrivSeg) -> u32 {
        let mut new = MapSegGlEx::default();
        new.v1 = seg.v1 as u32;
        new.v2 = seg.v2 as u32;
        new.linedef = if seg.linedef == -1 {
            NO_INDEX
        } else {
            seg.linedef as u32
        };

        if new.linedef != NO_INDEX {
            let ld = &self.level.lines[new.linedef as usize];
            if ld.sidenum[0] == ld.sidenum[1] {
                // The C++ does `Level.Vertices[ld->v1]` after replacing `Level.Vertices`
                // with the builder's expanded vertex array (processor.cpp:598-599). Our
                // builder vertices live in `self.vertices`, so look up *all three*
                // (lv1, sv1, sv2) there — `ld.v1` is a builder-vertex index after
                // `find_used_vertices` remapped it.
                let lv1 = &self.vertices[ld.v1 as usize];
                let sv1 = &self.vertices[seg.v1 as usize];
                let sv2 = &self.vertices[seg.v2 as usize];
                let d1x = (sv1.x - lv1.x) as f64;
                let d1y = (sv1.y - lv1.y) as f64;
                let d2x = (sv2.x - lv1.x) as f64;
                let d2y = (sv2.y - lv1.y) as f64;
                let dist1 = d1x * d1x + d1y * d1y;
                let dist2 = d2x * d2x + d2y * d2y;
                new.side = if dist1 < dist2 { 0 } else { 1 };
            } else {
                new.side = if ld.sidenum[1] == seg.sidedef { 1 } else { 0 };
            }
        } else {
            new.side = 0;
        }
        new.partner = seg.partner;
        let idx = segs.len() as u32;
        segs.push(new);
        idx
    }

    /// `PushConnectingGLSeg` (nodebuild_extract.cpp:363). Emit a synthetic miniseg
    /// connecting two vertices in an unclosed subsector.
    fn push_connecting_gl_seg(&self, segs: &mut Vec<MapSegGlEx>, v1: i32, v2: i32) {
        segs.push(MapSegGlEx {
            v1: v1 as u32,
            v2: v2 as u32,
            linedef: NO_INDEX,
            side: 0,
            partner: NO_NODE_INDEX,
        });
    }
}
