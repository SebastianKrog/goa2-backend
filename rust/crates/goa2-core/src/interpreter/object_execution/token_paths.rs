//! Source-ordered token paths: reconcile old supply before recomputing targets.
use super::passive_bodies::Kind;
use super::phases::Field;
use super::{
    Arc, BoundObjects, Datum, Hex, NativeSession, ObjectId, ObjectValue, Result, Symbol,
    unsupported,
};

pub(super) struct TokenPathPlan {
    kind: Symbol,
    targets: Vec<Hex>,
    owner: Option<Symbol>,
    output: Option<Symbol>,
    removals: usize,
    trail: bool,
}
impl NativeSession {
    pub(super) fn preflight_token_path_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        trail: bool,
    ) -> Result<Option<TokenPathPlan>> {
        let (_, kind) = self.token_effective_bound(objects, step)?;
        let mut output = None;
        let mut targets = Vec::new();
        if trail {
            let origin_key = self.passive_text_bound(objects, step, Field::OriginHexKey)?;
            let dest_key = self.passive_text_bound(objects, step, Field::TrailDestKey)?;
            let Some(origin) = objects
                .arena
                .context_value(origin_key)
                .filter(|v| objects.arena.truthy(v))
            else {
                return Ok(None);
            };
            let Some(dest) = objects
                .arena
                .context_value(dest_key)
                .filter(|v| objects.arena.truthy(v))
            else {
                return Ok(None);
            };
            let origin = self.spatial_hex_bound(objects, &origin)?.0;
            let dest = self.spatial_hex_bound(objects, &dest)?.0;
            if origin == dest || !origin.straight(dest) {
                return Ok(None);
            }
            let mut excluded = Vec::new();
            if let Some(key) = self
                .passive_optional_text_bound(objects, step, Field::ExcludedHexesKey)?
                .filter(|key| !self.symbols.name(*key).is_empty())
                && let Some(value) = objects.arena.context_value(key)
            {
                let ObjectValue::Object(list) = value else {
                    return Err(unsupported("trail exclusions require an iterable"));
                };
                for value in objects
                    .arena
                    .sequence(list)
                    .ok_or_else(|| unsupported("trail exclusions require a sequence"))?
                {
                    excluded.push(self.spatial_hex_bound(objects, value)?.0);
                }
            }
            // Source includes the origin, excludes the destination, and leaves
            // occupancy checks to each PlaceToken child (including off-map hexes).
            targets.push(origin);
            let line = origin.line_to(dest).unwrap();
            targets.extend(line.into_iter().take(origin.distance(dest) as usize - 1));
            targets.retain(|hex| !excluded.contains(hex));
        } else {
            let literal = self
                .passive_optional_text_bound(objects, step, Field::OriginId)?
                .filter(|id| !self.symbols.name(*id).is_empty());
            let key = self
                .passive_optional_text_bound(objects, step, Field::OriginKey)?
                .filter(|id| !self.symbols.name(*id).is_empty());
            let actor = if literal.is_some() {
                literal
            } else if let Some(key) = key {
                self.truthy_symbol_bound(objects, objects.arena.context_value(key))?
                    .or(self.planning.actor)
            } else {
                self.planning.actor
            };
            let Some(origin) = actor.and_then(|id| self.world.position(id)) else {
                return Ok(None);
            };
            let key = self.passive_text_bound(objects, step, Field::DirectionRefKey)?;
            let Some(reference) = objects
                .arena
                .context_value(key)
                .filter(|v| !matches!(v, ObjectValue::Atom(Datum::Null)))
            else {
                return Ok(None);
            };
            let reference = self.spatial_hex_bound(objects, &reference)?.0;
            if origin == reference || !origin.straight(reference) {
                return Ok(None);
            }
            let count = match objects
                .arena
                .field(step, objects.phases.k(Field::TokenCount))
            {
                Some(ObjectValue::Atom(Datum::Int(n))) => n,
                _ => return Err(unsupported("token line count must be an integer")),
            };
            let distance = i64::from(origin.distance(reference));
            let (q, r, s) = origin.coordinates();
            let (rq, rr, rs) = reference.coordinates();
            let (dq, dr, ds) = (
                (i64::from(rq) - i64::from(q)) / distance,
                (i64::from(rr) - i64::from(r)) / distance,
                (i64::from(rs) - i64::from(s)) / distance,
            );
            for n in 1..=50_i64 {
                let point = Hex::new(
                    (i64::from(q) + dq * n) as i32,
                    (i64::from(r) + dr * n) as i32,
                    (i64::from(s) + ds * n) as i32,
                )
                .map_err(unsupported)?;
                if !self.world.on_map(point) {
                    break;
                }
                self.tile_bound(objects, point)?;
                if !self.world.obstacle(point) {
                    targets.push(point);
                    if targets.len() as i64 >= count {
                        break;
                    }
                }
            }
            output = self.passive_optional_text_bound(objects, step, Field::Output)?;
        }
        let owner = self.passive_optional_text_bound(objects, step, Field::OwnerIdKey)?;
        let (total, placed) = self.token_supply_bound(objects, kind)?;
        let removals = targets.len().saturating_sub(total - placed).min(placed);
        self.phase_generation_check(
            0,
            if removals > 0 {
                (removals * 2 + 1) as u64
            } else {
                targets.len() as u64
            },
        )?;
        Ok(Some(TokenPathPlan {
            kind,
            targets,
            owner,
            output,
            removals,
            trail,
        }))
    }
    pub(super) fn token_path_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: Option<TokenPathPlan>,
    ) -> Result<()> {
        let Some(plan) = plan else { return Ok(()) };
        let program = objects.scheduler.as_ref().unwrap().clone();
        let recipes = program.passive_bodies.recipe(Kind::TokenOverflow)?;
        let mut steps = Vec::new();
        if plan.removals > 0 {
            let prompt = format!(
                "Select a {} token to remove from the board.",
                self.symbols.name(plan.kind)
            );
            let prompt = Arc::make_mut(&mut self.symbols).intern(&prompt);
            for i in 0..plan.removals {
                let key = format!(
                    "{}_rm_{i}",
                    if plan.trail {
                        "_trail_token"
                    } else {
                        "_line_token"
                    }
                );
                let key = Arc::make_mut(&mut self.symbols).intern(&key);
                objects.arena.symbols = self.symbols.clone();
                let select =
                    self.allocate_source_recipe_bound(objects, &recipes[0], &Datum::Null)?;
                let remove =
                    self.allocate_source_recipe_bound(objects, &recipes[1], &Datum::Null)?;
                for (field, value) in [
                    (Field::Prompt, Datum::Text(prompt)),
                    (Field::Output, Datum::Text(key)),
                    (
                        Field::OverridePlayerKey,
                        plan.owner.map_or(Datum::Null, Datum::Text),
                    ),
                ] {
                    objects
                        .arena
                        .set_field(select, objects.phases.k(field), ObjectValue::Atom(value))
                        .map_err(unsupported)?;
                }
                let filters = self.unit_filter_list_bound(objects, select, Field::Filters)?;
                let ObjectValue::Object(filter) = filters[1] else {
                    return Err(unsupported("token path removal filter missing"));
                };
                objects
                    .arena
                    .set_field(
                        filter,
                        objects.phases.k(Field::TokenType),
                        ObjectValue::Atom(Datum::Text(plan.kind)),
                    )
                    .map_err(unsupported)?;
                objects
                    .arena
                    .set_field(
                        remove,
                        objects.phases.k(Field::TokenKey),
                        ObjectValue::Atom(Datum::Text(key)),
                    )
                    .map_err(unsupported)?;
                steps.extend([select, remove]);
            }
            let (copy, _) = objects.copy_template(step)?;
            steps.push(copy);
        } else {
            let mut output = Vec::new();
            let default_overflow =
                Arc::make_mut(&mut self.symbols).intern("overflow_token_to_remove");
            objects.arena.symbols = self.symbols.clone();
            for (i, hex) in plan.targets.iter().enumerate() {
                let key = format!(
                    "{}{i}",
                    if plan.trail {
                        "_trail_hex_"
                    } else {
                        "_line_rock_hex_"
                    }
                );
                let key = Arc::make_mut(&mut self.symbols).intern(&key);
                objects.arena.symbols = self.symbols.clone();
                let (q, r, s) = hex.coordinates();
                let datum = Datum::Object(vec![
                    (self.coordinates[0], Datum::Int(i64::from(q))),
                    (self.coordinates[1], Datum::Int(i64::from(r))),
                    (self.coordinates[2], Datum::Int(i64::from(s))),
                ]);
                let value = objects.arena.allocate(&datum);
                objects.arena.set_context(key, value).map_err(unsupported)?;
                if plan.output.is_some() {
                    output.push(objects.arena.allocate(&datum));
                }
                let child =
                    self.allocate_source_recipe_bound(objects, &recipes[2], &Datum::Null)?;
                for (field, value) in [
                    (Field::TokenType, Datum::Text(plan.kind)),
                    (Field::HexKey, Datum::Text(key)),
                    (
                        Field::OwnerIdKey,
                        plan.owner.map_or(Datum::Null, Datum::Text),
                    ),
                    (Field::Output, Datum::Null),
                    (Field::ExistingTokenKey, Datum::Null),
                    (Field::OverflowKey, Datum::Text(default_overflow)),
                    (Field::ImmuneActions, Datum::Bool(false)),
                ] {
                    objects
                        .arena
                        .set_field(child, objects.phases.k(field), ObjectValue::Atom(value))
                        .map_err(unsupported)?;
                }
                steps.push(child);
            }
            if let Some(key) = plan.output {
                let value = objects.arena.list(output);
                objects.arena.set_context(key, value).map_err(unsupported)?;
            }
        }
        self.push_live_steps_bound(objects, &steps)
    }
}
