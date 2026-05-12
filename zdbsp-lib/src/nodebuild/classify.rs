// ABOUTME: Scalar ClassifyLine implementation. Direct port of nodebuild_classify_nosse2.cpp.
// ABOUTME: Determines which side(s) of a splitter a seg lies on. SIMD variants land in Phase 7.

use crate::workdata::{Node, Vertex};

use super::SIDE_EPSILON;

/// 4 << 32. Threshold below which we fall back to the precise distance test.
const FAR_ENOUGH: f64 = 17_179_869_184.0;

/// Side returned by [`classify_line`].
///
/// * `Front` — seg is entirely in front of the splitter.
/// * `Back` — seg is entirely behind.
/// * `Cross` — seg crosses the splitter; `sidev[0]` and `sidev[1]` differ in sign.
///
/// Mirrors the C++ contract:
/// * `0` = in front
/// * `1` = in back
/// * `-1` = crosses
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Front = 0,
    Back = 1,
    Cross = -1,
}

impl Side {
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

/// Scalar `ClassifyLine2`. Writes per-endpoint sides into `sidev` and returns the
/// overall classification. Each `sidev` entry is one of `-1` (in front), `0` (on the
/// line), `+1` (behind).
pub fn classify_line(node: &Node, v1: &Vertex, v2: &Vertex, sidev: &mut [i32; 2]) -> Side {
    let d_x1 = node.x as f64;
    let d_y1 = node.y as f64;
    let d_dx = node.dx as f64;
    let d_dy = node.dy as f64;
    let d_xv1 = v1.x as f64;
    let d_xv2 = v2.x as f64;
    let d_yv1 = v1.y as f64;
    let d_yv2 = v2.y as f64;

    let s_num1 = (d_y1 - d_yv1) * d_dx - (d_x1 - d_xv1) * d_dy;
    let s_num2 = (d_y1 - d_yv2) * d_dx - (d_x1 - d_xv2) * d_dy;

    let nears: i32;

    if s_num1 <= -FAR_ENOUGH {
        if s_num2 <= -FAR_ENOUGH {
            sidev[0] = 1;
            sidev[1] = 1;
            return Side::Back;
        }
        if s_num2 >= FAR_ENOUGH {
            sidev[0] = 1;
            sidev[1] = -1;
            return Side::Cross;
        }
        nears = 1;
    } else if s_num1 >= FAR_ENOUGH {
        if s_num2 >= FAR_ENOUGH {
            sidev[0] = -1;
            sidev[1] = -1;
            return Side::Front;
        }
        if s_num2 <= -FAR_ENOUGH {
            sidev[0] = -1;
            sidev[1] = 1;
            return Side::Cross;
        }
        nears = 1;
    } else {
        nears = 2 | i32::from(s_num2.abs() < FAR_ENOUGH);
    }

    if nears != 0 {
        let l = 1.0 / (d_dx * d_dx + d_dy * d_dy);
        if nears & 2 != 0 {
            let dist = s_num1 * s_num1 * l;
            sidev[0] = if dist < SIDE_EPSILON * SIDE_EPSILON {
                0
            } else if s_num1 > 0.0 {
                -1
            } else {
                1
            };
        } else {
            sidev[0] = if s_num1 > 0.0 { -1 } else { 1 };
        }
        if nears & 1 != 0 {
            let dist = s_num2 * s_num2 * l;
            sidev[1] = if dist < SIDE_EPSILON * SIDE_EPSILON {
                0
            } else if s_num2 > 0.0 {
                -1
            } else {
                1
            };
        } else {
            sidev[1] = if s_num2 > 0.0 { -1 } else { 1 };
        }
    } else {
        sidev[0] = if s_num1 > 0.0 { -1 } else { 1 };
        sidev[1] = if s_num2 > 0.0 { -1 } else { 1 };
    }

    if sidev[0] | sidev[1] == 0 {
        // Seg is coplanar with the splitter. Use seg direction vs splitter direction.
        if node.dx != 0 {
            if (node.dx > 0 && v2.x > v1.x) || (node.dx < 0 && v2.x < v1.x) {
                Side::Front
            } else {
                Side::Back
            }
        } else if (node.dy > 0 && v2.y > v1.y) || (node.dy < 0 && v2.y < v1.y) {
            Side::Front
        } else {
            Side::Back
        }
    } else if sidev[0] <= 0 && sidev[1] <= 0 {
        Side::Front
    } else if sidev[0] >= 0 && sidev[1] >= 0 {
        Side::Back
    } else {
        Side::Cross
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seg_clearly_in_front_returns_front() {
        // Splitter: y=0 line, +x direction.
        let node = Node {
            x: 0,
            y: 0,
            dx: 1 << 20,
            dy: 0,
            ..Default::default()
        };
        // ZDBSP's PointOnSide returns `-1` when `s_num > 0`, i.e. when the point is on
        // the *right* side of the splitter direction. With splitter pointing +x,
        // negative-y points are in front; positive-y points are behind.
        let v1 = Vertex {
            x: 0,
            y: -(1 << 20),
        };
        let v2 = Vertex {
            x: 1 << 20,
            y: -(1 << 20),
        };
        let mut sidev = [0i32; 2];
        let side = classify_line(&node, &v1, &v2, &mut sidev);
        assert_eq!(side, Side::Front);
        assert_eq!(sidev, [-1, -1]);
    }

    #[test]
    fn seg_crossing_splitter_returns_cross() {
        let node = Node {
            x: 0,
            y: 0,
            dx: 1 << 20,
            dy: 0,
            ..Default::default()
        };
        let v1 = Vertex {
            x: 1 << 19,
            y: -(1 << 20),
        };
        let v2 = Vertex {
            x: 1 << 19,
            y: 1 << 20,
        };
        let mut sidev = [0i32; 2];
        let side = classify_line(&node, &v1, &v2, &mut sidev);
        assert_eq!(side, Side::Cross);
    }
}
