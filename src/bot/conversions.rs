//! Angle/velocity conversions between Notchian protocol values and radians.
//! Port of typecraft's `bot/conversions.ts`.

use crate::vec3::{scalar_euclidean_mod, vec3, Vec3};

const PI: f64 = std::f64::consts::PI;
const PI_2: f64 = std::f64::consts::PI * 2.0;
const TO_RAD: f64 = PI / 180.0;
const TO_DEG: f64 = 180.0 / PI;
const FROM_NOTCH_BYTE: f64 = 360.0 / 256.0;
const FROM_NOTCH_VEL: f64 = 1.0 / 8000.0;

pub fn to_radians(degrees: f64) -> f64 {
    TO_RAD * degrees
}

pub fn to_degrees(radians: f64) -> f64 {
    TO_DEG * radians
}

/// Notchian yaw (degrees, clockwise from south) → radians.
pub fn from_notchian_yaw(yaw: f64) -> f64 {
    scalar_euclidean_mod(PI - to_radians(yaw), PI_2)
}

/// Notchian pitch (degrees) → radians.
pub fn from_notchian_pitch(pitch: f64) -> f64 {
    scalar_euclidean_mod(to_radians(-pitch) + PI, PI_2) - PI
}

/// Radians yaw → Notchian degrees.
pub fn to_notchian_yaw(yaw: f64) -> f64 {
    to_degrees(PI - yaw)
}

/// Radians pitch → Notchian degrees.
pub fn to_notchian_pitch(pitch: f64) -> f64 {
    to_degrees(-pitch)
}

pub fn from_notchian_yaw_byte(yaw: f64) -> f64 {
    from_notchian_yaw(yaw * FROM_NOTCH_BYTE)
}

pub fn from_notchian_pitch_byte(pitch: f64) -> f64 {
    from_notchian_pitch(pitch * FROM_NOTCH_BYTE)
}

/// Notchian velocity (fixed-point 1/8000 blocks/tick) → float Vec3.
pub fn from_notch_velocity(x: f64, y: f64, z: f64) -> Vec3 {
    vec3(x * FROM_NOTCH_VEL, y * FROM_NOTCH_VEL, z * FROM_NOTCH_VEL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn radians_degrees() {
        approx(to_radians(180.0), PI);
        approx(to_degrees(PI), 180.0);
    }

    #[test]
    fn yaw_roundtrip() {
        // to_notchian_yaw is the inverse of from_notchian_yaw (mod 2π).
        for deg in [0.0, 45.0, 90.0, 179.0, 270.0] {
            let rad = from_notchian_yaw(deg);
            let back = scalar_euclidean_mod(to_notchian_yaw(rad), 360.0);
            approx(scalar_euclidean_mod(deg, 360.0), back);
        }
    }

    #[test]
    fn velocity_scaling() {
        let v = from_notch_velocity(8000.0, -8000.0, 4000.0);
        approx(v.x, 1.0);
        approx(v.y, -1.0);
        approx(v.z, 0.5);
    }
}
