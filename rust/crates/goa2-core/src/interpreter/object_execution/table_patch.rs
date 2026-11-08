//! Table corrections update both dense bindings and the corresponding typed caches.
use super::{phases::Field, *};
use crate::{
    objects::{Mutation, Target},
    stats::{MarkerKind, Stat},
};

impl NativeSession {
    fn patch_known_entity_bound(&self, objects: &BoundObjects, name: &str) -> Result<Symbol> {
        let id = self
            .symbols
            .lookup(name)
            .ok_or_else(|| EngineError::InvalidInput("Unknown entity".into()))?;
        if self
            .world
            .entity(id)
            .is_some_and(|row| matches!(row.kind, EntityKind::Hero | EntityKind::Minion))
        {
            return Ok(id);
        }
        // Tokens and other pieces are eligible if registered in the live state.
        // This includes registered supply and pieces currently off the board.
        let misc = objects.phases.root(&objects.arena, Field::MiscEntities)?;
        if objects.arena.field(misc, id).is_some() {
            return Ok(id);
        }
        Err(EngineError::InvalidInput("Unknown entity".into()))
    }

    pub(in crate::interpreter) fn patch_entity_position(
        &mut self,
        name: &str,
        destination: Option<Hex>,
        require_on_board: bool,
    ) -> Result<()> {
        self.with_live_objects(|session, objects| {
            let mut id = session.patch_known_entity_bound(objects, name)?;
            if session.world.entity(id).is_some_and(|row| row.multi_piece) {
                let mut pieces = session.world.piece_ids(id);
                id = match (pieces.next(), pieces.next()) {
                    (Some(piece), None) => piece,
                    _ => {
                        return Err(EngineError::InvalidInput(
                            "Specify a concrete hero piece".into(),
                        ));
                    }
                };
            }
            if require_on_board && session.world.position(id).is_none() {
                return Err(EngineError::InvalidInput(
                    "Entity is not on the board".into(),
                ));
            }
            if let Some(to) = destination {
                if session.world.has_loaded_map() && !session.world.on_map(to) {
                    return Err(EngineError::InvalidInput(
                        "Destination is outside the board".into(),
                    ));
                }
                if session.world.occupant(to).is_some_and(|other| other != id) {
                    return Err(EngineError::InvalidInput("Destination is occupied".into()));
                }
                let placement = session.prepare_spatial_bound(objects, id, to, None)?;
                session.apply_spatial_bound(objects, &placement)?;
            } else {
                session.preflight_remove_spatial_bound(objects, id)?;
                session.remove_entity_spatial_bound(objects, id)?;
            }
            Ok(())
        })
    }

    pub(in crate::interpreter) fn patch_marker(
        &mut self,
        kind: MarkerKind,
        placement: Option<(&str, &str, i64)>,
    ) -> Result<()> {
        self.with_live_objects(|session, objects| {
            let mut intern = |text| {
                let id = Arc::make_mut(&mut session.symbols).intern(text);
                objects.arena.symbols = session.symbols.clone();
                id
            };
            let key = intern(kind.name());
            let placement = if let Some((target, source, value)) = placement {
                let source = intern(source);
                let target = session.patch_known_entity_bound(objects, target)?;
                Some((target, source, value))
            } else {
                None
            };
            // A table correction restores the marker without replaying the
            // card/passive that originally put it there.
            let plan = session.prepare_marker_update(objects, key, placement, false)?;
            session.marker_bound(objects, plan)?;
            Ok(())
        })
    }

    pub(in crate::interpreter) fn patch_hero_gold(
        &mut self,
        hero: Symbol,
        value: i64,
    ) -> Result<()> {
        self.with_live_objects(|session, objects| {
            let model = session
                .hero_model_bound(objects, hero)
                .ok_or_else(|| unsupported("resource hero model missing"))?;
            session.write_resource_gold_bound(objects, model, hero, value)
        })
    }

    pub(in crate::interpreter) fn patch_hero_level(
        &mut self,
        hero: Symbol,
        value: i64,
    ) -> Result<()> {
        self.with_live_objects(|session, objects| {
            let model = session
                .hero_model_bound(objects, hero)
                .ok_or_else(|| unsupported("resource hero model missing"))?;
            let level = session.cleanup_key("level");
            objects
                .arena
                .set_field(model, level, ObjectValue::Atom(Datum::Int(value)))
                .map_err(unsupported)?;
            session
                .world
                .hero_row_mut(hero)
                .ok_or_else(|| unsupported("resource hero cache missing"))?
                .level = value;
            Ok(())
        })
    }

    pub(in crate::interpreter) fn patch_hero_items(
        &mut self,
        hero: Symbol,
        stat: Stat,
        value: i64,
    ) -> Result<()> {
        self.with_live_objects(|session, objects| {
            let model = session
                .hero_model_bound(objects, hero)
                .ok_or_else(|| unsupported("resource hero model missing"))?;
            let Some(ObjectValue::Object(items)) =
                objects.arena.field(model, session.cleanup_key("items"))
            else {
                return Err(unsupported("hero items mapping missing"));
            };
            let key = session.symbols.lookup(stat.name()).unwrap_or_else(|| {
                let key = Arc::make_mut(&mut session.symbols).intern(stat.name());
                objects.arena.symbols = session.symbols.clone();
                key
            });
            if value > 0 {
                objects
                    .arena
                    .set_field(items, key, ObjectValue::Atom(Datum::Int(value)))
                    .map_err(unsupported)?;
            } else if objects.arena.field(items, key).is_some() {
                objects
                    .arena
                    .apply(&Mutation::Delete {
                        target: Target::Object(items),
                        key,
                    })
                    .map_err(unsupported)?;
            }
            session.sync_hero_items_bound(objects)
        })
    }

    pub(in crate::interpreter) fn patch_wave(&mut self, lane: &str, value: i64) -> Result<()> {
        self.with_live_objects(|session, objects| {
            let lane = session.battle_key(lane)?;
            let map = session.battle_root_map(objects, "wave_counters")?;
            let (_, old) = session
                .world
                .waves
                .iter_mut()
                .find(|(id, _)| *id == lane)
                .ok_or_else(|| unsupported("lane cache missing"))?;
            objects
                .arena
                .set_field(map, lane, ObjectValue::Atom(Datum::Int(value)))
                .map_err(unsupported)?;
            *old = value;
            Ok(())
        })
    }

    pub(in crate::interpreter) fn patch_tie_breaker(&mut self, team: &str) -> Result<()> {
        self.with_live_objects(|session, objects| {
            let team = session.battle_key(team)?;
            let key = session.battle_key("tie_breaker_team")?;
            objects
                .arena
                .set_root(key, ObjectValue::Atom(Datum::Text(team)))
                .map_err(unsupported)
        })
    }

    pub(in crate::interpreter) fn patch_life(
        &mut self,
        name: &str,
        value: i64,
    ) -> Result<Option<NativeEvent>> {
        let mut revive = false;
        let event = self.with_live_objects(|session, objects| {
            let team = session.battle_key(name)?;
            let teams = objects.phases.root(&objects.arena, Field::Teams)?;
            let Some(ObjectValue::Object(model)) = objects.arena.field(teams, team) else {
                return Err(unsupported("team binding missing"));
            };
            let life = objects.phases.k(Field::Life);
            let Some(ObjectValue::Atom(Datum::Int(old))) = objects.arena.field(model, life) else {
                return Err(unsupported("team life counters missing"));
            };
            objects
                .arena
                .set_field(model, life, ObjectValue::Atom(Datum::Int(value)))
                .map_err(unsupported)?;
            session
                .world
                .combat
                .set_team_life(team, value)
                .map_err(unsupported)?;
            if session.planning.phase != Phase::GameOver && value == 0 {
                let opponent = session.battle_key(if name == "RED" { "BLUE" } else { "RED" })?;
                let condition =
                    Arc::make_mut(&mut session.symbols).intern("override_life_counters");
                objects.arena.symbols = session.symbols.clone();
                return session
                    .game_over_bound(
                        objects,
                        GameOutcome {
                            team: Some(opponent),
                            hero: None,
                            condition,
                        },
                    )
                    .map(Some);
            }
            if session.planning.phase == Phase::GameOver && old == 0 && value > 0 {
                for field in [
                    Field::Winner,
                    Field::IndividualWinner,
                    Field::VictoryCondition,
                ] {
                    objects
                        .arena
                        .set_root(objects.phases.k(field), ObjectValue::Atom(Datum::Null))
                        .map_err(unsupported)?;
                }
                session.world.combat.outcome = None;
                session.phase_set(objects, Phase::Resolution)?;
                revive = true;
            }
            Ok(None)
        })?;
        if revive {
            self.unstick_end_turn()?;
        }
        Ok(event)
    }
}
