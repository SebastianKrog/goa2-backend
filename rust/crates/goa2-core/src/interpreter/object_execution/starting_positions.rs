//! Opening deployment is an administrative board edit, with teammate consent.
use super::{phases::Field, *};
use crate::interpreter::live_game::StartingPositionCommand;
use std::collections::HashSet;

type SwapRequests = Vec<(String, Symbol, Symbol)>;

fn invalid(message: &str) -> EngineError {
    EngineError::InvalidInput(message.into())
}

impl NativeSession {
    fn opening_piece(&self, hero: Symbol) -> Result<Option<Symbol>> {
        if self.planning.phase != crate::codec::Phase::Planning
            || self.round.round != 1
            || self.world.combat.turn != 1
            || self.world.hero_index(hero).is_none()
        {
            return Ok(None);
        }
        let pending = if let Some(objects) = &self.objects {
            let pending = objects.phases.root(&objects.arena, Field::Pending)?;
            objects.arena.field(pending, hero).is_some()
        } else {
            self.planning.pending.iter().any(|(id, _)| *id == hero)
        };
        let mut pieces = self.world.piece_ids(hero);
        Ok(pieces
            .next()
            .filter(|_| !pending && pieces.next().is_none()))
    }

    fn opening_team(&self, hero: Symbol) -> Option<Symbol> {
        self.world.entity(hero)?.team
    }

    fn opening_spawns(&self, team: Symbol) -> impl Iterator<Item = Hex> + '_ {
        self.world
            .definitions
            .zones
            .hero_spawns
            .iter()
            .filter_map(move |(owner, spawn)| (*owner == team).then_some(*spawn))
    }

    fn legal_starting_arrangement(&self, team: Symbol, changes: &[(Symbol, Hex)]) -> bool {
        let mut occupied: HashSet<_> = self
            .world
            .locations
            .iter()
            .filter(|(id, _)| !changes.iter().any(|(piece, _)| piece == id))
            .map(|(_, hex)| *hex)
            .collect();
        if changes.iter().any(|(_, dest)| {
            !self.world.on_map(*dest) || self.world.terrain(*dest) || !occupied.insert(*dest)
        }) {
            return false;
        }
        let spawns: HashSet<_> = self.opening_spawns(team).collect();
        let all_spawns_used = spawns.is_subset(&occupied);
        let red = self.symbols.name(team) == "RED";
        if changes.iter().any(|(_, dest)| {
            if spawns.contains(dest) {
                return false;
            }
            let zone = self.world.zone_at(*dest);
            !all_spawns_used
                || !self.world.definitions.zones.lanes.iter().any(|(_, zones)| {
                    let throne = if red { zones.first() } else { zones.last() };
                    throne.is_some() && throne.copied() == zone
                })
                || !spawns.iter().any(|spawn| spawn.neighbors().contains(dest))
        }) {
            return false;
        }
        all_spawns_used
            || self
                .world
                .heroes
                .iter()
                .filter(|hero| self.opening_team(hero.id) == Some(team))
                .flat_map(|hero| self.world.piece_ids(hero.id))
                .all(|piece| {
                    changes
                        .iter()
                        .find(|(id, _)| *id == piece)
                        .map(|(_, hex)| *hex)
                        .or_else(|| self.world.position(piece))
                        .is_some_and(|hex| spawns.contains(&hex))
                })
    }

    fn starting_swap(&self, hero: Symbol, target: Symbol) -> Result<Option<[(Symbol, Hex); 2]>> {
        let Some(team) = self.opening_team(hero) else {
            return Ok(None);
        };
        if hero == target || self.opening_team(target) != Some(team) {
            return Ok(None);
        }
        let (Some(piece), Some(other)) = (self.opening_piece(hero)?, self.opening_piece(target)?)
        else {
            return Ok(None);
        };
        let (Some(origin), Some(destination)) =
            (self.world.position(piece), self.world.position(other))
        else {
            return Ok(None);
        };
        let changes = [(piece, destination), (other, origin)];
        Ok(self
            .legal_starting_arrangement(team, &changes)
            .then_some(changes))
    }

    fn starting_swap_requests(&self) -> Result<SwapRequests> {
        let Some(objects) = &self.objects else {
            return Ok(self.planning.requests.clone());
        };
        let p = &objects.phases;
        let root = p.root(&objects.arena, Field::Requests)?;
        objects
            .arena
            .fields(root)
            .ok_or_else(|| unsupported("starting requests must be a dictionary"))?
            .iter()
            .map(|(key, value)| {
                let ObjectValue::Object(row) = value else {
                    return Err(unsupported("starting request row missing"));
                };
                let read = |field| match objects.arena.field(*row, p.k(field)) {
                    Some(ObjectValue::Atom(Datum::Text(hero))) => Ok(hero),
                    _ => Err(unsupported("invalid starting request hero")),
                };
                Ok((
                    self.symbols.name(*key).to_owned(),
                    read(Field::From)?,
                    read(Field::To)?,
                ))
            })
            .collect()
    }

    pub(in crate::interpreter) fn starting_position_view(
        &self,
        hero: Symbol,
    ) -> Result<Option<Value>> {
        let (Some(piece), Some(team)) = (self.opening_piece(hero)?, self.opening_team(hero)) else {
            return Ok(None);
        };
        let origin = self.world.position(piece);
        let mut destinations: Vec<_> = self
            .opening_spawns(team)
            .flat_map(|spawn| std::iter::once(spawn).chain(spawn.neighbors()))
            .collect();
        destinations.sort_unstable_by_key(|hex| hex.coordinates());
        destinations.dedup();
        destinations.retain(|dest| {
            Some(*dest) != origin && self.legal_starting_arrangement(team, &[(piece, *dest)])
        });

        let mut swap_targets = Vec::new();
        for teammate in &self.world.heroes {
            if self.starting_swap(hero, teammate.id)?.is_some() {
                swap_targets.push(self.symbols.name(teammate.id));
            }
        }
        let requests = self.starting_swap_requests()?.into_iter()
            .filter(|(_, from, to)| hero == *from || hero == *to)
            .map(|(id, from, to)| json!({"id": id, "from": self.symbols.name(from), "to": self.symbols.name(to)}))
            .collect::<Vec<_>>();
        Ok(Some(
            json!({"destinations": destinations, "swap_targets": swap_targets, "requests": requests}),
        ))
    }

    pub(in crate::interpreter) fn change_starting_position(
        &mut self,
        actor: Symbol,
        operation: StartingPositionCommand,
        new_id: String,
    ) -> Result<()> {
        use StartingPositionCommand::*;
        match operation {
            Move { destination } => {
                let piece = self.opening_piece(actor)?.ok_or_else(|| {
                    invalid(
                        "Starting positions can only change in opening planning before committing",
                    )
                })?;
                let team = self
                    .opening_team(actor)
                    .ok_or_else(|| invalid("Hero has no team"))?;
                let changes = [(piece, destination)];
                if self.world.position(piece) == Some(destination)
                    || !self.legal_starting_arrangement(team, &changes)
                {
                    return Err(invalid("Invalid starting space"));
                }
                self.apply_starting_arrangement(&changes)?;
                self.clear_starting_requests(actor)
            }
            RequestSwap { target } => {
                let target = self
                    .symbols
                    .lookup(&target)
                    .ok_or_else(|| invalid("Unknown swap teammate"))?;
                self.starting_swap(actor, target)?
                    .ok_or_else(|| invalid("Invalid starting-position swap"))?;
                self.clear_starting_requests(actor)?;
                self.edit_starting_request(new_id, Some((actor, target)))
            }
            RespondSwap { request_id, accept } => {
                let (_, from, _) = self
                    .starting_swap_requests()?
                    .into_iter()
                    .find(|(id, _, to)| id == &request_id && *to == actor)
                    .ok_or_else(|| invalid("Starting-position request is no longer available"))?;
                if accept {
                    let changes = self
                        .starting_swap(from, actor)?
                        .ok_or_else(|| invalid("Invalid starting-position swap"))?;
                    self.apply_starting_arrangement(&changes)?;
                    self.clear_starting_requests(from)?;
                    self.clear_starting_requests(actor)
                } else {
                    self.edit_starting_request(request_id, None)
                }
            }
            Cancel => self.clear_starting_requests(actor),
        }
    }

    fn apply_starting_arrangement(&mut self, changes: &[(Symbol, Hex)]) -> Result<()> {
        if self.objects.is_some() {
            return self.with_live_objects(|session, objects| {
                for (piece, _) in changes {
                    session.remove_entity_spatial_bound(objects, *piece)?;
                }
                for (piece, dest) in changes {
                    let placement = session.prepare_spatial_bound(objects, *piece, *dest, None)?;
                    session.apply_spatial_bound(objects, &placement)?;
                }
                session.record_position_snapshot_bound(objects)
            });
        }
        for (piece, _) in changes {
            self.world
                .remove_entity(*piece)
                .map_err(EngineError::Rule)?;
        }
        for (piece, dest) in changes {
            self.world
                .place_entity(*piece, *dest)
                .map_err(EngineError::Rule)?;
        }
        self.boundary.snapshot(&self.world);
        Ok(())
    }

    fn clear_starting_requests(&mut self, actor: Symbol) -> Result<()> {
        if self.objects.is_some() {
            return self.with_live_objects(|session, objects| {
                session.clear_planning_requests_bound(objects, actor)
            });
        }
        self.planning
            .requests
            .retain(|(_, from, to)| actor != *from && actor != *to);
        Ok(())
    }

    fn edit_starting_request(
        &mut self,
        id: String,
        heroes: Option<(Symbol, Symbol)>,
    ) -> Result<()> {
        if let Some(objects) = &mut self.objects {
            let p = &objects.phases;
            let root = p.root(&objects.arena, Field::Requests)?;
            let key = self
                .symbols
                .lookup(&id)
                .unwrap_or_else(|| Arc::make_mut(&mut self.symbols).intern(&id));
            objects.arena.symbols = self.symbols.clone();
            let result = if let Some((from, to)) = heroes {
                let row = objects
                    .arena
                    .dictionary(vec![
                        (p.k(Field::From), ObjectValue::Atom(Datum::Text(from))),
                        (p.k(Field::To), ObjectValue::Atom(Datum::Text(to))),
                    ])
                    .map_err(unsupported)?;
                objects.arena.set_field(root, key, row)
            } else {
                objects.arena.apply(&crate::objects::Mutation::Delete {
                    target: crate::objects::Target::Object(root),
                    key,
                })
            };
            return result.map_err(unsupported);
        }
        if let Some((from, to)) = heroes {
            self.planning.requests.push((id, from, to));
        } else {
            self.planning.requests.retain(|(key, _, _)| key != &id);
        }
        Ok(())
    }
}
