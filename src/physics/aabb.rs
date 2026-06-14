//! Axis-aligned bounding boxes and sweep-collision offset computations.
//! Port of typecraft's `physics/aabb.ts` (here `AABB` is `Copy`, so the
//! mutating helpers return new boxes).

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min_x: f64,
    pub min_y: f64,
    pub min_z: f64,
    pub max_x: f64,
    pub max_y: f64,
    pub max_z: f64,
}

impl Aabb {
    pub fn new(x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> Aabb {
        Aabb {
            min_x: x0,
            min_y: y0,
            min_z: z0,
            max_x: x1,
            max_y: y1,
            max_z: z1,
        }
    }

    /// Expand toward the offset direction (for sweep tests).
    pub fn extend(mut self, dx: f64, dy: f64, dz: f64) -> Aabb {
        if dx < 0.0 {
            self.min_x += dx;
        } else {
            self.max_x += dx;
        }
        if dy < 0.0 {
            self.min_y += dy;
        } else {
            self.max_y += dy;
        }
        if dz < 0.0 {
            self.min_z += dz;
        } else {
            self.max_z += dz;
        }
        self
    }

    /// Shrink inward symmetrically.
    pub fn contract(mut self, x: f64, y: f64, z: f64) -> Aabb {
        self.min_x += x;
        self.min_y += y;
        self.min_z += z;
        self.max_x -= x;
        self.max_y -= y;
        self.max_z -= z;
        self
    }

    /// Translate by offset.
    pub fn offset(mut self, x: f64, y: f64, z: f64) -> Aabb {
        self.min_x += x;
        self.min_y += y;
        self.min_z += z;
        self.max_x += x;
        self.max_y += y;
        self.max_z += z;
        self
    }

    pub fn intersects(self, b: Aabb) -> bool {
        self.min_x < b.max_x
            && self.max_x > b.min_x
            && self.min_y < b.max_y
            && self.max_y > b.min_y
            && self.min_z < b.max_z
            && self.max_z > b.min_z
    }
}

/// Sweep-collision offset along one axis: if `other` overlaps `bb` on the two
/// *perpendicular* axes (`$p1`,`$p2`) and lies ahead of it along the swept axis
/// (`$ax`), clamp `offset` to just touch. `$ax`/`$p1`/`$p2` name the min/max field
/// pairs; the body is identical per axis (collision-critical — covered by tests).
macro_rules! compute_offset_axis {
    ($name:ident, $off:ident, ($amin:ident,$amax:ident), ($p1min:ident,$p1max:ident), ($p2min:ident,$p2max:ident)) => {
        #[inline]
        pub fn $name(bb: Aabb, other: Aabb, $off: f64) -> f64 {
            if other.$p1max > bb.$p1min
                && other.$p1min < bb.$p1max
                && other.$p2max > bb.$p2min
                && other.$p2min < bb.$p2max
            {
                if $off > 0.0 && other.$amax <= bb.$amin {
                    return (bb.$amin - other.$amax).min($off);
                }
                if $off < 0.0 && other.$amin >= bb.$amax {
                    return (bb.$amax - other.$amin).max($off);
                }
            }
            $off
        }
    };
}
compute_offset_axis!(compute_offset_x, offset_x, (min_x, max_x), (min_y, max_y), (min_z, max_z));
compute_offset_axis!(compute_offset_y, offset_y, (min_y, max_y), (min_x, max_x), (min_z, max_z));
compute_offset_axis!(compute_offset_z, offset_z, (min_z, max_z), (min_x, max_x), (min_y, max_y));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersects_and_offset() {
        let a = Aabb::new(0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
        let b = Aabb::new(0.5, 0.5, 0.5, 1.5, 1.5, 1.5);
        assert!(a.intersects(b));
        assert!(!a.intersects(b.offset(5.0, 0.0, 0.0)));
    }

    #[test]
    fn offset_y_stops_at_floor() {
        let player = Aabb::new(0.0, 1.0, 0.0, 1.0, 2.8, 1.0);
        let floor = Aabb::new(0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
        // moving down by -2 collides with floor top at y=1 → offset clamped to -0... actually 0
        let dy = compute_offset_y(floor, player, -2.0);
        assert_eq!(dy, 0.0);
    }
}
