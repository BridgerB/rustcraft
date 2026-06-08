//! Spatial iterators (Manhattan spiral, octahedron, DDA raycast, 2D spiral).
//! Port of typecraft's `world/iterators.ts`.

use crate::vec3::{vec3, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockFace {
    Unknown,
    Bottom,
    Top,
    North,
    South,
    West,
    East,
}

impl BlockFace {
    pub fn id(self) -> i32 {
        match self {
            BlockFace::Unknown => -999,
            BlockFace::Bottom => 0,
            BlockFace::Top => 1,
            BlockFace::North => 2,
            BlockFace::South => 3,
            BlockFace::West => 4,
            BlockFace::East => 5,
        }
    }
}

/// 2D Manhattan spiral around a center, e.g. for loading chunks around a player.
pub struct ManhattanIterator {
    start_x: i32,
    start_z: i32,
    max: i32,
    x: i32,
    y: i32,
    layer: i32,
    leg: i32,
}

pub fn manhattan_iterator(start_x: i32, start_z: i32, max_distance: i32) -> ManhattanIterator {
    ManhattanIterator {
        start_x,
        start_z,
        max: max_distance,
        x: 2,
        y: -1,
        layer: 1,
        leg: -1,
    }
}

impl Iterator for ManhattanIterator {
    type Item = Vec3;
    fn next(&mut self) -> Option<Vec3> {
        if self.leg == -1 {
            self.leg = 0;
            return Some(vec3(self.start_x as f64, 0.0, self.start_z as f64));
        }
        match self.leg {
            0 => {
                if self.max == 1 {
                    return None;
                }
                self.x -= 1;
                self.y += 1;
                if self.x == 0 {
                    self.leg = 1;
                }
            }
            1 => {
                self.x -= 1;
                self.y -= 1;
                if self.y == 0 {
                    self.leg = 2;
                }
            }
            2 => {
                self.x += 1;
                self.y -= 1;
                if self.x == 0 {
                    self.leg = 3;
                }
            }
            _ => {
                self.x += 1;
                self.y += 1;
                if self.y == 0 {
                    self.x += 1;
                    self.leg = 0;
                    self.layer += 1;
                    if self.layer == self.max {
                        return None;
                    }
                }
            }
        }
        Some(vec3(
            (self.start_x + self.x) as f64,
            0.0,
            (self.start_z + self.y) as f64,
        ))
    }
}

/// 3D octahedron expansion from a starting point, e.g. for block search.
pub struct OctahedronIterator {
    s: Vec3,
    max_distance: i32,
    apothem: i32,
    x: i32,
    y: i32,
    z: i32,
    l: i32,
    r: i32,
}

pub fn octahedron_iterator(start: Vec3, max_distance: i32) -> OctahedronIterator {
    let apothem = 1;
    OctahedronIterator {
        s: start.floor(),
        max_distance,
        apothem,
        x: -1,
        y: -1,
        z: -1,
        l: apothem,
        r: apothem + 1,
    }
}

impl Iterator for OctahedronIterator {
    type Item = Vec3;
    fn next(&mut self) -> Option<Vec3> {
        if self.apothem > self.max_distance {
            return None;
        }
        self.r -= 1;
        if self.r < 0 {
            self.l -= 1;
            if self.l < 0 {
                self.z += 2;
                if self.z > 1 {
                    self.y += 2;
                    if self.y > 1 {
                        self.x += 2;
                        if self.x > 1 {
                            self.apothem += 1;
                            self.x = -1;
                        }
                        self.y = -1;
                    }
                    self.z = -1;
                }
                self.l = self.apothem;
            }
            self.r = self.l;
        }
        let big_x = self.x * self.r;
        let big_y = self.y * (self.apothem - self.l);
        let big_z = self.z * (self.apothem - (big_x.abs() + big_y.abs()));
        Some(vec3(
            self.s.x + big_x as f64,
            self.s.y + big_y as f64,
            self.s.z + big_z as f64,
        ))
    }
}

/// A block stepped through by the raycast iterator.
#[derive(Debug, Clone, Copy)]
pub struct RaycastBlock {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub face: BlockFace,
}

#[derive(Debug, Clone, Copy)]
pub struct RaycastHit {
    pub pos: Vec3,
    pub face: BlockFace,
}

/// DDA voxel traversal along a ray, with AABB slab intersection.
pub struct RaycastIterator {
    pos: Vec3,
    dir: Vec3,
    max_distance: f64,
    block: RaycastBlock,
    inv_dir: Vec3,
    step: (i32, i32, i32),
    t_delta: Vec3,
    t_max: Vec3,
}

const MAX_VALUE: f64 = f64::MAX;

fn sign(v: f64) -> i32 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}

pub fn raycast_iterator(pos: Vec3, dir: Vec3, max_distance: f64) -> RaycastIterator {
    let block = RaycastBlock {
        x: pos.x.floor() as i32,
        y: pos.y.floor() as i32,
        z: pos.z.floor() as i32,
        face: BlockFace::Unknown,
    };
    let inv = |d: f64| if d == 0.0 { MAX_VALUE } else { 1.0 / d };
    let t_delta = |d: f64| if d == 0.0 { MAX_VALUE } else { (1.0 / d).abs() };
    let t_max = |bc: i32, p: f64, d: f64| {
        if d == 0.0 {
            MAX_VALUE
        } else {
            ((bc as f64 + if d > 0.0 { 1.0 } else { 0.0 } - p) / d).abs()
        }
    };
    RaycastIterator {
        pos,
        dir,
        max_distance,
        block,
        inv_dir: vec3(inv(dir.x), inv(dir.y), inv(dir.z)),
        step: (sign(dir.x), sign(dir.y), sign(dir.z)),
        t_delta: vec3(t_delta(dir.x), t_delta(dir.y), t_delta(dir.z)),
        t_max: vec3(
            t_max(block.x, pos.x, dir.x),
            t_max(block.y, pos.y, dir.y),
            t_max(block.z, pos.z, dir.z),
        ),
    }
}

impl RaycastIterator {
    /// AABB slab intersection against block collision shapes (each `[x0,y0,z0,x1,y1,z1]`).
    pub fn intersect(&self, shapes: &[Vec<f64>], offset: Vec3) -> Option<RaycastHit> {
        let mut t = MAX_VALUE;
        let mut f = BlockFace::Unknown;
        let p = self.pos.subtract(offset);
        let (idx_x, idx_y, idx_z) = (
            if self.inv_dir.x > 0.0 { (0, 3) } else { (3, 0) },
            if self.inv_dir.y > 0.0 { (1, 4) } else { (4, 1) },
            if self.inv_dir.z > 0.0 { (2, 5) } else { (5, 2) },
        );

        for shape in shapes {
            let mut tmin = (shape[idx_x.0] - p.x) * self.inv_dir.x;
            let mut tmax = (shape[idx_x.1] - p.x) * self.inv_dir.x;
            let tymin = (shape[idx_y.0] - p.y) * self.inv_dir.y;
            let tymax = (shape[idx_y.1] - p.y) * self.inv_dir.y;
            let mut face = if self.step.0 > 0 {
                BlockFace::West
            } else {
                BlockFace::East
            };

            if tmin > tymax || tymin > tmax {
                continue;
            }
            if tymin > tmin {
                tmin = tymin;
                face = if self.step.1 > 0 {
                    BlockFace::Bottom
                } else {
                    BlockFace::Top
                };
            }
            if tymax < tmax {
                tmax = tymax;
            }

            let tzmin = (shape[idx_z.0] - p.z) * self.inv_dir.z;
            let tzmax = (shape[idx_z.1] - p.z) * self.inv_dir.z;
            if tmin > tzmax || tzmin > tmax {
                continue;
            }
            if tzmin > tmin {
                tmin = tzmin;
                face = if self.step.2 > 0 {
                    BlockFace::North
                } else {
                    BlockFace::South
                };
            }
            if tmin < t {
                t = tmin;
                f = face;
            }
        }

        if t == MAX_VALUE {
            None
        } else {
            Some(RaycastHit {
                pos: self.pos.add(self.dir.scale(t)),
                face: f,
            })
        }
    }
}

impl Iterator for RaycastIterator {
    type Item = RaycastBlock;
    fn next(&mut self) -> Option<RaycastBlock> {
        if self.t_max.x.min(self.t_max.y).min(self.t_max.z) > self.max_distance {
            return None;
        }
        if self.t_max.x < self.t_max.y {
            if self.t_max.x < self.t_max.z {
                self.block.x += self.step.0;
                self.t_max.x += self.t_delta.x;
                self.block.face = if self.step.0 > 0 {
                    BlockFace::West
                } else {
                    BlockFace::East
                };
            } else {
                self.block.z += self.step.2;
                self.t_max.z += self.t_delta.z;
                self.block.face = if self.step.2 > 0 {
                    BlockFace::North
                } else {
                    BlockFace::South
                };
            }
        } else if self.t_max.y < self.t_max.z {
            self.block.y += self.step.1;
            self.t_max.y += self.t_delta.y;
            self.block.face = if self.step.1 > 0 {
                BlockFace::Bottom
            } else {
                BlockFace::Top
            };
        } else {
            self.block.z += self.step.2;
            self.t_max.z += self.t_delta.z;
            self.block.face = if self.step.2 > 0 {
                BlockFace::North
            } else {
                BlockFace::South
            };
        }
        Some(self.block)
    }
}

/// 2D outward spiral in growing squares.
pub struct SpiralIterator2d {
    start: Vec3,
    num_points: i64,
    di: i32,
    dj: i32,
    segment_length: i32,
    i: i32,
    j: i32,
    segment_passed: i32,
    k: i64,
}

pub fn spiral_iterator_2d(start: Vec3, max_distance: f64) -> SpiralIterator2d {
    let n = ((max_distance.floor() - 0.5) * 2.0) as i64;
    SpiralIterator2d {
        start,
        num_points: n * n,
        di: 1,
        dj: 0,
        segment_length: 1,
        i: 0,
        j: 0,
        segment_passed: 0,
        k: 0,
    }
}

impl Iterator for SpiralIterator2d {
    type Item = Vec3;
    fn next(&mut self) -> Option<Vec3> {
        if self.k >= self.num_points {
            return None;
        }
        let output = vec3(
            self.start.x + self.i as f64,
            0.0,
            self.start.z + self.j as f64,
        );
        self.i += self.di;
        self.j += self.dj;
        self.segment_passed += 1;
        if self.segment_passed == self.segment_length {
            self.segment_passed = 0;
            let tmp = self.di;
            self.di = -self.dj;
            self.dj = tmp;
            if self.dj == 0 {
                self.segment_length += 1;
            }
        }
        self.k += 1;
        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manhattan_covers_center_first() {
        let mut it = manhattan_iterator(10, 20, 3);
        assert_eq!(it.next(), Some(vec3(10.0, 0.0, 20.0)));
        let count = 1 + it.count();
        // A radius-3 manhattan spiral visits a diamond of points.
        assert!(count > 1);
    }

    #[test]
    fn spiral_visits_center_first() {
        let mut it = spiral_iterator_2d(vec3(0.0, 0.0, 0.0), 3.0);
        assert_eq!(it.next(), Some(vec3(0.0, 0.0, 0.0)));
    }

    #[test]
    fn raycast_steps_along_axis() {
        let mut it = raycast_iterator(vec3(0.5, 0.5, 0.5), vec3(1.0, 0.0, 0.0), 5.0);
        let first = it.next().unwrap();
        assert_eq!((first.x, first.y, first.z), (1, 0, 0));
        assert_eq!(first.face, BlockFace::West);
    }

    #[test]
    fn octahedron_starts_near_center() {
        let mut it = octahedron_iterator(vec3(0.0, 0.0, 0.0), 2);
        let first = it.next().unwrap();
        assert!(first.x.abs() + first.y.abs() + first.z.abs() <= 2.0);
    }
}
