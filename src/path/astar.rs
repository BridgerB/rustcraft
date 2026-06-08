//! Incremental A* with a binary min-heap, arena-allocated nodes (parent links
//! are arena indices). Port of typecraft's `path/astar.ts` + `heap.ts`.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::types::{Goal, Move, PathResult, PathStatus};

/// Generates neighboring moves for a node.
pub trait NeighborGen {
    fn neighbors(&self, node: &Move) -> Vec<Move>;
}

struct PathNode {
    data: Move,
    g: f64,
    h: f64,
    f: f64,
    parent: Option<usize>,
    heap_index: usize,
}

const NONE_IDX: usize = usize::MAX;

pub struct AStar {
    nodes: Vec<PathNode>,
    /// 1-indexed heap of node arena indices (slot 0 is a sentinel).
    heap: Vec<usize>,
    open_map: HashMap<i64, usize>,
    closed: HashSet<i64>,
    best: usize,
    start_time: Instant,
    max_cost: f64,
}

impl AStar {
    pub fn new(start: Move, goal: &dyn Goal, search_radius: f64) -> AStar {
        let h = goal.heuristic(start.x, start.y, start.z);
        let start_node = PathNode {
            data: start.clone(),
            g: 0.0,
            h,
            f: h,
            parent: None,
            heap_index: NONE_IDX,
        };
        let start_hash = start.hash;
        let mut astar = AStar {
            nodes: vec![start_node],
            heap: vec![NONE_IDX], // sentinel at index 0
            open_map: HashMap::new(),
            closed: HashSet::new(),
            best: 0,
            start_time: Instant::now(),
            max_cost: if search_radius < 0.0 {
                -1.0
            } else {
                h + search_radius
            },
        };
        astar.heap_push(0);
        astar.open_map.insert(start_hash, 0);
        astar
    }

    // ── Heap ops (compare by f) ──

    fn heap_is_empty(&self) -> bool {
        self.heap.len() <= 1
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.heap.swap(i, j);
        self.nodes[self.heap[i]].heap_index = i;
        self.nodes[self.heap[j]].heap_index = j;
    }

    fn bubble_up(&mut self, mut i: usize) {
        while i > 1 && self.nodes[self.heap[i >> 1]].f > self.nodes[self.heap[i]].f {
            let parent = i >> 1;
            self.swap(i, parent);
            i = parent;
        }
    }

    fn sift_down(&mut self, mut i: usize) {
        let size = self.heap.len();
        loop {
            let left = i * 2;
            let right = left + 1;
            let mut smallest = i;
            if left < size && self.nodes[self.heap[left]].f < self.nodes[self.heap[smallest]].f {
                smallest = left;
            }
            if right < size && self.nodes[self.heap[right]].f < self.nodes[self.heap[smallest]].f {
                smallest = right;
            }
            if smallest == i {
                break;
            }
            self.swap(i, smallest);
            i = smallest;
        }
    }

    fn heap_push(&mut self, node_idx: usize) {
        self.nodes[node_idx].heap_index = self.heap.len();
        self.heap.push(node_idx);
        self.bubble_up(self.heap.len() - 1);
    }

    fn heap_pop(&mut self) -> usize {
        let min = self.heap[1];
        self.nodes[min].heap_index = NONE_IDX;
        let last = self.heap.pop().unwrap();
        if self.heap.len() > 1 {
            self.heap[1] = last;
            self.nodes[last].heap_index = 1;
            self.sift_down(1);
        }
        min
    }

    fn reconstruct(&self, mut idx: usize) -> Vec<Move> {
        let mut path = Vec::new();
        while let Some(parent) = self.nodes[idx].parent {
            path.push(self.nodes[idx].data.clone());
            idx = parent;
        }
        path.reverse();
        path
    }

    fn result(&self, status: PathStatus, idx: usize) -> PathResult {
        PathResult {
            status,
            cost: self.nodes[idx].g,
            visited_nodes: self.closed.len(),
            generated_nodes: self.closed.len() + self.open_map.len(),
            path: self.reconstruct(idx),
        }
    }

    /// Run one budgeted slice of the search.
    pub fn compute(
        &mut self,
        goal: &dyn Goal,
        movements: &dyn NeighborGen,
        tick_timeout: Duration,
        total_timeout: Duration,
    ) -> PathResult {
        let tick_start = Instant::now();
        while !self.heap_is_empty() {
            if tick_start.elapsed() > tick_timeout {
                return self.result(PathStatus::Partial, self.best);
            }
            if self.start_time.elapsed() > total_timeout {
                return self.result(PathStatus::Timeout, self.best);
            }

            let node_idx = self.heap_pop();
            let (nx, ny, nz, nhash, ng) = {
                let n = &self.nodes[node_idx];
                (n.data.x, n.data.y, n.data.z, n.data.hash, n.g)
            };
            if goal.is_end(nx, ny, nz) {
                return self.result(PathStatus::Success, node_idx);
            }
            self.open_map.remove(&nhash);
            self.closed.insert(nhash);

            let node_data = self.nodes[node_idx].data.clone();
            for neighbor in movements.neighbors(&node_data) {
                if self.closed.contains(&neighbor.hash) {
                    continue;
                }
                let g = ng + neighbor.cost;
                let h = goal.heuristic(neighbor.x, neighbor.y, neighbor.z);
                if self.max_cost > 0.0 && g + h > self.max_cost {
                    continue;
                }

                match self.open_map.get(&neighbor.hash).copied() {
                    Some(existing) => {
                        if self.nodes[existing].g <= g {
                            continue;
                        }
                        let n = &mut self.nodes[existing];
                        n.data = neighbor;
                        n.g = g;
                        n.h = h;
                        n.f = g + h;
                        n.parent = Some(node_idx);
                        if h < self.nodes[self.best].h {
                            self.best = existing;
                        }
                        let hi = self.nodes[existing].heap_index;
                        if hi != NONE_IDX {
                            self.bubble_up(hi);
                        }
                    }
                    None => {
                        let hash = neighbor.hash;
                        self.nodes.push(PathNode {
                            data: neighbor,
                            g,
                            h,
                            f: g + h,
                            parent: Some(node_idx),
                            heap_index: NONE_IDX,
                        });
                        let new_idx = self.nodes.len() - 1;
                        self.open_map.insert(hash, new_idx);
                        if h < self.nodes[self.best].h {
                            self.best = new_idx;
                        }
                        self.heap_push(new_idx);
                    }
                }
            }
        }
        self.result(PathStatus::NoPath, self.best)
    }
}

/// Compute a full path in one call (generous budget).
pub fn compute_path(
    start: Move,
    goal: &dyn Goal,
    movements: &dyn NeighborGen,
    search_radius: f64,
    total_timeout: Duration,
) -> PathResult {
    let mut astar = AStar::new(start, goal, search_radius);
    astar.compute(goal, movements, total_timeout, total_timeout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::types::pos_hash;

    /// A trivial flat-plane neighbor generator: 4-connected grid at y=0.
    struct GridMoves;
    impl NeighborGen for GridMoves {
        fn neighbors(&self, node: &Move) -> Vec<Move> {
            [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .map(|(dx, dz)| {
                    let (x, z) = (node.x + dx, node.z + dz);
                    Move {
                        x,
                        y: 0,
                        z,
                        cost: 1.0,
                        hash: pos_hash(x, 0, z),
                        remaining_blocks: 0,
                        to_break: vec![],
                        to_place: vec![],
                        parkour: false,
                    }
                })
                .collect()
        }
    }

    struct GoalBlock {
        x: i32,
        z: i32,
    }
    impl Goal for GoalBlock {
        fn heuristic(&self, x: i32, _y: i32, z: i32) -> f64 {
            ((self.x - x).abs() + (self.z - z).abs()) as f64
        }
        fn is_end(&self, x: i32, _y: i32, z: i32) -> bool {
            x == self.x && z == self.z
        }
    }

    #[test]
    fn finds_straight_path() {
        let goal = GoalBlock { x: 5, z: 0 };
        let result = compute_path(
            Move::start(0, 0, 0),
            &goal,
            &GridMoves,
            -1.0,
            Duration::from_secs(5),
        );
        assert_eq!(result.status, PathStatus::Success);
        assert_eq!(result.path.len(), 5);
        assert_eq!(result.cost, 5.0);
        let last = result.path.last().unwrap();
        assert_eq!((last.x, last.z), (5, 0));
    }

    #[test]
    fn finds_diagonal_manhattan_path() {
        let goal = GoalBlock { x: 3, z: 4 };
        let result = compute_path(
            Move::start(0, 0, 0),
            &goal,
            &GridMoves,
            -1.0,
            Duration::from_secs(5),
        );
        assert_eq!(result.status, PathStatus::Success);
        assert_eq!(result.cost, 7.0); // 3 + 4 manhattan
    }
}
