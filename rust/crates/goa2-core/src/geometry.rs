use crate::{bitset::BitSet, ids::HexId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

const OFFSETS: [(i32, i32, i32); 6] = [
    (1, 0, -1),
    (1, -1, 0),
    (0, -1, 1),
    (-1, 0, 1),
    (-1, 1, 0),
    (0, 1, -1),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Coordinates")]
pub struct Hex {
    q: i32,
    r: i32,
    s: i32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Coordinates {
    q: i32,
    r: i32,
    s: i32,
}
impl TryFrom<Coordinates> for Hex {
    type Error = String;
    fn try_from(v: Coordinates) -> Result<Self, Self::Error> {
        Self::new(v.q, v.r, v.s)
    }
}
impl Hex {
    pub fn new(q: i32, r: i32, s: i32) -> Result<Self, String> {
        if i64::from(q) + i64::from(r) + i64::from(s) != 0 {
            return Err("cube coordinates must sum to zero".into());
        }
        // Leave headroom for neighboring coordinates; never silently wrap.
        if [q, r, s].iter().any(|v| v.unsigned_abs() > 1_000_000_000) {
            return Err("coordinate exceeds supported range".into());
        }
        Ok(Self { q, r, s })
    }
    pub fn coordinates(self) -> (i32, i32, i32) {
        (self.q, self.r, self.s)
    }
    pub fn straight(self, other: Self) -> bool {
        self.q == other.q || self.r == other.r || self.s == other.s
    }
    pub fn line_to(self, other: Self) -> Option<Vec<Self>> {
        self.line_points(other).map(Iterator::collect)
    }

    pub(crate) fn line_points(self, other: Self) -> Option<impl Iterator<Item = Self>> {
        if !self.straight(other) {
            return None;
        }
        let distance = self.distance(other);
        let divisor = i64::from(distance.max(1));
        let (dq, dr, ds) = (
            (i64::from(other.q) - i64::from(self.q)) / divisor,
            (i64::from(other.r) - i64::from(self.r)) / divisor,
            (i64::from(other.s) - i64::from(self.s)) / divisor,
        );
        Some((1..=distance).map(move |n| Self {
            q: (i64::from(self.q) + dq * i64::from(n)) as i32,
            r: (i64::from(self.r) + dr * i64::from(n)) as i32,
            s: (i64::from(self.s) + ds * i64::from(n)) as i32,
        }))
    }
    pub fn direction_to(self, other: Self) -> Option<(i64, i64, i64)> {
        let distance = self.distance(other);
        if distance == 0 || !self.straight(other) {
            return None;
        }
        Some((
            (i64::from(other.q) - i64::from(self.q)) / i64::from(distance),
            (i64::from(other.r) - i64::from(self.r)) / i64::from(distance),
            (i64::from(other.s) - i64::from(self.s)) / i64::from(distance),
        ))
    }
    pub fn distance(self, other: Self) -> u32 {
        self.q
            .abs_diff(other.q)
            .max(self.r.abs_diff(other.r))
            .max(self.s.abs_diff(other.s))
    }
    pub fn neighbors(self) -> [Self; 6] {
        OFFSETS.map(|(q, r, s)| Self {
            q: self.q + q,
            r: self.r + r,
            s: self.s + s,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileSpec {
    pub hex: Hex,
    pub terrain: bool,
    pub occupied: bool,
}

/// Plain-board rules only. Stateful topology, passable tokens and actor effects
/// must be compiled separately before this primitive can serve a game decision.
#[derive(Clone, Debug)]
pub struct BoardGeometry {
    hexes: Vec<Hex>,
    lookup: HexLookup,
    adjacent: Vec<[Option<HexId>; 6]>,
    terrain: BitSet,
}
#[derive(Clone, Debug)]
enum HexLookup {
    Dense {
        min_q: i32,
        min_r: i32,
        width: usize,
        height: usize,
        cells: Vec<Option<HexId>>,
    },
    Sparse(HashMap<Hex, HexId>),
}

impl HexLookup {
    fn compile(sparse: HashMap<Hex, HexId>) -> Self {
        let Some(&first) = sparse.keys().next() else {
            return Self::Sparse(sparse);
        };
        let (mut min_q, mut max_q, mut min_r, mut max_r) = (first.q, first.q, first.r, first.r);
        for hex in sparse.keys() {
            min_q = min_q.min(hex.q);
            max_q = max_q.max(hex.q);
            min_r = min_r.min(hex.r);
            max_r = max_r.max(hex.r);
        }
        let width = (max_q - min_q) as usize + 1;
        let height = (max_r - min_r) as usize + 1;
        // Bound memory for valid but widely separated coordinates.
        let Some(area) = width
            .checked_mul(height)
            .filter(|&area| area <= sparse.len().saturating_mul(4))
        else {
            return Self::Sparse(sparse);
        };
        let mut cells = vec![None; area];
        for (hex, id) in sparse {
            cells[(hex.r - min_r) as usize * width + (hex.q - min_q) as usize] = Some(id);
        }
        Self::Dense {
            min_q,
            min_r,
            width,
            height,
            cells,
        }
    }

    fn get(&self, hex: Hex) -> Option<HexId> {
        match self {
            Self::Dense {
                min_q,
                min_r,
                width,
                height,
                cells,
            } => {
                let q = (hex.q - min_q) as usize;
                let r = (hex.r - min_r) as usize;
                if q >= *width || r >= *height {
                    return None;
                }
                cells[r * width + q]
            }
            Self::Sparse(cells) => cells.get(&hex).copied(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Occupancy {
    occupied: BitSet,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovementQuery {
    pub start: Hex,
    pub ends: Vec<Hex>,
    pub max_steps: i32,
    pub ignore_obstacles: bool,
    pub pass_through_obstacles: bool,
}
impl BoardGeometry {
    pub fn compile(tiles: &[TileSpec]) -> Result<(Self, Occupancy), String> {
        let len = tiles.len();
        let mut lookup = HashMap::with_capacity(len);
        let mut terrain = BitSet::new(len);
        let mut occupied = BitSet::new(len);
        for (i, tile) in tiles.iter().enumerate() {
            let id = HexId::checked(i, len).ok_or("board too large")?;
            if lookup.insert(tile.hex, id).is_some() {
                return Err("duplicate board coordinate".into());
            }
            if tile.terrain {
                terrain.insert(i);
            }
            if tile.occupied {
                occupied.insert(i);
            }
        }
        let lookup = HexLookup::compile(lookup);
        let hexes: Vec<_> = tiles.iter().map(|t| t.hex).collect();
        let adjacent = hexes
            .iter()
            .map(|h| h.neighbors().map(|n| lookup.get(n)))
            .collect();
        Ok((
            Self {
                hexes,
                lookup,
                adjacent,
                terrain,
            },
            Occupancy { occupied },
        ))
    }
    pub fn id(&self, hex: Hex) -> Option<HexId> {
        self.lookup.get(hex)
    }
    pub(crate) fn neighbor_indices(&self, index: usize) -> [Option<usize>; 6] {
        self.adjacent[index].map(|id| id.map(HexId::index))
    }
    pub(crate) fn obstacle_at(&self, index: usize, occupancy: &Occupancy) -> bool {
        self.terrain.contains(index) || occupancy.occupied.contains(index)
    }
    pub fn terrain(&self, hex: Hex) -> bool {
        self.id(hex)
            .is_some_and(|id| self.terrain.contains(id.index()))
    }
    pub fn obstacle(&self, hex: Hex, occupancy: &Occupancy) -> bool {
        self.id(hex)
            .is_none_or(|id| self.obstacle_at(id.index(), occupancy))
    }
    /// Stateful topology's plain case: intermediate spaces must exist on map,
    /// even when terrain/occupants can be crossed. Landing remains obstacle-free.
    pub fn stateful_paths(&self, occupancy: &Occupancy, query: &MovementQuery) -> Vec<bool> {
        if query.max_steps <= 0 {
            return vec![false; query.ends.len()];
        }
        let mut visited = BitSet::new(self.hexes.len());
        let mut reached = BitSet::new(self.hexes.len());
        let mut terminal = BitSet::new(self.hexes.len());
        let mut queue = VecDeque::from([(query.start, 0)]);
        if let Some(id) = self.id(query.start) {
            visited.insert(id.index());
        }
        while let Some((hex, distance)) = queue.pop_front() {
            if distance >= query.max_steps {
                continue;
            }
            for neighbor in hex.neighbors() {
                let Some(id) = self.id(neighbor) else {
                    continue;
                };
                terminal.insert(id.index());
                if !visited.insert(id.index()) {
                    continue;
                }
                if !query.pass_through_obstacles && self.obstacle(neighbor, occupancy) {
                    continue;
                }
                reached.insert(id.index());
                queue.push_back((neighbor, distance + 1));
            }
        }
        query
            .ends
            .iter()
            .map(|&end| {
                end != query.start
                    && self.id(end).is_some_and(|id| {
                        (reached.contains(id.index())
                            || (query.ignore_obstacles && terminal.contains(id.index())))
                            && (query.ignore_obstacles || !self.obstacle(end, occupancy))
                    })
            })
            .collect()
    }
    pub fn stateful_path(
        &self,
        occupancy: &Occupancy,
        start: Hex,
        end: Hex,
        max_steps: i32,
        through: bool,
    ) -> bool {
        self.stateful_paths(
            occupancy,
            &MovementQuery {
                start,
                ends: vec![end],
                max_steps,
                ignore_obstacles: false,
                pass_through_obstacles: through,
            },
        )[0]
    }
    pub fn set_occupied(
        &self,
        occupancy: &mut Occupancy,
        hex: Hex,
        value: bool,
    ) -> Result<(), String> {
        let id = self.id(hex).ok_or("occupancy location outside board")?;
        if value {
            occupancy.occupied.insert(id.index());
        } else {
            occupancy.occupied.remove(id.index());
        }
        Ok(())
    }
    /// One traversal for all destinations. Occupancy is read anew each call.
    /// This matches the original Python bare-board BFS, including virtual
    /// off-map terrain and an off-map origin. No query cache survives mutation.
    pub fn movement_paths(&self, occupancy: &Occupancy, query: &MovementQuery) -> Vec<bool> {
        if query.max_steps <= 0 {
            return vec![false; query.ends.len()];
        }
        if query.pass_through_obstacles {
            return query
                .ends
                .iter()
                .map(|&end| {
                    end != query.start
                        && query.start.distance(end) <= query.max_steps as u32
                        && (query.ignore_obstacles || !self.obstacle(end, occupancy))
                })
                .collect();
        }
        let mut visited = BitSet::new(self.hexes.len());
        let mut reached = BitSet::new(self.hexes.len());
        let mut terminals = HashSet::new();
        let mut queue = VecDeque::from([(query.start, 0)]);
        if let Some(id) = self.id(query.start) {
            visited.insert(id.index());
        }
        while let Some((current, distance)) = queue.pop_front() {
            if distance >= query.max_steps {
                continue;
            }
            let neighbors = if let Some(id) = self.id(current) {
                self.adjacent[id.index()].map(|id| id.map(|id| self.hexes[id.index()]))
            } else {
                current.neighbors().map(Some)
            };
            // Off-map destinations may be accepted only as a final step, never
            // as intermediate terrain. Include absent neighbors for that case.
            if query.ignore_obstacles {
                terminals.extend(current.neighbors());
            }
            for neighbor in neighbors.into_iter().flatten() {
                if neighbor == query.start {
                    continue;
                }
                let Some(id) = self.id(neighbor) else {
                    continue;
                };
                if !visited.insert(id.index()) {
                    continue;
                }
                if self.obstacle(neighbor, occupancy) {
                    continue;
                }
                reached.insert(id.index());
                queue.push_back((neighbor, distance + 1));
            }
        }
        query
            .ends
            .iter()
            .map(|&end| {
                end != query.start
                    && (self.id(end).is_some_and(|id| reached.contains(id.index()))
                        || (query.ignore_obstacles && terminals.contains(&end)))
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn h(q: i32) -> Hex {
        Hex::new(q, -q, 0).unwrap()
    }
    #[test]
    fn coordinate_lookup_preserves_holes_input_order_and_sparse_bounds() {
        let tiles = |coordinates: &[i32]| {
            coordinates
                .iter()
                .map(|&q| TileSpec {
                    hex: h(q),
                    terrain: false,
                    occupied: false,
                })
                .collect::<Vec<_>>()
        };
        let mut dense_tiles = tiles(&[0, -2, 1]);
        dense_tiles.push(TileSpec {
            hex: Hex::new(0, 1, -1).unwrap(),
            terrain: false,
            occupied: false,
        });
        let (dense, _) = BoardGeometry::compile(&dense_tiles).unwrap();
        let (sparse, _) = BoardGeometry::compile(&tiles(&[1_000_000_000, -1_000_000_000])).unwrap();
        assert!(matches!(dense.lookup, HexLookup::Dense { .. }));
        assert!(matches!(sparse.lookup, HexLookup::Sparse(_)));
        assert_eq!(dense.id(h(-2)).unwrap().index(), 1);
        assert_eq!(dense.id(h(-1)), None);
        assert_eq!(dense.id(h(-3)), None);
        assert_eq!(dense.id(h(2)), None);
        assert_eq!(sparse.id(h(-1_000_000_000)).unwrap().index(), 1);
        assert_eq!(sparse.id(h(0)), None);
        assert!(BoardGeometry::compile(&tiles(&[1, 1])).is_err());
    }

    #[test]
    fn destination_and_intermediate_obstacles_differ_and_mutations_apply() {
        let tiles: Vec<_> = (0..4)
            .map(|q| TileSpec {
                hex: h(q),
                terrain: false,
                occupied: q == 1,
            })
            .collect();
        let (board, mut occ) = BoardGeometry::compile(&tiles).unwrap();
        let mut query = MovementQuery {
            start: h(0),
            ends: vec![h(0), h(1), h(2), h(4)],
            max_steps: 4,
            ignore_obstacles: true,
            pass_through_obstacles: false,
        };
        assert_eq!(
            board.movement_paths(&occ, &query),
            [false, true, false, false]
        );
        query.pass_through_obstacles = true;
        assert_eq!(
            board.movement_paths(&occ, &query),
            [false, true, true, true]
        );
        query.ignore_obstacles = false;
        assert_eq!(
            board.movement_paths(&occ, &query),
            [false, false, true, false]
        );
        board.set_occupied(&mut occ, h(1), false).unwrap();
        query.pass_through_obstacles = false;
        assert_eq!(
            board.movement_paths(&occ, &query),
            [false, true, true, false]
        );
    }
    #[test]
    fn rejects_invalid_geometry_and_duplicate_tiles() {
        assert!(serde_json::from_str::<Hex>(r#"{"q":1,"r":0,"s":0}"#).is_err());
        let tile = TileSpec {
            hex: h(0),
            terrain: false,
            occupied: false,
        };
        assert!(BoardGeometry::compile(&[tile.clone(), tile]).is_err());
    }
    #[test]
    fn zero_range_and_off_map_origin() {
        let (board, occ) = BoardGeometry::compile(&[TileSpec {
            hex: h(0),
            terrain: false,
            occupied: false,
        }])
        .unwrap();
        let mut query = MovementQuery {
            start: h(-1),
            ends: vec![h(0), h(-2)],
            max_steps: 1,
            ignore_obstacles: false,
            pass_through_obstacles: false,
        };
        assert_eq!(board.movement_paths(&occ, &query), [true, false]);
        query.ignore_obstacles = true;
        assert_eq!(board.movement_paths(&occ, &query), [true, true]);
        query.max_steps = 0;
        assert_eq!(board.movement_paths(&occ, &query), [false, false]);
    }
}
