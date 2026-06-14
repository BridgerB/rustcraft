//! 3D vector math.
//!
//! Port of typecraft's `vec3` module. Where typecraft exposes free functions
//! over an immutable `{x, y, z}` record, this exposes inherent methods plus
//! operator overloads on a `Copy` struct. The free-function constructors
//! [`vec3`] and the [`ZERO`] constant are kept for parity.

use std::fmt;
use std::ops::{Add, Div, Mul, Sub};
use std::str::FromStr;

/// A 3D vector with `f64` components.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// The zero vector.
pub const ZERO: Vec3 = Vec3 {
    x: 0.0,
    y: 0.0,
    z: 0.0,
};

/// Construct a vector from components. Mirrors typecraft's `vec3(x, y, z)`.
pub const fn vec3(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

impl Vec3 {
    // ─── Construction ──────────────────────────────────────────────────────

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub const fn from_array(arr: [f64; 3]) -> Self {
        Self {
            x: arr[0],
            y: arr[1],
            z: arr[2],
        }
    }

    // ─── Arithmetic ────────────────────────────────────────────────────────

    pub fn add(self, other: Vec3) -> Vec3 {
        vec3(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    pub fn subtract(self, other: Vec3) -> Vec3 {
        vec3(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    pub fn multiply(self, other: Vec3) -> Vec3 {
        vec3(self.x * other.x, self.y * other.y, self.z * other.z)
    }

    pub fn divide(self, other: Vec3) -> Vec3 {
        vec3(self.x / other.x, self.y / other.y, self.z / other.z)
    }

    pub fn scale(self, scalar: f64) -> Vec3 {
        vec3(self.x * scalar, self.y * scalar, self.z * scalar)
    }

    pub fn offset(self, dx: f64, dy: f64, dz: f64) -> Vec3 {
        vec3(self.x + dx, self.y + dy, self.z + dz)
    }

    // ─── Rounding ──────────────────────────────────────────────────────────

    pub fn floor(self) -> Vec3 {
        vec3(self.x.floor(), self.y.floor(), self.z.floor())
    }

    pub fn round(self) -> Vec3 {
        // Match JS Math.round: half rounds toward +infinity, not away from zero.
        vec3(js_round(self.x), js_round(self.y), js_round(self.z))
    }

    pub fn abs(self) -> Vec3 {
        vec3(self.x.abs(), self.y.abs(), self.z.abs())
    }

    // ─── Component-wise comparison ─────────────────────────────────────────

    pub fn min(self, other: Vec3) -> Vec3 {
        vec3(
            self.x.min(other.x),
            self.y.min(other.y),
            self.z.min(other.z),
        )
    }

    pub fn max(self, other: Vec3) -> Vec3 {
        vec3(
            self.x.max(other.x),
            self.y.max(other.y),
            self.z.max(other.z),
        )
    }

    // ─── Modulus ───────────────────────────────────────────────────────────

    pub fn euclidean_mod(self, other: Vec3) -> Vec3 {
        vec3(
            scalar_euclidean_mod(self.x, other.x),
            scalar_euclidean_mod(self.y, other.y),
            scalar_euclidean_mod(self.z, other.z),
        )
    }

    // ─── Vector operations ─────────────────────────────────────────────────

    pub fn dot(self, other: Vec3) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Vec3) -> Vec3 {
        vec3(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    pub fn normalize(self) -> Vec3 {
        let len = self.length();
        if len == 0.0 {
            vec3(0.0, 0.0, 0.0)
        } else {
            self.scale(1.0 / len)
        }
    }

    // ─── Distances ─────────────────────────────────────────────────────────

    pub fn distance(self, other: Vec3) -> f64 {
        self.distance_squared(other).sqrt()
    }

    pub fn distance_squared(self, other: Vec3) -> f64 {
        let dx = other.x - self.x;
        let dy = other.y - self.y;
        let dz = other.z - self.z;
        dx * dx + dy * dy + dz * dz
    }

    pub fn distance_xy(self, other: Vec3) -> f64 {
        let dx = other.x - self.x;
        let dy = other.y - self.y;
        (dx * dx + dy * dy).sqrt()
    }

    pub fn distance_xz(self, other: Vec3) -> f64 {
        let dx = other.x - self.x;
        let dz = other.z - self.z;
        (dx * dx + dz * dz).sqrt()
    }

    pub fn distance_yz(self, other: Vec3) -> f64 {
        let dy = other.y - self.y;
        let dz = other.z - self.z;
        (dy * dy + dz * dz).sqrt()
    }

    pub fn manhattan_distance(self, other: Vec3) -> f64 {
        (other.x - self.x).abs() + (other.y - self.y).abs() + (other.z - self.z).abs()
    }

    // ─── Scalar queries ────────────────────────────────────────────────────

    pub fn volume(self) -> f64 {
        self.x * self.y * self.z
    }

    pub fn is_zero(self) -> bool {
        self.x == 0.0 && self.y == 0.0 && self.z == 0.0
    }

    pub fn equals(self, other: Vec3, tolerance: f64) -> bool {
        (self.x - other.x).abs() <= tolerance
            && (self.y - other.y).abs() <= tolerance
            && (self.z - other.z).abs() <= tolerance
    }

    pub fn component(self, index: usize) -> f64 {
        [self.x, self.y, self.z][index]
    }

    // ─── Conversions ───────────────────────────────────────────────────────

    pub fn to_array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    pub fn to_xz(self) -> [f64; 2] {
        [self.x, self.z]
    }

    pub fn to_xy(self) -> [f64; 2] {
        [self.x, self.y]
    }

    pub fn to_yz(self) -> [f64; 2] {
        [self.y, self.z]
    }

    pub fn swap_yz(self) -> Vec3 {
        vec3(self.x, self.z, self.y)
    }
}

/// Euclidean modulo: result always has the sign of the denominator.
pub fn scalar_euclidean_mod(numerator: f64, denominator: f64) -> f64 {
    let result = numerator % denominator;
    if result < 0.0 {
        result + denominator
    } else {
        result
    }
}

/// JS `Math.round` semantics: round half toward positive infinity.
fn js_round(n: f64) -> f64 {
    (n + 0.5).floor()
}

// ─── Operator overloads ──────────────────────────────────────────────────────

/// Component-wise `Vec3 op Vec3` operator, delegating to the named inherent method
/// (which carries the actual semantics + tests). One arm per operator.
macro_rules! impl_vec3_binop {
    ($trait:ident, $op:ident, $inherent:ident) => {
        impl $trait for Vec3 {
            type Output = Vec3;
            fn $op(self, rhs: Vec3) -> Vec3 {
                Vec3::$inherent(self, rhs)
            }
        }
    };
}
impl_vec3_binop!(Add, add, add);
impl_vec3_binop!(Sub, sub, subtract);
impl_vec3_binop!(Mul, mul, multiply);
impl_vec3_binop!(Div, div, divide);

impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, rhs: f64) -> Vec3 {
        self.scale(rhs)
    }
}

// ─── Display / parse ─────────────────────────────────────────────────────────

/// Matches typecraft's `formatVec3`: `(x, y, z)`.
impl fmt::Display for Vec3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {}, {})", self.x, self.y, self.z)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseVec3Error(String);

impl fmt::Display for ParseVec3Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "vec3: cannot parse: {}", self.0)
    }
}

impl std::error::Error for ParseVec3Error {}

/// Parses `(x, y, z)`, mirroring typecraft's `vec3FromString`.
impl FromStr for Vec3 {
    type Err = ParseVec3Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parse_err = || ParseVec3Error(s.to_string());
        let inner = s
            .trim()
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .ok_or_else(parse_err)?;
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() != 3 {
            return Err(parse_err());
        }
        let x = parts[0].parse::<f64>().map_err(|_| parse_err())?;
        let y = parts[1].parse::<f64>().map_err(|_| parse_err())?;
        let z = parts[2].parse::<f64>().map_err(|_| parse_err())?;
        Ok(vec3(x, y, z))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert_eq!((a * 100000.0).round(), (b * 100000.0).round());
    }

    // ─── construction ───────────────────────────────────────────────────────

    #[test]
    fn creates_from_xyz() {
        let v = vec3(-1.0, 5.0, 10.1);
        assert_eq!(v.x, -1.0);
        assert_eq!(v.y, 5.0);
        assert_eq!(v.z, 10.1);
    }

    #[test]
    fn creates_from_array() {
        assert_eq!(Vec3::from_array([4.0, 5.0, 6.0]), vec3(4.0, 5.0, 6.0));
    }

    #[test]
    fn parses_from_string() {
        let v: Vec3 = "(1, -3.5, 0)".parse().unwrap();
        assert_eq!(v, vec3(1.0, -3.5, 0.0));
    }

    #[test]
    fn roundtrips_through_to_string() {
        let original = vec3(1.0, -3.5, 0.0);
        assert_eq!(original.to_string().parse::<Vec3>().unwrap(), original);
    }

    #[test]
    fn roundtrips_large_values() {
        let original = vec3(-111.0, 222.0, 9876543210.12345);
        assert_eq!(original.to_string().parse::<Vec3>().unwrap(), original);
    }

    #[test]
    fn errors_on_unparseable() {
        assert!("lol hax".parse::<Vec3>().is_err());
    }

    #[test]
    fn zero_is_zero_vector() {
        assert_eq!(ZERO, vec3(0.0, 0.0, 0.0));
    }

    // ─── arithmetic ─────────────────────────────────────────────────────────

    #[test]
    fn adds() {
        assert_eq!(
            vec3(1.0, 2.0, 3.0) + vec3(-1.0, 0.0, 1.0),
            vec3(0.0, 2.0, 4.0)
        );
    }

    #[test]
    fn subtracts() {
        assert_eq!(
            vec3(1.0, 2.0, 3.0) - vec3(-1.0, 0.0, 1.0),
            vec3(2.0, 2.0, 2.0)
        );
    }

    #[test]
    fn multiplies_componentwise() {
        assert_eq!(
            vec3(1.0, 2.0, 3.0) * vec3(-1.0, -2.0, -5.0),
            vec3(-1.0, -4.0, -15.0)
        );
    }

    #[test]
    fn divides_componentwise() {
        assert_eq!(
            vec3(10.0, 20.0, 30.0) / vec3(2.0, 5.0, 3.0),
            vec3(5.0, 4.0, 10.0)
        );
    }

    #[test]
    fn scales() {
        assert_eq!(vec3(1.0, 2.0, 3.0).scale(2.0), vec3(2.0, 4.0, 6.0));
        assert_eq!(vec3(1.0, 2.0, 3.0) * 2.0, vec3(2.0, 4.0, 6.0));
    }

    #[test]
    fn offsets() {
        assert_eq!(
            vec3(1.0, 2.0, 3.0).offset(10.0, -10.0, 20.0),
            vec3(11.0, -8.0, 23.0)
        );
    }

    // ─── rounding ───────────────────────────────────────────────────────────

    #[test]
    fn rounds() {
        assert_eq!(vec3(1.1, -1.5, 1.9).round(), vec3(1.0, -1.0, 2.0));
    }

    #[test]
    fn floors() {
        assert_eq!(vec3(1.1, -1.5, 1.9).floor(), vec3(1.0, -2.0, 1.0));
    }

    #[test]
    fn computes_abs() {
        assert_eq!(vec3(1.1, -1.5, 1.9).abs(), vec3(1.1, 1.5, 1.9));
    }

    // ─── vector operations ──────────────────────────────────────────────────

    #[test]
    fn computes_length() {
        approx(vec3(-10.0, 0.0, 10.0).length(), 14.1421356237);
    }

    #[test]
    fn computes_dot() {
        assert_eq!(vec3(-1.0, -1.0, -1.0).dot(vec3(1.0, 1.0, 1.0)), -3.0);
    }

    #[test]
    fn computes_cross() {
        assert_eq!(
            vec3(1.0, 0.0, 0.0).cross(vec3(0.0, 1.0, 0.0)),
            vec3(0.0, 0.0, 1.0)
        );
    }

    #[test]
    fn normalizes() {
        let r = vec3(10.0, -10.0, 1.1).normalize();
        approx(r.x, 0.7049774402);
        approx(r.y, -0.7049774402);
        approx(r.z, 0.07754751842);
    }

    #[test]
    fn normalizes_zero_to_zero() {
        assert_eq!(ZERO.normalize(), ZERO);
    }

    // ─── distances ──────────────────────────────────────────────────────────

    #[test]
    fn computes_distance() {
        let a = vec3(1.0, 1.0, 1.0);
        let b = vec3(2.0, 2.0, 2.0);
        assert_eq!(a.distance(b), b.distance(a));
        approx(a.distance(b), 1.7320508075688772);
    }

    #[test]
    fn computes_distance_squared() {
        let a = vec3(1.0, 1.0, 1.0);
        let b = vec3(2.0, 2.0, 2.0);
        assert_eq!(a.distance_squared(b), b.distance_squared(a));
        assert_eq!(a.distance_squared(b), 3.0);
    }

    #[test]
    fn computes_planar_distances() {
        let a = vec3(1.0, 1.0, 1.0);
        let b = vec3(2.0, 2.0, 2.0);
        approx(a.distance_xy(b), std::f64::consts::SQRT_2);
        approx(a.distance_xz(b), std::f64::consts::SQRT_2);
        approx(a.distance_yz(b), std::f64::consts::SQRT_2);
    }

    #[test]
    fn computes_manhattan() {
        let a = vec3(-1.0, 0.0, 1.0);
        let b = vec3(10.0, -10.0, 1.1);
        assert_eq!(a.manhattan_distance(b), b.manhattan_distance(a));
        approx(a.manhattan_distance(b), 21.1);
    }

    // ─── comparisons ────────────────────────────────────────────────────────

    #[test]
    fn checks_equality_with_tolerance() {
        let sum = vec3(0.1, 0.0, 0.0) + vec3(0.2, 0.0, 0.0);
        assert!(sum.equals(vec3(0.3, 0.0, 0.0), f64::EPSILON));
    }

    #[test]
    fn checks_zero() {
        assert!(ZERO.is_zero());
        assert!(!vec3(0.0, 1.0, 2.0).is_zero());
    }

    #[test]
    fn componentwise_min_max() {
        assert_eq!(
            vec3(-1.0, 0.0, 1.0).min(vec3(10.0, -10.0, 1.1)),
            vec3(-1.0, -10.0, 1.0)
        );
        assert_eq!(
            vec3(-1.0, 0.0, 1.0).max(vec3(10.0, -10.0, 1.1)),
            vec3(10.0, 0.0, 1.1)
        );
    }

    #[test]
    fn computes_volume() {
        assert_eq!(vec3(3.0, 4.0, 5.0).volume(), 60.0);
    }

    // ─── modulus ────────────────────────────────────────────────────────────

    #[test]
    fn euclidean_mod_componentwise() {
        assert_eq!(
            vec3(12.0, 32.0, -1.0).euclidean_mod(vec3(14.0, 32.0, 16.0)),
            vec3(12.0, 0.0, 15.0)
        );
    }

    #[test]
    fn scalar_euclidean_mod_works() {
        assert_eq!(scalar_euclidean_mod(-1.0, 16.0), 15.0);
        assert_eq!(scalar_euclidean_mod(12.0, 14.0), 12.0);
    }

    // ─── conversions ────────────────────────────────────────────────────────

    #[test]
    fn formats_to_string() {
        assert_eq!(vec3(1.0, -1.0, 3.14).to_string(), "(1, -1, 3.14)");
    }

    #[test]
    fn converts_to_array_and_projections() {
        let v = vec3(0.0, 1.0, 2.0);
        assert_eq!(vec3(1.0, -1.0, 3.14).to_array(), [1.0, -1.0, 3.14]);
        assert_eq!(v.to_xz(), [0.0, 2.0]);
        assert_eq!(v.to_xy(), [0.0, 1.0]);
        assert_eq!(v.to_yz(), [1.0, 2.0]);
        assert_eq!(v.swap_yz(), vec3(0.0, 2.0, 1.0));
    }

    #[test]
    fn accesses_component_by_index() {
        let v = vec3(0.0, 1.0, 2.0);
        assert_eq!(v.component(0), 0.0);
        assert_eq!(v.component(1), 1.0);
        assert_eq!(v.component(2), 2.0);
    }
}
