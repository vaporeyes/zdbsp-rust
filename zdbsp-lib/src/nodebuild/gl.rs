// ABOUTME: Port of nodebuild_gl.cpp. GL-only build-time helpers: AddIntersection,
// ABOUTME: FixSplitSharers, AddMinisegs, AddMiniseg, CheckLoopStart, CheckLoopEnd,
// ABOUTME: plus CheckSubsectorOverlappingSegs from nodebuild.cpp's GL path.

use crate::fixed::{point_to_angle, Angle, Fixed};
use crate::workdata::Node;

use super::{
    point_on_side, EventInfo, NodeBuilder, PrivSeg, NO_NODE_INDEX,
};

const ANGLE_180: Angle = 1u32 << 31;
const ANGLE_MAX: Angle = u32::MAX;
const ANGLE_EPSILON: Angle = 5000;

impl<'a> NodeBuilder<'a> {
    /// Direct port of `AddIntersection` (nodebuild_gl.cpp:33). Computes the signed
    /// distance of `vertex` along the splitter and inserts an event record at that
    /// distance, deduping by exact distance match.
    pub(crate) fn add_intersection(&mut self, node: &Node, vertex: i32) -> f64 {
        let v = self.vertices[vertex as usize];
        let dist =
            ((v.x as f64) - (node.x as f64)) * (node.dx as f64)
                + ((v.y as f64) - (node.y as f64)) * (node.dy as f64);
        if self.events.find_event(dist).is_none() {
            self.events.insert(
                dist,
                EventInfo {
                    vertex,
                    front_seg: NO_NODE_INDEX,
                },
            );
        }
        dist
    }

    /// Direct port of `FixSplitSharers` (nodebuild_gl.cpp:64). Force-splits segs that
    /// span more than two events on the current splitter.
    pub(crate) fn fix_split_sharers(&mut self) {
        // Snapshot the shared-event list — `SplitSeg` mutates `Segs`, so we work from
        // a captured copy of the sharer records.
        let sharers = self.split_sharers.clone();
        for i in 0..sharers.len() {
            let mut seg = sharers[i].seg;
            let v2 = self.segs[seg as usize].v2;
            let event_opt = self.events.find_event(sharers[i].distance);
            let Some(event0) = event_opt else { continue };

            let (mut event, mut next) = if sharers[i].forward {
                let Some(succ) = self.events.successor(event0) else {
                    continue;
                };
                let next2 = self.events.successor(succ);
                (Some(succ), next2)
            } else {
                let Some(pred) = self.events.predecessor(event0) else {
                    continue;
                };
                let next2 = self.events.predecessor(pred);
                (Some(pred), next2)
            };

            while let (Some(e), Some(n)) = (event, next) {
                let info_vertex = self.events.info(e).vertex;
                if info_vertex == v2 {
                    break;
                }
                let newseg = self.split_seg(seg, info_vertex, 1);

                let old_next = self.segs[seg as usize].next;
                self.segs[newseg as usize].next = old_next;
                self.segs[seg as usize].next = newseg;

                let partner = self.segs[seg as usize].partner;
                if partner != NO_NODE_INDEX {
                    let endpartner = self.split_seg(partner, info_vertex, 1);
                    let part_next = self.segs[partner as usize].next;
                    self.segs[endpartner as usize].next = part_next;
                    self.segs[partner as usize].next = endpartner;
                    self.segs[seg as usize].partner = endpartner;
                    self.segs[partner as usize].partner = newseg;
                }

                seg = newseg;
                event = Some(n);
                next = if sharers[i].forward {
                    self.events.successor(n)
                } else {
                    self.events.predecessor(n)
                };
            }
        }
    }

    /// Direct port of `AddMinisegs` (nodebuild_gl.cpp:162). Walks event pairs and adds
    /// matching front/back minisegs across the splitter wherever a closed loop can form
    /// on both sides.
    pub(crate) fn add_minisegs(
        &mut self,
        node: &Node,
        splitseg: u32,
        outset0: &mut u32,
        outset1: &mut u32,
    ) {
        let mut prev: Option<u32> = None;
        let mut cur = self.events.get_minimum();
        while let Some(c) = cur {
            if let Some(p) = prev {
                let pv = self.events.info(p).vertex;
                let cv = self.events.info(c).vertex;

                let fseg1 = self.check_loop_start(node.dx, node.dy, pv, cv);
                if fseg1 != NO_NODE_INDEX {
                    let bseg1 = self.check_loop_start(
                        node.dx.wrapping_neg(),
                        node.dy.wrapping_neg(),
                        cv,
                        pv,
                    );
                    if bseg1 != NO_NODE_INDEX {
                        let fseg2 = self.check_loop_end(node.dx, node.dy, cv);
                        if fseg2 != NO_NODE_INDEX {
                            let bseg2 = self.check_loop_end(
                                node.dx.wrapping_neg(),
                                node.dy.wrapping_neg(),
                                pv,
                            );
                            if bseg2 != NO_NODE_INDEX {
                                // Front-side miniseg.
                                let fnseg = self.add_miniseg(pv, cv, NO_NODE_INDEX, fseg1, splitseg);
                                self.segs[fnseg as usize].next = *outset0;
                                *outset0 = fnseg;

                                // Back-side miniseg, paired with the front.
                                let bnseg = self.add_miniseg(cv, pv, fnseg, bseg1, splitseg);
                                self.segs[bnseg as usize].next = *outset1;
                                *outset1 = bnseg;

                                let fsector = self.segs[fseg1 as usize].frontsector;
                                let bsector = self.segs[bseg1 as usize].frontsector;
                                self.segs[fnseg as usize].frontsector = fsector;
                                self.segs[fnseg as usize].backsector = bsector;
                                self.segs[bnseg as usize].frontsector = bsector;
                                self.segs[bnseg as usize].backsector = fsector;
                            }
                        }
                    }
                }
            }
            prev = Some(c);
            cur = self.events.successor(c);
        }
    }

    /// Direct port of `AddMiniseg` (nodebuild_gl.cpp:228). Allocates a new seg between
    /// `v1` and `v2`, splices it into the per-vertex lists, and optionally pairs it
    /// with `partner`.
    pub(crate) fn add_miniseg(
        &mut self,
        v1: i32,
        v2: i32,
        partner: u32,
        seg1: u32,
        splitseg: u32,
    ) -> u32 {
        let seg1_next = self.segs[seg1 as usize].next;
        let plane_num = if splitseg != NO_NODE_INDEX {
            self.segs[splitseg as usize].plane_num
        } else {
            -1
        };
        let new = PrivSeg {
            v1,
            v2,
            sidedef: crate::level::NO_INDEX,
            // In the C++, `linedef` is `int` and the GL miniseg path stuffs `NO_INDEX`
            // (0xFFFFFFFF) into it, where it's later treated as a signed -1. We use -1.
            linedef: -1,
            frontsector: -1,
            backsector: -1,
            next: seg1_next,
            next_for_vert: self.vertices[v1 as usize].segs,
            next_for_vert2: self.vertices[v2 as usize].segs2,
            loop_num: 0,
            partner,
            stored_seg: NO_NODE_INDEX,
            angle: 0,
            offset: 0,
            plane_num,
            plane_front: true,
            hash_next: NO_NODE_INDEX,
        };
        let nseg = self.segs.len() as u32;
        self.segs.push(new);
        if partner != NO_NODE_INDEX {
            self.segs[partner as usize].partner = nseg;
        }
        self.vertices[v1 as usize].segs = nseg;
        self.vertices[v2 as usize].segs2 = nseg;
        nseg
    }

    /// `CheckLoopStart` (nodebuild_gl.cpp:282). Find a seg ending at `vertex` forming
    /// the smallest angle to the splitter — used to decide whether a back-of-splitter
    /// loop can be closed there.
    pub(crate) fn check_loop_start(
        &self,
        dx: Fixed,
        dy: Fixed,
        vertex: i32,
        vertex2: i32,
    ) -> u32 {
        let v = self.vertices[vertex as usize];
        let split_angle = point_to_angle(dx, dy);

        let mut segnum = v.segs2;
        let mut best_ang: Angle = ANGLE_MAX;
        let mut best_seg: u32 = NO_NODE_INDEX;
        while segnum != NO_NODE_INDEX {
            let seg = self.segs[segnum as usize];
            let sv = self.vertices[seg.v1 as usize];
            let seg_angle = point_to_angle(sv.x - v.x, sv.y - v.y);
            let diff = split_angle.wrapping_sub(seg_angle);

            let on_splitter = diff < ANGLE_EPSILON
                && point_on_side(sv.x, sv.y, v.x, v.y, dx, dy) == 0;
            if !on_splitter && diff <= best_ang {
                best_ang = diff;
                best_seg = segnum;
            }
            segnum = seg.next_for_vert2;
        }
        if best_seg == NO_NODE_INDEX {
            return NO_NODE_INDEX;
        }
        // Make sure no seg starting at this vertex forms an even smaller angle.
        let mut segnum = v.segs;
        while segnum != NO_NODE_INDEX {
            let seg = self.segs[segnum as usize];
            if seg.v2 == vertex2 {
                return NO_NODE_INDEX;
            }
            let sv = self.vertices[seg.v2 as usize];
            let seg_angle = point_to_angle(sv.x - v.x, sv.y - v.y);
            let diff = split_angle.wrapping_sub(seg_angle);
            if diff < best_ang && seg.partner != best_seg {
                return NO_NODE_INDEX;
            }
            segnum = seg.next_for_vert;
        }
        best_seg
    }

    /// `CheckLoopEnd` (nodebuild_gl.cpp:341). Mirror of `check_loop_start` for the
    /// "seg starting at vertex" direction. The split-angle here is offset by 180°.
    pub(crate) fn check_loop_end(&self, dx: Fixed, dy: Fixed, vertex: i32) -> u32 {
        let v = self.vertices[vertex as usize];
        let split_angle = point_to_angle(dx, dy).wrapping_add(ANGLE_180);

        let mut segnum = v.segs;
        let mut best_ang: Angle = ANGLE_MAX;
        let mut best_seg: u32 = NO_NODE_INDEX;
        while segnum != NO_NODE_INDEX {
            let seg = self.segs[segnum as usize];
            let sv = self.vertices[seg.v2 as usize];
            let seg_angle = point_to_angle(sv.x - v.x, sv.y - v.y);
            let diff = seg_angle.wrapping_sub(split_angle);

            let on_splitter = diff < ANGLE_EPSILON
                && point_on_side(
                    self.vertices[seg.v1 as usize].x,
                    self.vertices[seg.v1 as usize].y,
                    v.x,
                    v.y,
                    dx,
                    dy,
                ) == 0;
            if !on_splitter && diff <= best_ang {
                best_ang = diff;
                best_seg = segnum;
            }
            segnum = seg.next_for_vert;
        }
        if best_seg == NO_NODE_INDEX {
            return NO_NODE_INDEX;
        }
        let mut segnum = v.segs2;
        while segnum != NO_NODE_INDEX {
            let seg = self.segs[segnum as usize];
            let sv = self.vertices[seg.v1 as usize];
            let seg_angle = point_to_angle(sv.x - v.x, sv.y - v.y);
            let diff = seg_angle.wrapping_sub(split_angle);
            if diff < best_ang && seg.partner != best_seg {
                return NO_NODE_INDEX;
            }
            segnum = seg.next_for_vert2;
        }
        best_seg
    }

    /// `CheckSubsectorOverlappingSegs` from nodebuild.cpp:350. When a set of segs has
    /// all the same front-sector but two share start+end vertices, synthesize a
    /// splitter so the overlapping pair is shoved behind it.
    pub(crate) fn check_subsector_overlapping_segs(
        &mut self,
        set: u32,
        node: &mut Node,
        splitseg: &mut u32,
    ) -> bool {
        let mut seg1 = set;
        while seg1 != NO_NODE_INDEX {
            let s1 = self.segs[seg1 as usize];
            if s1.linedef != -1 {
                let v1 = s1.v1;
                let v2 = s1.v2;
                let mut seg2 = s1.next;
                while seg2 != NO_NODE_INDEX {
                    let s2 = self.segs[seg2 as usize];
                    if s2.v1 == v1 && s2.v2 == v2 {
                        // Prefer to shove the linedef-bearing seg (s1) behind, not the
                        // miniseg variant. The C++ swaps so `seg2` always points at the
                        // non-miniseg candidate.
                        let (a, b) = if s2.linedef == -1 {
                            (seg2, seg1)
                        } else {
                            (seg1, seg2)
                        };
                        *splitseg = NO_NODE_INDEX;
                        return self.shove_seg_behind_helper(set, node, a, b);
                    }
                    seg2 = self.segs[seg2 as usize].next;
                }
            }
            seg1 = self.segs[seg1 as usize].next;
        }
        false
    }

    /// Thin wrapper so `shove_seg_behind` can stay private to `build.rs`.
    fn shove_seg_behind_helper(
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
}
