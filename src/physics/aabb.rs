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

pub fn compute_offset_x(bb: Aabb, other: Aabb, offset_x: f64) -> f64 {
    if other.max_y > bb.min_y
        && other.min_y < bb.max_y
        && other.max_z > bb.min_z
        && other.min_z < bb.max_z
    {
        if offset_x > 0.0 && other.max_x <= bb.min_x {
            return (bb.min_x - other.max_x).min(offset_x);
        }
        if offset_x < 0.0 && other.min_x >= bb.max_x {
            return (bb.max_x - other.min_x).max(offset_x);
        }
    }
    offset_x
}

pub fn compute_offset_y(bb: Aabb, other: Aabb, offset_y: f64) -> f64 {
    if other.max_x > bb.min_x
        && other.min_x < bb.max_x
        && other.max_z > bb.min_z
        && other.min_z < bb.max_z
    {
        if offset_y > 0.0 && other.max_y <= bb.min_y {
            return (bb.min_y - other.max_y).min(offset_y);
        }
        if offset_y < 0.0 && other.min_y >= bb.max_y {
            return (bb.max_y - other.min_y).max(offset_y);
        }
    }
    offset_y
}

pub fn compute_offset_z(bb: Aabb, other: Aabb, offset_z: f64) -> f64 {
    if other.max_x > bb.min_x
        && other.min_x < bb.max_x
        && other.max_y > bb.min_y
        && other.min_y < bb.max_y
    {
        if offset_z > 0.0 && other.max_z <= bb.min_z {
            return (bb.min_z - other.max_z).min(offset_z);
        }
        if offset_z < 0.0 && other.min_z >= bb.max_z {
            return (bb.max_z - other.min_z).max(offset_z);
        }
    }
    offset_z
}

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
