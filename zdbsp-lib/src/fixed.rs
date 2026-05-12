// ABOUTME: 16.16 fixed-point arithmetic. Mirrors the C++ `fixed_t` typedef (which is `int`)
// ABOUTME: and the FRACBITS / scaled-multiplication helpers from zdbsp.h.

/// Number of fractional bits in a `Fixed` value.
pub const FRACBITS: u32 = 16;

/// Value of 1.0 in fixed-point.
pub const FRACUNIT: i32 = 1 << FRACBITS;

/// 16.16 fixed-point scalar. Aliased to `i32` so that arithmetic matches the C++
/// `fixed_t` (which is `int`) bit-for-bit. The wrapping semantics of integer overflow
/// are explicit when needed via `wrapping_*` ops.
pub type Fixed = i32;

/// Convert a 16-bit map-unit coordinate (as stored on disk in Doom-format maps) into
/// 16.16 fixed-point world coordinates. Matches `LittleShort(x) << FRACBITS`.
#[inline]
pub fn from_map_unit(coord: i16) -> Fixed {
    (coord as i32) << FRACBITS
}

/// Doom Binary Angular Measurement (BAM). Full circle = 2^32.
pub type Angle = u32;

/// Direct port of `PointToAngle` from main.cpp:686.
///
/// `atan2(y, x) * 2^30 / pi`, truncated to `i32`, reinterpreted as `u32`, then shifted
/// left one bit. The intermediate `f64 → i32` cast is safe because `|dbam| <= 2^30 < 2^31`.
#[inline]
pub fn point_to_angle(x: Fixed, y: Fixed) -> Angle {
    let ang = (y as f64).atan2(x as f64);
    let rad2bam = (1i64 << 30) as f64 / std::f64::consts::PI;
    let dbam = ang * rad2bam;
    ((dbam as i32) as u32) << 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_map_unit_preserves_sign() {
        assert_eq!(from_map_unit(0), 0);
        assert_eq!(from_map_unit(1), FRACUNIT);
        assert_eq!(from_map_unit(-1), -FRACUNIT);
        assert_eq!(from_map_unit(i16::MIN), (i16::MIN as i32) << FRACBITS);
        assert_eq!(from_map_unit(i16::MAX), (i16::MAX as i32) << FRACBITS);
    }
}
