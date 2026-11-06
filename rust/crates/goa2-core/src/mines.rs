//! Mine-aware BFS with shared mine sets and parent links instead of copied paths.
use crate::{data::Symbol, geometry::Hex, world::NativeWorld};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

#[derive(Clone, Debug, Hash, PartialEq)]
pub struct MinePath {
    /// Mathematical set in stable entity/supply order, independent of symbol IDs.
    pub mines: Arc<[Symbol]>,
    pub path: Vec<Hex>,
}
struct Node {
    hex: Hex,
    distance: i32,
    mines: Arc<[Symbol]>,
    parent: Option<usize>,
}
impl NativeWorld {
    /// Detection intentionally reads token-pool rows, even if the entity table
    /// has a different copy: that is the source MoveUnit detection boundary.
    pub(crate) fn has_enemy_mine_supply(&self, team: Symbol) -> bool {
        self.definitions
            .mine_supply
            .iter()
            .any(|(id, passable, owner)| {
                *passable
                    && self.position(*id).is_some()
                    && owner.is_some_and(|owner| {
                        owner != self.definitions.effect_empty
                            && self.hero(owner).is_none_or(|hero| hero.team != team)
                    })
            })
    }
    pub(crate) fn has_deployed_passable(&self) -> bool {
        self.definitions
            .entities
            .iter()
            .any(|e| e.passable && self.position(e.id).is_some())
            || self
                .definitions
                .supply_passable
                .iter()
                .any(|(id, passable)| *passable && self.position(*id).is_some())
    }
    fn mine_identity_rank(&self, id: Symbol) -> usize {
        self.entity_index(id).unwrap_or_else(|| {
            self.definitions.entities.len()
                + self
                    .definitions
                    .supply_passable
                    .iter()
                    .position(|(candidate, _)| *candidate == id)
                    .expect("passable mine identity must be in entities or supply")
        })
    }

    /// One traversal for all map destinations; no result survives a state mutation.
    /// Like the frozen mine helper, connectivity is checked per edge, without
    /// the separate original-space restriction of ordinary movement validation.
    pub fn mine_paths(
        &self,
        start: Hex,
        range: i32,
        actor: Option<Symbol>,
        moving_team: Option<Symbol>,
        units: &[Symbol],
    ) -> Result<Vec<Vec<MinePath>>, String> {
        let mut result = vec![Vec::new(); self.tile_hexes().len()];
        if range <= 0 {
            return Ok(result);
        }
        let empty: Arc<[Symbol]> = Arc::from([]);
        let mut visited = HashSet::from([(start, empty.clone())]);
        let mut arena = vec![Node {
            hex: start,
            distance: 0,
            mines: empty,
            parent: None,
        }];
        let mut queue = VecDeque::from([0usize]);
        while let Some(current) = queue.pop_front() {
            let node = &arena[current];
            let (hex, distance, mines) = (node.hex, node.distance, node.mines.clone());
            if hex != start
                && !self.passable_token(hex, &[])
                && let Some(index) = self.tile_index(hex)
            {
                let mut path = Vec::with_capacity(distance as usize + 1);
                let mut cursor = Some(current);
                while let Some(index) = cursor {
                    path.push(arena[index].hex);
                    cursor = arena[index].parent;
                }
                path.reverse();
                result[index].push(MinePath {
                    mines: mines.clone(),
                    path,
                });
            }
            if distance >= range {
                continue;
            }
            for next in hex.neighbors() {
                if !self.effect_connected(hex, next, units)? || !self.on_map(next) {
                    continue;
                }
                let passable = self.passable_token(next, &[]);
                if self.obstacle_for_actor(next, actor)? && !passable {
                    continue;
                }
                let mut next_mines = mines.clone();
                if passable && let Some(token) = self.occupant(next) {
                    let friendly = moving_team.is_some_and(|team| {
                        self.entity(token)
                            .and_then(|t| t.owner)
                            .filter(|id| *id != self.definitions.effect_empty)
                            .and_then(|id| self.hero(id))
                            .is_some_and(|hero| hero.team == team)
                    });
                    if !friendly
                        && let Err(index) = mines
                            .binary_search_by_key(&self.mine_identity_rank(token), |id| {
                                self.mine_identity_rank(*id)
                            })
                    {
                        let mut ids = mines.to_vec();
                        ids.insert(index, token);
                        next_mines = Arc::from(ids);
                    }
                }
                if !visited.insert((next, next_mines.clone())) {
                    continue;
                }
                let index = arena.len();
                arena.push(Node {
                    hex: next,
                    distance: distance + 1,
                    mines: next_mines,
                    parent: Some(current),
                });
                queue.push_back(index);
            }
        }
        Ok(result)
    }
}
