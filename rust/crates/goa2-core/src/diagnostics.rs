//! Offline differential diagnostics. JSON conversion stays outside native rules.
use crate::{
    cards::{CardError, CardId},
    codec::StateEnvelope,
    data::{Datum, Registers, Symbols},
    stats::Stat,
    world::NativeWorld,
};
use serde_json::{Value, json};

pub fn stats_batch(envelope: StateEnvelope, queries: Vec<Value>) -> Result<Value, String> {
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&envelope, &mut symbols)?;
    let mut context = Registers::new(&symbols);
    for (key, value) in envelope.state["execution_context"]
        .as_object()
        .ok_or("execution context missing")?
    {
        let key = symbols.intern(key);
        context.set(key, Datum::from_json(value, &mut symbols)?);
    }
    let actor = envelope.state["current_actor_id"]
        .as_str()
        .map(|id| symbols.intern(id));
    let mut results = Vec::new();
    for query in queries {
        let unit = symbols.intern(query["unit"].as_str().ok_or("unit missing")?);
        results.push(match query["kind"].as_str() {
            Some("stat") => json!(world.computed_stat_in_context(
                unit,
                Stat::parse(query["stat"].as_str().ok_or("stat missing")?)?,
                query["base"].as_i64().ok_or("base missing")?,
                &context
            )?),
            Some("card") => json!(world.card_stats(unit, card_id(&query, "card")?)?),
            Some("minion_defense") => json!(world.minion_defense(unit, actor)?),
            _ => return Err("unknown stat query".into()),
        });
    }
    Ok(json!({"results":results}))
}
pub fn card_trace(envelope: StateEnvelope, operations: Vec<Value>) -> Result<Value, String> {
    let mut symbols = Symbols::default();
    let mut world = NativeWorld::compile(&envelope, &mut symbols)?;
    let mut records = Vec::new();
    for operation in operations {
        let hero = symbols.intern(operation["hero"].as_str().ok_or("hero missing")?);
        let result = match operation["kind"].as_str() {
            Some("play") => world.play_card(hero, card_id(&operation, "card")?),
            Some("unplay") => world.unplay_card(hero, card_id(&operation, "card")?),
            Some("resolve") => world.resolve_current_card(
                hero,
                operation
                    .get("turn")
                    .filter(|v| !v.is_null())
                    .map(|v| v.as_i64().ok_or("invalid turn"))
                    .transpose()?,
            ),
            Some("discard") => world.discard_card(
                hero,
                card_id(&operation, "card")?,
                operation
                    .get("from_hand")
                    .map_or(Ok(true), |v| v.as_bool().ok_or("invalid from_hand"))?,
            ),
            Some("return_hand") => world.return_card_to_hand(hero, card_id(&operation, "card")?),
            Some("return_deck") => world.return_card_to_deck(hero, card_id(&operation, "card")?),
            Some("retrieve") => world.retrieve_cards(hero),
            Some("swap") => {
                world.swap_cards(hero, card_id(&operation, "a")?, card_id(&operation, "b")?)
            }
            _ => return Err("unknown card operation".into()),
        };
        let error = match result {
            Ok(()) => Value::Null,
            Err(CardError::Illegal(_)) => json!("invalid_card_action"),
            Err(error) => return Err(error.to_string()),
        };
        let mut current = envelope.clone();
        world.export_cards(&mut current, &symbols);
        world.export_spatial(&mut current.state, &symbols);
        records.push(json!({"semantic_state":current.materialize()?,"error":error}));
    }
    Ok(json!({"records":records}))
}
fn card_id(value: &Value, key: &str) -> Result<CardId, String> {
    Ok(CardId(
        value[key]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| format!("invalid physical card reference: {key}"))?,
    ))
}

/// Trace actual phase functions, stopping at their queued resolution steps.
pub fn planning_trace(envelope: StateEnvelope, operations: Vec<Value>) -> Result<Value, String> {
    let mut symbols = Symbols::default();
    let mut world = NativeWorld::compile(&envelope, &mut symbols)?;
    let mut phase = crate::planning::Planning::compile(&envelope, &mut symbols)?;
    let mut records = Vec::new();
    for operation in operations {
        let hero = symbols.intern(operation["hero"].as_str().ok_or("hero missing")?);
        let result = match operation["kind"].as_str() {
            Some("commit") => phase.commit(&mut world, hero, card_id(&operation, "card")?),
            Some("uncommit") => phase.uncommit(&mut world, hero),
            Some("pass") => phase.pass(&mut world, hero),
            Some("finish") => phase.finish(&mut world, hero),
            _ => return Err("unknown planning operation".into()),
        };
        let error = match result {
            Ok(()) => Value::Null,
            Err(CardError::Illegal(_)) => json!("invalid_planning_action"),
            Err(error) => return Err(error.to_string()),
        };
        let mut current = envelope.clone();
        world.export_cards(&mut current, &symbols);
        world.export_spatial(&mut current.state, &symbols);
        phase.export(&mut current, &world, &symbols)?;
        records.push(json!({"semantic_state":current.materialize()?,"error":error}));
    }
    Ok(json!({"records":records}))
}

/// Isolated tie-step resolution, without executing the generated card actions.
pub fn tie_trace(
    envelope: StateEnvelope,
    tied: Vec<String>,
    operations: Vec<Value>,
) -> Result<Value, String> {
    let mut symbols = Symbols::default();
    let mut world = NativeWorld::compile(&envelope, &mut symbols)?;
    let mut phase = crate::planning::Planning::compile(&envelope, &mut symbols)?;
    let favored = symbols.intern(
        envelope.state["tie_breaker_team"]
            .as_str()
            .ok_or("tie team missing")?,
    );
    let tied = tied.iter().map(|h| symbols.intern(h)).collect::<Vec<_>>();
    let mut records = Vec::new();
    let mut requests = 0;
    for operation in operations {
        let selected = operation
            .get("selection")
            .map(|v| crate::data::Datum::from_json(v, &mut symbols))
            .transpose()?;
        let choice = phase
            .resolve_tie(&mut world, &tied, favored, selected.as_ref())
            .map_err(|e| e.to_string())?;
        let request = if let Some(choice) = choice {
            requests += 1;
            let team = symbols.name(choice.team);
            let names = choice
                .candidates
                .iter()
                .map(|h| symbols.name(*h))
                .collect::<Vec<_>>();
            // Registered hero identities are ASCII identifiers. Keep their order.
            if names
                .iter()
                .any(|s| !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            {
                return Err("uncompiled non-identifier hero prompt formatting".into());
            }
            let listed = format!(
                "[{}]",
                names
                    .iter()
                    .map(|s| format!("'{s}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            json!({"id":format!("request/{requests}"),"request_type":"CHOOSE_ACTOR","player_id":format!("team:{team}"),"prompt":format!("Team {team}, choose who acts first between {listed}."),"options":names.iter().map(|s|json!({"id":s,"text":s,"metadata":{}})).collect::<Vec<_>>(),"can_skip":false,"can_rollback":false,"context":{"team":team},"decision_context":null})
        } else {
            Value::Null
        };
        let mut current = envelope.clone();
        phase.export(&mut current, &world, &symbols)?;
        records.push(json!({"semantic_state":current.materialize()?,"request":request}));
    }
    Ok(json!({"records":records}))
}

/// Complete action menus and generated step recipes; child rules are not drained.
pub fn action_trace(envelope: StateEnvelope, operations: Vec<Value>) -> Result<Value, String> {
    use crate::{
        actions::{ActionResolution, ActionRules},
        data::{Datum, Registers},
    };
    let mut symbols = Symbols::default();
    let mut world = NativeWorld::compile(&envelope, &mut symbols)?;
    let rules = ActionRules::compile(&mut symbols);
    let mut initial = Vec::new();
    for (key, value) in envelope.state["execution_context"]
        .as_object()
        .ok_or("context missing")?
    {
        initial.push((symbols.intern(key), Datum::from_json(value, &mut symbols)?));
    }
    let mut context = Registers::new(&symbols);
    for (key, value) in initial {
        context.set(key, value);
    }
    let mut records = Vec::new();
    for operation in operations {
        let hero = symbols.intern(operation["hero"].as_str().ok_or("hero missing")?);
        if let Some(index) = world.heroes.iter().position(|h| h.id == hero)
            && let Some(card) = operation.get("card")
        {
            world.heroes[index].current = if card.is_null() {
                None
            } else {
                Some(card_id(&operation, "card")?)
            };
        }
        let selected = operation
            .get("selection")
            .map(|v| Datum::from_json(v, &mut symbols))
            .transpose()?;
        let result = rules.resolve(&world, hero, selected.as_ref(), &mut context)?;
        let (request, steps) = match result {
            ActionResolution::Prompt(menu) => {
                (menu.to_json("request/action".into(), &symbols), Vec::new())
            }
            ActionResolution::Finished(steps) => {
                (Value::Null, action_steps_json(&envelope, &steps, &symbols)?)
            }
        };
        records.push(json!({"context":context.to_json(&symbols),"request":request,"steps":steps}));
    }
    Ok(json!({"records":records}))
}
fn action_steps_json(
    envelope: &StateEnvelope,
    steps: &[crate::actions::ActionStep],
    symbols: &Symbols,
) -> Result<Vec<Value>, String> {
    use crate::actions::ActionStep;
    let frozen = crate::templates::instructions()?;
    if frozen["source_sha256"] != envelope.source_sha256 {
        return Err("action templates identity mismatch".into());
    }
    let templates = &frozen["templates"];
    let mut output = Vec::new();
    for step in steps {
        let mut wire = templates[step.template()].clone();
        step.patch_wire(&mut wire, symbols);
        wire["step_id"] = json!(format!("native-step-action-{}", output.len()));
        output.push(wire);
        if matches!(step, ActionStep::Clear) {
            let mut each = templates["clear_each"].clone();
            each["step_id"] = json!(format!("native-step-action-{}", output.len()));
            each["steps_template"][0]["step_id"] =
                json!(format!("native-step-action-{}-remove", output.len()));
            output.push(each);
        }
    }
    Ok(output)
}

/// Zone safety is recomputed from mutable entity locations, never occupancy caches.
pub fn travel_zones(envelope: StateEnvelope, queries: Vec<Value>) -> Result<Value, String> {
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&envelope, &mut symbols)?;
    let mut results = Vec::new();
    for query in queries {
        if query["kind"] == "zone_at" {
            let hex = serde_json::from_value(query["hex"].clone())
                .map_err(|e| format!("invalid zone coordinate: {e}"))?;
            results.push(json!(world.zone_at(hex).map(|z| symbols.name(z))));
            continue;
        }
        let team = symbols.intern(query["team"].as_str().ok_or("team missing")?);
        let zone = symbols.intern(query["zone"].as_str().ok_or("zone missing")?);
        results.push(json!(
            world
                .safe_travel_zones(team, zone)
                .iter()
                .map(|z| symbols.name(*z))
                .collect::<Vec<_>>()
        ));
    }
    Ok(json!({"results":results}))
}

/// EffectManager lifecycle diagnostics; does not execute effect behavior/payloads.
pub fn effect_trace(envelope: StateEnvelope, operations: Vec<Value>) -> Result<Value, String> {
    use crate::effects::{Duration, Expiry};
    let mut symbols = Symbols::default();
    let mut world = NativeWorld::compile(&envelope, &mut symbols)?;
    let actor = envelope.state["current_actor_id"]
        .as_str()
        .map(|s| symbols.intern(s));
    let mut registers = Registers::new(&symbols);
    for (k, v) in envelope.state["execution_context"]
        .as_object()
        .ok_or("execution context missing")?
    {
        let key = symbols.intern(k);
        registers.set(key, Datum::from_json(v, &mut symbols)?);
    }
    let mut round = envelope.state["round"].as_i64().ok_or("round missing")?;
    let mut turn = envelope.state["turn"].as_i64().ok_or("turn missing")?;
    let finishing = |expiry: Expiry, symbols: &Symbols| -> Value {
        json!(
            expiry
                .finishing
                .iter()
                .map(|(source, steps)| json!([
                    symbols.name(*source),
                    steps.iter().map(|v| v.to_json(symbols)).collect::<Vec<_>>()
                ]))
                .collect::<Vec<_>>()
        )
    };
    let mut records = Vec::new();
    for operation in operations {
        let mut result = Value::Null;
        match operation["kind"].as_str() {
            Some("create") => {
                let spec =
                    crate::effects::EffectCreateSpec::compile(&operation["effect"], &mut symbols)?;
                let context = crate::effects::CreationContext {
                    round,
                    turn,
                    actor,
                    registers: &registers,
                };
                let index = world.create_effect(&spec, &context, &mut symbols)?;
                result =
                    json!({"index":index,"effect":world.effects.instances[index].export(&symbols)});
            }
            Some("activate_card" | "deactivate_card") => {
                let card = symbols.intern(operation["card"].as_str().ok_or("card missing")?);
                world
                    .effects
                    .activate_card(card, operation["kind"] == "activate_card");
            }
            Some("activate_id" | "deactivate_id") => {
                let id = symbols.intern(operation["id"].as_str().ok_or("effect id missing")?);
                world
                    .effects
                    .activate_id(id, operation["kind"] == "activate_id");
            }
            Some("remove" | "expire_id") => {
                let ids = if operation["kind"] == "expire_id" {
                    vec![symbols.intern(operation["id"].as_str().ok_or("effect id missing")?)]
                } else {
                    operation["ids"]
                        .as_array()
                        .ok_or("effect IDs missing")?
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(|s| symbols.intern(s))
                                .ok_or("invalid effect id")
                        })
                        .collect::<Result<_, _>>()?
                };
                world.remove_effects(&ids);
            }
            Some("expire_card") => world.expire_effects_by_card(
                symbols.intern(operation["card"].as_str().ok_or("card missing")?),
            ),
            Some("expire_source") => world.expire_effects_by_source(
                symbols.intern(operation["source"].as_str().ok_or("source missing")?),
            ),
            Some("cleanup") => world.cleanup_stale_effects(),
            Some("collect_turn") => {
                let expiry = world.effects.collect_turn_expiry(round, turn);
                result = json!({"ids":expiry.ids.iter().map(|s|symbols.name(*s)).collect::<Vec<_>>(),"finishing":finishing(expiry.clone(),&symbols)});
            }
            Some("expire_turn") => {
                result = finishing(world.expire_turn_effects(round, turn), &symbols)
            }
            Some("expire_duration") => {
                result = finishing(
                    world.expire_duration_effects(Duration::parse(
                        operation["duration"].as_str().ok_or("duration missing")?,
                    )?),
                    &symbols,
                )
            }
            Some("active") => {
                result = json!(
                    world
                        .effects
                        .instances
                        .iter()
                        .map(|e| e.is_active(round, turn))
                        .collect::<Vec<_>>()
                )
            }
            Some("clock") => {
                round = operation["round"].as_i64().ok_or("round missing")?;
                turn = operation["turn"].as_i64().ok_or("turn missing")?;
            }
            Some("restore") => {}
            _ => return Err("unknown effect lifecycle operation".into()),
        }
        let mut current = envelope.clone();
        world.export_cards(&mut current, &symbols);
        world.export_spatial(&mut current.state, &symbols);
        current.state["round"] = json!(round);
        current.state["turn"] = json!(turn);
        records.push(json!({"semantic_state":current.materialize()?,"result":result}));
        if operation["kind"] == "restore" {
            world = NativeWorld::compile(&current, &mut symbols)?;
        }
    }
    Ok(json!({"records":records}))
}

pub fn displacement_batch(envelope: StateEnvelope, queries: Vec<Value>) -> Result<Value, String> {
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&envelope, &mut symbols)?;
    let mut results = Vec::with_capacity(queries.len());
    for query in queries {
        let kind = crate::validation::DisplacementKind::parse(
            query["kind"].as_str().ok_or("displacement kind missing")?,
        )?;
        let unit = symbols.intern(query["unit"].as_str().ok_or("displacement unit missing")?);
        let actor = match &query["actor"] {
            Value::Null => None,
            Value::String(value) => Some(symbols.intern(value)),
            _ => return Err("invalid displacement actor".into()),
        };
        let destination = if query["destination"].is_null() {
            None
        } else {
            Some(serde_json::from_value(query["destination"].clone()).map_err(|e| e.to_string())?)
        };
        results.push(
            world
                .can_displace(unit, actor, kind, destination)?
                .export(&symbols),
        );
    }
    Ok(json!({"results":results}))
}

pub fn action_legality_batch(
    envelope: StateEnvelope,
    queries: Vec<Value>,
) -> Result<Value, String> {
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&envelope, &mut symbols)?;
    let mut initial = Vec::new();
    for (key, value) in envelope.state["execution_context"]
        .as_object()
        .ok_or("context missing")?
    {
        let key = symbols.intern(key);
        initial.push((key, Datum::from_json(value, &mut symbols)?));
    }
    let mut context = Registers::new(&symbols);
    for (key, value) in initial {
        context.set(key, value);
    }
    let mut results = Vec::with_capacity(queries.len());
    for query in queries {
        let unit = symbols.intern(query["unit"].as_str().ok_or("actor missing")?);
        let card = if query["card"].is_null() {
            None
        } else {
            Some(card_id(&query, "card")?)
        };
        let result = match query["kind"].as_str() {
            Some("action") => world.action_legality(
                unit,
                serde_json::from_value(query["action"].clone()).map_err(|e| e.to_string())?,
                card,
                &context,
            )?,
            Some("repeat") => world.repeat_legality(unit)?,
            Some("move") => world.movement_legality(
                unit,
                query["distance"].as_i64().ok_or("distance missing")?,
                query["is_action"]
                    .as_bool()
                    .ok_or("movement action flag missing")?,
                card,
                &context,
            )?,
            _ => return Err("unknown action validation query".into()),
        };
        results.push(result.export(&symbols));
    }
    Ok(json!({"results":results}))
}

pub fn terrain_batch(envelope: StateEnvelope, queries: Vec<Value>) -> Result<Value, String> {
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&envelope, &mut symbols)?;
    let current = envelope.state["current_actor_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| symbols.intern(s));
    let mut results = Vec::with_capacity(queries.len());
    for query in queries {
        let actor = query["actor"].as_str().map(|s| symbols.intern(s));
        let effective_actor = actor.or(current);
        let ids =
            |field: &str, symbols: &mut Symbols| -> Result<Vec<crate::data::Symbol>, String> {
                query[field]
                    .as_array()
                    .ok_or("invalid movement identities")?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(|s| symbols.intern(s))
                            .ok_or("invalid movement identity".into())
                    })
                    .collect()
            };
        let moved = ids("moved", &mut symbols)?;
        let value = match query["kind"].as_str() {
            Some("obstacle") => json!(world.obstacle_for_actor(
                serde_json::from_value(query["hex"].clone()).map_err(|e| e.to_string())?,
                effective_actor
            )?),
            Some("terrain") => json!(world.terrain_hex(
                serde_json::from_value(query["hex"].clone()).map_err(|e| e.to_string())?
            )?),
            Some("passable") => json!(world.passable_token(
                serde_json::from_value(query["hex"].clone()).map_err(|e| e.to_string())?,
                &moved
            )),
            Some("paths" | "reachable") => {
                let units = if query["units"].is_null() {
                    actor.into_iter().collect()
                } else {
                    ids("units", &mut symbols)?
                };
                let movement =
                    serde_json::from_value(query["query"].clone()).map_err(|e| e.to_string())?;
                json!(if query["kind"] == "paths" {
                    world.movement_paths(&movement, effective_actor, Some(&units), &moved)?
                } else {
                    world.movement_reachable(&movement, effective_actor, Some(&units), &moved)?
                })
            }
            _ => return Err("unknown terrain query".into()),
        };
        results.push(value);
    }
    Ok(json!({"results":results}))
}

pub fn hex_filter_batch(envelope: StateEnvelope, queries: Vec<Value>) -> Result<Value, String> {
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&envelope, &mut symbols)?;
    let actor = envelope.state["current_actor_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| symbols.intern(s));
    let coordinates = ["q", "r", "s"].map(|s| symbols.intern(s));
    let mut entries = Vec::new();
    for (key, value) in envelope.state["execution_context"]
        .as_object()
        .ok_or("context missing")?
    {
        entries.push((symbols.intern(key), Datum::from_json(value, &mut symbols)?));
    }
    let mut context = Registers::new(&symbols);
    for (key, value) in entries {
        context.set(key, value);
    }
    let mut results = Vec::new();
    for query in queries {
        let filter = crate::spatial::compile_filter(&query["filter"], &mut symbols)?;
        let candidates =
            serde_json::from_value::<Vec<crate::geometry::Hex>>(query["candidates"].clone())
                .map_err(|e| e.to_string())?;
        let mut accepted = vec![true; candidates.len()];
        filter.apply_scan(
            &crate::spatial::SpatialContext {
                world: &world,
                registers: &context,
                symbols: &symbols,
                actor,
                frame: None,
                coordinates,
                range_override: None,
                through_override: None,
            },
            &candidates,
            &mut accepted,
        )?;
        results.push(accepted);
    }
    Ok(json!({"results":results}))
}

pub fn mine_path_batch(envelope: StateEnvelope, queries: Vec<Value>) -> Result<Value, String> {
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&envelope, &mut symbols)?;
    let current = envelope.state["current_actor_id"]
        .as_str()
        .filter(|v| !v.is_empty())
        .map(|v| symbols.intern(v));
    let mut results = Vec::new();
    for query in queries {
        let start = serde_json::from_value(query["start"].clone()).map_err(|e| e.to_string())?;
        let range = i32::try_from(query["range"].as_i64().ok_or("mine range missing")?)
            .map_err(|_| "mine range outside i32")?;
        let actor = query["actor"]
            .as_str()
            .map(|v| symbols.intern(v))
            .or(current);
        let team = query["team"].as_str().map(|v| symbols.intern(v));
        let units = if query["units"].is_null() {
            Vec::new()
        } else {
            query["units"]
                .as_array()
                .ok_or("mine unit list missing")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(|v| symbols.intern(v))
                        .ok_or("invalid mine unit".into())
                })
                .collect::<Result<Vec<_>, String>>()?
        };
        let paths = world.mine_paths(start, range, actor, team, &units)?;
        let ends = serde_json::from_value::<Vec<crate::geometry::Hex>>(query["ends"].clone())
            .map_err(|e| e.to_string())?;
        results.push(
            ends.iter()
                .map(|hex| {
                    world
                        .tile_index(*hex)
                        .map(|index| {
                            paths[index]
                                .iter()
                                .map(|option| {
                                    let mut ids = option
                                        .mines
                                        .iter()
                                        .map(|id| symbols.name(*id))
                                        .collect::<Vec<_>>();
                                    ids.sort();
                                    json!({"mine_ids":ids,"path":option.path})
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>(),
        );
    }
    Ok(json!({"results":results}))
}

/// Comparison boundary for explicitly tagged mathematical mine-choice sets.
/// Ordered trigger queues, event order, option order and path order are untouched.
pub fn normalize_mine_choice_record(record: &mut Value, keys: &[String]) {
    for key in keys {
        if let Some(values) = record
            .get_mut("semantic_state")
            .and_then(|s| s.get_mut("execution_context"))
            .and_then(|s| s.get_mut(key))
            .and_then(Value::as_array_mut)
            && values.iter().all(Value::is_string)
        {
            values.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        }
    }
    if record["request"]["prompt"] == "Choose which mines to move through"
        && let Some(options) = record["request"]["options"].as_array_mut()
    {
        for option in options {
            if let Some(hexes) = option["metadata"]["metadata"]["mine_hexes"].as_array_mut() {
                hexes.sort_by_key(|h| (h["q"].as_i64(), h["r"].as_i64(), h["s"].as_i64()));
            }
        }
    }
}

/// Batch source-compatible targeting/filter diagnostics; JSON stays at the boundary.
pub fn targeting_batch(envelope: StateEnvelope, queries: Vec<Value>) -> Result<Value, String> {
    use crate::targeting::{UnitCandidates, compile_effective_unit_filters};
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&envelope, &mut symbols)?;
    let actor = envelope.state["current_actor_id"]
        .as_str()
        .map(|id| symbols.intern(id));
    let mut registers = Registers::new(&symbols);
    for (key, value) in envelope.state["execution_context"]
        .as_object()
        .ok_or("context missing")?
    {
        let key = symbols.intern(key);
        registers.set(key, Datum::from_json(value, &mut symbols)?);
    }
    let coordinates = ["q", "r", "s"].map(|key| symbols.intern(key));
    let mut results = Vec::with_capacity(queries.len());
    for query in queries {
        let mode = query["mode"].as_str().ok_or("targeting mode missing")?;
        if mode == "target" {
            let from = query["actor"].as_str().map(|id| symbols.intern(id));
            let to = query["unit"].as_str().map(|id| symbols.intern(id));
            results.push(world.can_target(from, to)?.export(&symbols));
            continue;
        }
        if !matches!(mode, "select" | "multi" | "count" | "collect") {
            return Err("targeting mode not compiled".into());
        }
        let pool = match query["target_type"].as_str() {
            Some("UNIT_OR_TOKEN") => UnitCandidates::UnitsAndTokens,
            Some("UNIT") if mode == "select" => UnitCandidates::UnitsWithIllusions,
            Some("UNIT") => UnitCandidates::Units,
            _ => return Err("targeting candidate type not compiled".into()),
        };
        let target_actor = if matches!(mode, "select" | "multi") {
            query["target_actor"]
                .as_str()
                .map(|id| symbols.intern(id))
                .or(actor)
        } else {
            None
        };
        let filters = compile_effective_unit_filters(&query, &mut symbols, mode == "count")?;
        let context = crate::spatial::SpatialContext {
            world: &world,
            registers: &registers,
            symbols: &symbols,
            actor,
            frame: None,
            coordinates,
            range_override: None,
            through_override: None,
        };
        let choices = world.select_units(pool, target_actor, &filters, &context)?;
        results.push(if mode == "count" {
            json!(choices.len())
        } else {
            json!(
                choices
                    .into_iter()
                    .map(|id| symbols.name(id))
                    .collect::<Vec<_>>()
            )
        });
    }
    Ok(json!({"results":results}))
}
