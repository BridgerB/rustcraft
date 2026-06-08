//! Goal implementations for A* pathfinding. Port of typecraft's `path/goals.ts`.

use crate::vec3::vec3;
use crate::world::{raycast, World};

use super::types::Goal;

/// Octile distance on XZ + manhattan on Y (admissible).
fn octile(dx: i32, dy: i32, dz: i32) -> f64 {
    let adx = dx.abs();
    let adz = dz.abs();
    (adx - adz).abs() as f64 + adx.min(adz) as f64 * std::f64::consts::SQRT_2 + dy.abs() as f64
}

/// Stand at an exact block position.
pub struct GoalBlock {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}
impl GoalBlock {
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        GoalBlock {
            x: x.floor() as i32,
            y: y.floor() as i32,
            z: z.floor() as i32,
        }
    }
}
impl Goal for GoalBlock {
    fn heuristic(&self, x: i32, y: i32, z: i32) -> f64 {
        octile(self.x - x, self.y - y, self.z - z)
    }
    fn is_end(&self, x: i32, y: i32, z: i32) -> bool {
        x == self.x && y == self.y && z == self.z
    }
}

/// Within range of a static position.
pub struct GoalNear {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub range_sq: f64,
}
impl GoalNear {
    pub fn new(x: f64, y: f64, z: f64, range: f64) -> Self {
        GoalNear {
            x: x.floor() as i32,
            y: y.floor() as i32,
            z: z.floor() as i32,
            range_sq: range * range,
        }
    }
}
impl Goal for GoalNear {
    fn heuristic(&self, x: i32, y: i32, z: i32) -> f64 {
        octile(self.x - x, self.y - y, self.z - z)
    }
    fn is_end(&self, x: i32, y: i32, z: i32) -> bool {
        let (dx, dy, dz) = (self.x - x, self.y - y, self.z - z);
        (dx * dx + dy * dy + dz * dz) as f64 <= self.range_sq
    }
}

/// Reach specific X,Z (any Y).
pub struct GoalXZ {
    pub x: i32,
    pub z: i32,
}
impl GoalXZ {
    pub fn new(x: f64, z: f64) -> Self {
        GoalXZ {
            x: x.floor() as i32,
            z: z.floor() as i32,
        }
    }
}
impl Goal for GoalXZ {
    fn heuristic(&self, x: i32, _y: i32, z: i32) -> f64 {
        octile(self.x - x, 0, self.z - z)
    }
    fn is_end(&self, x: i32, _y: i32, z: i32) -> bool {
        x == self.x && z == self.z
    }
}

/// Within range of X,Z (any Y).
pub struct GoalNearXZ {
    pub x: i32,
    pub z: i32,
    pub range_sq: f64,
}
impl GoalNearXZ {
    pub fn new(x: f64, z: f64, range: f64) -> Self {
        GoalNearXZ {
            x: x.floor() as i32,
            z: z.floor() as i32,
            range_sq: range * range,
        }
    }
}
impl Goal for GoalNearXZ {
    fn heuristic(&self, x: i32, _y: i32, z: i32) -> f64 {
        octile(self.x - x, 0, self.z - z)
    }
    fn is_end(&self, x: i32, _y: i32, z: i32) -> bool {
        let (dx, dz) = (self.x - x, self.z - z);
        (dx * dx + dz * dz) as f64 <= self.range_sq
    }
}

/// Reach a specific Y level (any X,Z).
pub struct GoalY {
    pub y: i32,
}
impl GoalY {
    pub fn new(y: f64) -> Self {
        GoalY {
            y: y.floor() as i32,
        }
    }
}
impl Goal for GoalY {
    fn heuristic(&self, _x: i32, y: i32, _z: i32) -> f64 {
        (self.y - y).abs() as f64
    }
    fn is_end(&self, _x: i32, y: i32, _z: i32) -> bool {
        y == self.y
    }
}

/// Stand adjacent to a block (manhattan distance 1).
pub struct GoalGetToBlock {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}
impl GoalGetToBlock {
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        GoalGetToBlock {
            x: x.floor() as i32,
            y: y.floor() as i32,
            z: z.floor() as i32,
        }
    }
}
impl Goal for GoalGetToBlock {
    fn heuristic(&self, x: i32, y: i32, z: i32) -> f64 {
        octile((x - self.x).abs(), (y - self.y).abs(), (z - self.z).abs()) - 1.0
    }
    fn is_end(&self, x: i32, y: i32, z: i32) -> bool {
        (x - self.x).abs() + (y - self.y).abs() + (z - self.z).abs() == 1
    }
}

/// OR over sub-goals.
pub struct GoalCompositeAny {
    pub goals: Vec<Box<dyn Goal>>,
}
impl Goal for GoalCompositeAny {
    fn heuristic(&self, x: i32, y: i32, z: i32) -> f64 {
        self.goals
            .iter()
            .map(|g| g.heuristic(x, y, z))
            .fold(f64::INFINITY, f64::min)
    }
    fn is_end(&self, x: i32, y: i32, z: i32) -> bool {
        self.goals.iter().any(|g| g.is_end(x, y, z))
    }
    fn is_valid(&self) -> bool {
        self.goals.iter().any(|g| g.is_valid())
    }
}

/// AND over sub-goals.
pub struct GoalCompositeAll {
    pub goals: Vec<Box<dyn Goal>>,
}
impl Goal for GoalCompositeAll {
    fn heuristic(&self, x: i32, y: i32, z: i32) -> f64 {
        self.goals
            .iter()
            .map(|g| g.heuristic(x, y, z))
            .fold(f64::NEG_INFINITY, f64::max)
    }
    fn is_end(&self, x: i32, y: i32, z: i32) -> bool {
        self.goals.iter().all(|g| g.is_end(x, y, z))
    }
    fn is_valid(&self) -> bool {
        self.goals.iter().all(|g| g.is_valid())
    }
}

/// Path away from a goal.
pub struct GoalInvert {
    pub goal: Box<dyn Goal>,
}
impl Goal for GoalInvert {
    fn heuristic(&self, x: i32, y: i32, z: i32) -> f64 {
        -self.goal.heuristic(x, y, z)
    }
    fn is_end(&self, x: i32, y: i32, z: i32) -> bool {
        !self.goal.is_end(x, y, z)
    }
}

/// Position from which a block is visible and within reach (break/interact).
pub struct GoalLookAtBlock<'a> {
    x: i32,
    y: i32,
    z: i32,
    world: &'a World<'a>,
    reach: f64,
    entity_height: f64,
}
impl<'a> GoalLookAtBlock<'a> {
    pub fn new(
        x: f64,
        y: f64,
        z: f64,
        world: &'a World<'a>,
        reach: f64,
        entity_height: f64,
    ) -> Self {
        GoalLookAtBlock {
            x: x.floor() as i32,
            y: y.floor() as i32,
            z: z.floor() as i32,
            world,
            reach,
            entity_height,
        }
    }
}
impl Goal for GoalLookAtBlock<'_> {
    fn heuristic(&self, x: i32, y: i32, z: i32) -> f64 {
        octile((x - self.x).abs(), (y - self.y).abs(), (z - self.z).abs()) - 1.0
    }
    fn is_end(&self, x: i32, y: i32, z: i32) -> bool {
        let eye = vec3(
            x as f64 + 0.5,
            y as f64 + self.entity_height,
            z as f64 + 0.5,
        );
        let center = vec3(
            self.x as f64 + 0.5,
            self.y as f64 + 0.5,
            self.z as f64 + 0.5,
        );
        let d = center.subtract(eye);
        if d.dot(d) > self.reach * self.reach {
            return false;
        }
        let pd = eye.subtract(center);
        let mut faces = Vec::new();
        if pd.x.abs() > 0.5 {
            faces.push(vec3(pd.x.signum() * 0.5, 0.0, 0.0));
        }
        if pd.y.abs() > 0.5 {
            faces.push(vec3(0.0, pd.y.signum() * 0.5, 0.0));
        }
        if pd.z.abs() > 0.5 {
            faces.push(vec3(0.0, 0.0, pd.z.signum() * 0.5));
        }
        if faces.is_empty() {
            return true;
        }
        for face in faces {
            let target = center.add(face);
            let ray = target.subtract(eye);
            let len = ray.length();
            if len == 0.0 {
                continue;
            }
            let dir = ray.scale(1.0 / len);
            if let Some(hit) = raycast(self.world, eye, dir, self.reach, None) {
                if hit.position.x as i32 == self.x
                    && hit.position.y as i32 == self.y
                    && hit.position.z as i32 == self.z
                {
                    return true;
                }
            }
        }
        false
    }
}
