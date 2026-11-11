use super::instruction::DefenseProgram;
use super::{
    BattleProgram, CompiledStep, Conditions, EngineError, FlowProgram, Frame, Instruction,
    PrimaryProgram, RequestToken, Result, StepProgram, passive_provider_symbol, unsupported,
};
use crate::actions::{ActionStep, Passive};
use crate::attack::AttackSpec;
use crate::card_selection::CardSelectSpec;
use crate::combat::GameOutcome;
use crate::data::{Datum, Symbol, Symbols};
use crate::primary::{PrimaryOperands, PrimaryValues};
use crate::spatial::{HexFilter, Origin, compile_filter};
use crate::stats::Stat;
use serde_json::{Value, json};
use std::sync::Arc;

fn required_symbol(raw: &Value, key: &str, symbols: &mut Symbols) -> Result<Symbol> {
    Ok(symbols.intern(
        raw[key]
            .as_str()
            .ok_or_else(|| unsupported(format!("missing string field: {key}")))?,
    ))
}
fn optional_symbol(raw: &Value, key: &str, symbols: &mut Symbols) -> Result<Option<Symbol>> {
    match raw.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok((!value.is_empty()).then(|| symbols.intern(value))),
        _ => Err(unsupported(format!("invalid string field: {key}"))),
    }
}
fn integer(raw: &Value, key: &str, default: i64) -> Result<i64> {
    raw.get(key).map_or(Ok(default), |v| {
        v.as_i64()
            .ok_or_else(|| unsupported(format!("invalid integer field: {key}")))
    })
}
fn flag(raw: &Value, key: &str, default: bool) -> Result<bool> {
    raw.get(key).map_or(Ok(default), |v| {
        v.as_bool()
            .ok_or_else(|| unsupported(format!("invalid boolean field: {key}")))
    })
}
fn compile_movement_sequence(
    raw: &Value,
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<Instruction> {
    let stat = raw
        .get("range_stat_type")
        .filter(|v| !v.is_null())
        .map(|v| {
            Stat::parse(
                v.as_str()
                    .ok_or_else(|| unsupported("invalid movement stat"))?,
            )
            .map_err(unsupported)
        })
        .transpose()?;
    let templates = crate::templates::instructions().map_err(unsupported)?;
    let range = i32::try_from(integer(raw, "range_val", 1)?)
        .map_err(|_| unsupported("movement range outside i32"))?;
    let mandatory = flag(raw, "is_mandatory", true)?;
    let through = flag(raw, "pass_through_obstacles", false)?;
    let straight = flag(raw, "force_straight_line", false)?;
    let destination = raw["destination_key"].as_str().unwrap_or("target_hex");
    let mut filters = vec![
        templates["templates"]["obstacle"].clone(),
        templates["templates"]["movement_path"].clone(),
    ];
    filters[1]["range_val"] = json!(range);
    filters[1]["pass_through_obstacles"] = json!(through);
    if straight {
        filters.push(templates["templates"]["straight"].clone());
        let mut path = templates["templates"]["straight_path"].clone();
        path["pass_through_obstacles"] = json!(through);
        filters.push(path);
    }
    if flag(raw, "force_full_distance", false)? {
        let mut filter = templates["templates"]["range"].clone();
        filter["min_range"] = json!(range);
        filter["max_range"] = json!(range);
        filters.push(filter);
    }
    let mut selection = templates["templates"]["select_hex"].clone();
    selection["prompt"] = json!(format!("Select Movement Destination (Range {range})"));
    selection["output_key"] = json!(destination);
    selection["is_mandatory"] = json!(mandatory);
    selection["filters"] = Value::Array(filters);
    let select = compile_step(&selection, steps, symbols)?;
    if let Instruction::Hex { filters, .. } = &mut steps[select].instruction {
        for filter in filters {
            filter.bind_frame();
        }
    }
    let mut movement = templates["templates"]["move_unit"].clone();
    movement["range_val"] = json!(range);
    movement["destination_key"] = json!(destination);
    movement["is_mandatory"] = json!(mandatory);
    movement["is_movement_action"] = json!(true);
    movement["pass_through_obstacles"] = json!(through);
    movement["force_straight_line"] = json!(straight);
    let movement = compile_step(&movement, steps, symbols)?;
    Ok(Instruction::MovementSequence {
        unit: optional_symbol(raw, "unit_id", symbols)?,
        destination: symbols.intern(destination),
        select,
        movement,
        range,
        stat,
    })
}
fn compile_travel_sequence(
    raw: &Value,
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<Instruction> {
    let frozen = crate::templates::instructions().map_err(unsupported)?;
    let destination = raw["destination_key"].as_str().unwrap_or("target_hex");
    let change = flag(raw, "require_zone_change", false)?;
    let source_card = raw.get("source_card_id").cloned().unwrap_or(Value::Null);
    let mut filter = frozen["templates"]["travel_filter"].clone();
    filter["unit_id"] = Value::Null;
    filter["require_zone_change"] = json!(change);
    filter["source_card_id"] = source_card.clone();
    let mut selection = frozen["templates"]["select_hex"].clone();
    selection["prompt"] = json!("Select Fast Travel Destination");
    selection["output_key"] = json!(destination);
    selection["is_mandatory"] = json!(false);
    selection["filters"] = json!([filter]);
    let select = compile_step(&selection, steps, symbols)?;
    if let Instruction::Hex { filters, .. } = &mut steps[select].instruction
        && let HexFilter::FastTravel { origin, .. } = &mut filters[0]
    {
        *origin = Origin {
            frame: true,
            ..Default::default()
        };
    }
    let mut travel = frozen["templates"]["fast_travel_unit"].clone();
    travel["destination_key"] = json!(destination);
    travel["require_zone_change"] = json!(change);
    travel["source_card_id"] = source_card;
    travel["is_mandatory"] = json!(false);
    let travel = compile_step(&travel, steps, symbols)?;
    Ok(Instruction::TravelSequence {
        unit: optional_symbol(raw, "unit_id", symbols)?,
        destination: symbols.intern(destination),
        select,
        travel,
    })
}

fn compile_primary_program(
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<PrimaryProgram> {
    let source = crate::primary::catalog().map_err(unsupported)?;
    let compiled = crate::primary::compiled_catalog().map_err(unsupported)?;
    let mut bodies = Vec::new();
    for (name, wires) in compiled
        .recipes
        .as_object()
        .ok_or_else(|| unsupported("primary recipes missing"))?
    {
        let body = wires
            .as_array()
            .ok_or_else(|| unsupported("primary recipe not a list"))?
            .iter()
            .enumerate()
            .map(|(index, wire)| {
                let first = steps.len();
                let id = compile_step(wire, steps, symbols)?;
                for step in &mut steps[first..] {
                    step.primary_template = true;
                }
                if source["ranged_steps"][name]
                    .as_array()
                    .is_some_and(|v| v.contains(&json!(index)))
                {
                    let Instruction::Attack { spec, .. } = &mut steps[id].instruction else {
                        return Err(unsupported("ranged operand on non-attack recipe"));
                    };
                    spec.card_ranged = true;
                }
                Ok(id)
            })
            .collect::<Result<Vec<_>>>()?;
        bodies.push((symbols.intern(name), body));
    }
    let mut movement = combat_template("movement_sequence")?;
    movement["unit_id"] = json!(crate::primary::HERO);
    movement["range_val"] = json!(compiled.primary);
    movement["range_stat_type"] = Value::Null;
    let movement = compile_step(&movement, steps, symbols)?;
    let mut attack = combat_template("attack_sequence")?;
    attack["damage"] = json!(compiled.primary);
    attack["range_val"] = json!(compiled.range);
    attack["damage_stat_type"] = Value::Null;
    attack["range_stat_type"] = Value::Null;
    let attack = compile_step(&attack, steps, symbols)?;
    let mut log = combat_template("log")?;
    log["message"] = json!(crate::primary::DEFENSE_LOG);
    let defense = compile_step(&log, steps, symbols)?;
    log["message"] = json!(crate::primary::SKILL_LOG);
    let skill = compile_step(&log, steps, symbols)?;
    Ok(PrimaryProgram {
        bodies,
        movement,
        attack,
        defense,
        skill,
    })
}
pub(super) fn bind_primary(
    instruction: &mut Instruction,
    v: &PrimaryValues,
    slots: &PrimaryOperands,
) -> Result<()> {
    let number = |n: i64| slots.number(n, v).map_err(unsupported);
    match instruction {
        Instruction::Attack { spec, .. } => {
            if spec.card_ranged {
                spec.ranged = v.ranged;
            }
            spec.damage = number(spec.damage)?;
            spec.range = number(spec.range)?;
        }
        Instruction::Query { spec, .. } => spec.bind(slots, v).map_err(unsupported)?,
        Instruction::ContextCondition { threshold, .. } => *threshold = number(*threshold)?,
        Instruction::Hex { filters, .. } => {
            for f in filters {
                slots.hex_filter(f, v).map_err(unsupported)?;
            }
        }
        Instruction::Unit { filters, .. } | Instruction::Multi { filters, .. } => {
            for f in filters {
                slots.unit_filter(f, v).map_err(unsupported)?;
            }
        }
        Instruction::MovementSequence { unit, range, .. }
        | Instruction::Move { unit, range, .. } => {
            slots.optional_symbol(unit, v);
            *range = i32::try_from(number(i64::from(*range))?)
                .map_err(|_| unsupported("primary movement range outside i32"))?;
        }
        Instruction::Push { spec, .. } => {
            slots.optional_symbol(&mut spec.target, v);
            spec.distance = number(spec.distance)?;
        }
        Instruction::Place { unit, .. }
        | Instruction::RecordHex { unit, .. }
        | Instruction::FastTravel { unit, .. }
        | Instruction::TravelSequence { unit, .. } => slots.optional_symbol(unit, v),
        Instruction::Swap { a, b, .. } | Instruction::CheckAdjacency { a, b, .. } => {
            slots.optional_symbol(a, v);
            slots.optional_symbol(b, v);
        }
        Instruction::PlaceMarker { target, value, .. } => {
            slots.optional_symbol(target, v);
            *value = number(*value)?;
        }
        Instruction::ForceDiscard { victim, .. } => slots.optional_symbol(victim, v),
        Instruction::CreateEffect(spec) => Arc::make_mut(spec)
            .bind_primary(|value| bind_primary_datum(value, v, slots).map_err(|e| e.to_string()))
            .map_err(unsupported)?,
        Instruction::Set { value, .. } => bind_primary_datum(value, v, slots)?,
        _ => {}
    }
    Ok(())
}
fn bind_primary_datum(value: &mut Datum, v: &PrimaryValues, slots: &PrimaryOperands) -> Result<()> {
    match value {
        Datum::Text(id) => *id = slots.symbol(*id, v),
        Datum::Int(n) => *n = slots.number(*n, v).map_err(unsupported)?,
        Datum::List(values) => {
            for value in values {
                bind_primary_datum(value, v, slots)?;
            }
        }
        Datum::Object(fields) => {
            for (_, value) in fields {
                bind_primary_datum(value, v, slots)?;
            }
        }
        _ => {}
    }
    Ok(())
}
/// Bind small typed recipe operands without constructing or parsing step JSON.
pub(super) fn bind_action(
    instruction: &mut Instruction,
    action: &ActionStep,
    symbols: &Symbols,
) -> Result<()> {
    match (action, instruction) {
        (ActionStep::ChoosePiece(h), Instruction::ChoosePiece { hero, .. }) => *hero = *h,
        (ActionStep::Passive(p), Instruction::Passive { trigger, .. }) => {
            *trigger = symbols
                .lookup(p.name())
                .ok_or_else(|| unsupported("passive trigger missing"))?
        }
        (ActionStep::PreMovement(owner), Instruction::PreMovement { hero, .. })
        | (ActionStep::PreDiscard(owner), Instruction::PreDiscard { hero, .. }) => {
            *hero = Some(*owner)
        }
        (ActionStep::CardText { hero: h, .. }, Instruction::CardText { hero, .. }) => *hero = *h,
        (
            ActionStep::Movement { hero, base },
            Instruction::MovementSequence {
                unit, range, stat, ..
            },
        ) => {
            *unit = Some(*hero);
            *range = i32::try_from(*base).map_err(|_| unsupported("movement base outside i32"))?;
            *stat = Some(Stat::Movement);
        }
        (ActionStep::FastTravel { hero, .. }, Instruction::TravelSequence { unit, .. }) => {
            *unit = Some(*hero)
        }
        (ActionStep::Attack { damage, range }, Instruction::Attack { spec, .. }) => {
            spec.damage = *damage;
            spec.range = *range;
            spec.damage_stat = Some(Stat::Attack);
            spec.range_stat = Some(Stat::Range);
        }
        (ActionStep::Hold(_) | ActionStep::Defend(_), Instruction::Log) => {}
        (ActionStep::Context(k, v), Instruction::Set { key, value }) => {
            *key = *k;
            *value = v.clone();
        }
        _ => return Err(unsupported("invalid action instruction binding")),
    }
    Ok(())
}

pub(super) fn combat_template(name: &str) -> Result<Value> {
    let frozen = crate::templates::instructions().map_err(unsupported)?;
    let wire = frozen["templates"][name].clone();
    if wire.is_null() {
        return Err(unsupported("missing combat template"));
    }
    Ok(wire)
}
fn nullable_symbol(raw: &Value, key: &str, symbols: &mut Symbols) -> Result<Option<Symbol>> {
    match raw.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(symbols.intern(s))),
        _ => Err(unsupported(format!("invalid nullable symbol {key}"))),
    }
}
fn compile_attack(
    raw: &Value,
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<Instruction> {
    let stat = |name: &str| -> Result<Option<Stat>> {
        raw.get(name)
            .filter(|v| !v.is_null())
            .map(|v| {
                Stat::parse(
                    v.as_str()
                        .ok_or_else(|| unsupported("invalid attack stat"))?,
                )
                .map_err(unsupported)
            })
            .transpose()
    };
    let spec = AttackSpec {
        damage: integer(raw, "damage", 0)?,
        range: integer(raw, "range_val", 1)?,
        ranged: flag(raw, "is_ranged", false)?,
        card_ranged: false,
        initiative: flag(raw, "defense_uses_initiative", false)?,
        target: optional_symbol(raw, "target_id_key", symbols)?,
        output: optional_symbol(raw, "target_output_key", symbols)?,
        damage_bonus: optional_symbol(raw, "damage_bonus_key", symbols)?,
        range_bonus: optional_symbol(raw, "range_bonus_key", symbols)?,
        damage_stat: stat("damage_stat_type")?,
        range_stat: stat("range_stat_type")?,
    };
    let mut select = combat_template("select_unit")?;
    select["prompt"] = json!("Select Attack Target");
    select["output_key"] = json!("victim_id");
    select["is_mandatory"] = json!(flag(raw, "is_mandatory", true)?);
    let mut range = combat_template("range")?;
    range["max_range"] = json!(spec.range);
    let mut filters = vec![range, json!({"type":"team_filter","relation":"ENEMY"})];
    if let Some(extra) = raw.get("target_filters") {
        filters.extend(
            extra
                .as_array()
                .ok_or_else(|| unsupported("invalid attack filters"))?
                .iter()
                .cloned(),
        );
    }
    select["filters"] = json!(filters);
    let select = compile_step(&select, steps, symbols)?;
    let mut recipes = Vec::new();
    recipes.push(combat_template("reaction")?);
    let mut actor = combat_template("set_actor")?;
    actor["actor_key"] = json!("defender_id");
    actor["save_key"] = json!("_pre_pam_actor");
    recipes.push(actor);
    recipes.push(combat_template("passive")?);
    let mut movement = combat_template("pre_movement")?;
    movement["hero_id"] = Value::Null;
    movement["hero_key"] = json!("defender_id");
    movement["active_if_key"] = json!("is_primary_defense");
    recipes.push(movement);
    let mut discard = combat_template("pre_discard")?;
    discard["hero_id"] = Value::Null;
    discard["hero_key"] = json!("defender_id");
    discard["active_if_key"] = json!("is_primary_defense");
    discard["abort_on_defeat"] = json!(false);
    recipes.push(discard);
    let mut actor = combat_template("set_actor")?;
    actor["actor_key"] = json!("_pre_pam_actor");
    actor["save_key"] = json!("_discard_pam");
    recipes.push(actor);
    recipes.push(combat_template("defense_text")?);
    recipes.push(combat_template("resolve_combat")?);
    recipes.push(combat_template("on_block")?);
    recipes.push(combat_template("restore_action")?);
    let children = recipes
        .iter()
        .map(|wire| compile_step(wire, steps, symbols))
        .collect::<Result<Vec<_>>>()?;
    Ok(Instruction::Attack {
        spec,
        select,
        children,
    })
}
pub(super) fn bind_pre_movement(
    instruction: &mut Instruction,
    hero: Symbol,
    range: i32,
) -> Result<()> {
    match instruction {
        Instruction::Hex { filters, .. } => {
            for filter in filters {
                match filter {
                    HexFilter::Obstacle { exclude, .. } => {
                        *exclude = Origin {
                            literal: Some(hero),
                            key: None,
                            frame: false,
                        }
                    }
                    HexFilter::Movement {
                        origin,
                        range: distance,
                        ..
                    } => {
                        *origin = Origin {
                            literal: Some(hero),
                            key: None,
                            frame: false,
                        };
                        *distance = range;
                    }
                    _ => return Err(unsupported("unknown pre-action movement predicate")),
                }
            }
        }
        Instruction::Move {
            unit,
            range: distance,
            ..
        } => {
            *unit = Some(hero);
            *distance = range;
        }
        _ => return Err(unsupported("unknown pre-action movement child")),
    }
    Ok(())
}
pub(super) fn compile_projectile(
    deflect: bool,
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<(usize, usize)> {
    let key = if deflect {
        "deflect_victim"
    } else {
        "reflect_victim"
    };
    let mut select = combat_template("select_unit")?;
    select["prompt"] = json!(if deflect {
        "Select enemy hero in range to discard (not attacker)"
    } else {
        "Select enemy hero in range to discard"
    });
    select["output_key"] = json!(key);
    select["is_mandatory"] = json!(false);
    let mut range = combat_template("range")?;
    range["max_range"] = json!(3);
    let mut filters = vec![
        json!({"type":"unit_type_filter","unit_type":"HERO"}),
        json!({"type":"team_filter","relation":"ENEMY"}),
        range,
    ];
    if deflect {
        filters.push(json!({"type":"exclude_identity_filter","exclude_self":true,"exclude_keys":["attacker_id"]}));
    }
    select["filters"] = json!(filters);
    let mut force = combat_template("force_discard")?;
    force["victim_key"] = json!(key);
    Ok((
        compile_step(&select, steps, symbols)?,
        compile_step(&force, steps, symbols)?,
    ))
}
pub(super) fn compile_step(
    raw: &Value,
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<usize> {
    let conditions = Conditions {
        active: optional_symbol(raw, "active_if_key", symbols)?,
        skip: optional_symbol(raw, "skip_if_key", symbols)?,
        mandatory: flag(raw, "is_mandatory", true)?,
    };
    let instruction = match raw["type"]
        .as_str()
        .ok_or_else(|| unsupported("step lacks type"))?
    {
        "cast_spell" => Instruction::LiveSpell(crate::spells::SpellOp::Cast),
        "wish_sequence" => Instruction::LiveSpell(crate::spells::SpellOp::Wish),
        "prepare_spellbook" => Instruction::LiveSpell(crate::spells::SpellOp::Prepare),
        "remove_prepared_spells" => Instruction::LiveSpell(crate::spells::SpellOp::Remove),
        "check_unit_filters" => Instruction::LiveCheckUnitFilters,
        "check_distance" => Instruction::LiveDistanceCheck,
        "check_unit_on_board" => Instruction::LiveUnitOnBoard,
        "check_unit_type" => Instruction::LiveUnitTypeCheck,
        "combine_boolean_context" => Instruction::LiveBooleanCombine,
        "convert_card_to_item" => Instruction::LiveConvertCardItem,
        "apply_after_attack_card_text" => Instruction::LiveAfterAttackText,
        "place_token" => Instruction::LiveTokenPlace,
        "respawn_minion_at_hex" => Instruction::LiveMinionRespawn,
        "count_adjacent_enemies" => Instruction::LiveAdjacentEnemies,
        "check_zone_change" => Instruction::LiveZoneChange,
        "check_solo_win" => Instruction::LiveSoloWin,
        "count_cards" => Instruction::LiveCardCount,
        "check_hero_defeated_this_round" => Instruction::LiveRoundDefeat,
        "gain_coins" | "lose_coins" | "restore_spent_life_counter" | "steal_coins" => {
            Instruction::LiveResource
        }
        "perform_card_action" => Instruction::LivePerformCard,
        "find_next_actor" => Instruction::Flow(crate::turn::FlowStep::Next),
        "finalize_hero_turn" => Instruction::Flow(crate::turn::FlowStep::Finalize(
            required_symbol(raw, "hero_id", symbols)?,
        )),
        "confirm_resolution" => Instruction::Flow(crate::turn::FlowStep::Confirm(required_symbol(
            raw, "hero_id", symbols,
        )?)),
        "respawn_hero" => Instruction::Flow(crate::turn::FlowStep::Respawn(required_symbol(
            raw, "hero_id", symbols,
        )?)),
        "resolve_tie_breaker" => Instruction::Flow(crate::turn::FlowStep::Tie(
            raw["tied_hero_ids"]
                .as_array()
                .ok_or_else(|| unsupported("tie candidates missing"))?
                .iter()
                .map(|id| {
                    id.as_str()
                        .map(|id| symbols.intern(id))
                        .ok_or_else(|| unsupported("invalid tied hero"))
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        "advance_turn" => Instruction::Flow(crate::turn::FlowStep::Advance),
        "end_phase" => Instruction::Flow(crate::turn::FlowStep::EndPhase),
        "end_phase_cleanup" => Instruction::Flow(crate::turn::FlowStep::Cleanup),
        "resolve_upgrades" => Instruction::Flow(crate::turn::FlowStep::Upgrades),
        "round_reset" => Instruction::Flow(crate::turn::FlowStep::Reset),
        "attack_sequence" => compile_attack(raw, steps, symbols)?,
        "check_target_defeated" => Instruction::CheckDefeated {
            target: required_symbol(raw, "target_key", symbols)?,
            output: required_symbol(raw, "output_key", symbols)?,
            by_actor: flag(raw, "by_actor", false)?,
        },
        "reaction_window" => {
            let discard = compile_step(&combat_template("discard_card")?, steps, symbols)?;
            let mut passive = combat_template("passive")?;
            passive["trigger"] = json!("before_defense");
            let passive = compile_step(&passive, steps, symbols)?;
            Instruction::Reaction {
                target: symbols.intern(raw["target_player_key"].as_str().unwrap_or("target_id")),
                discard,
                passive,
            }
        }
        "multi_select" => {
            if raw["target_type"] != "UNIT" && raw["target_type"] != "UNIT_OR_TOKEN" {
                return Err(unsupported("multi-selection target closure not compiled"));
            }
            let mut single = raw.clone();
            single["type"] = json!("select_step");
            let id = compile_step(&single, steps, symbols)?;
            let Instruction::Unit {
                output,
                filters,
                tokens,
                prompt,
                ..
            } = steps[id].instruction.clone()
            else {
                return Err(unsupported("invalid multi-selection template"));
            };
            Instruction::Multi {
                output,
                filters,
                tokens,
                prompt,
                min: integer(raw, "min_selections", 0)?,
                max: integer(raw, "max_selections", 0)?,
            }
        }
        "remove_token" => Instruction::RemoveToken {
            literal: optional_symbol(raw, "token_id", symbols)?,
            key: optional_symbol(raw, "token_key", symbols)?,
        },
        "resolve_card" => {
            let mut program = Vec::new();
            for key in [
                "action_piece",
                "passive",
                "pre_movement",
                "pre_discard",
                "card_text",
                "movement_sequence",
                "fast_travel",
                "attack_sequence",
                "log",
                "context",
                "clear_select",
                "clear_each",
            ] {
                let mut wire = combat_template(key)?;
                if key == "clear_each" {
                    wire["steps_template"][0]["step_id"] =
                        json!("native-step-clear-remove-template");
                }
                program.push((key, compile_step(&wire, steps, symbols)?));
            }
            for trigger in [
                Passive::BeforeAction,
                Passive::BeforeAttack,
                Passive::BeforeMovement,
                Passive::BeforeSkill,
                Passive::AfterAttack,
                Passive::AfterMovement,
                Passive::AfterBasicSkill,
                Passive::AfterBasicAction,
                Passive::AfterPrimaryAction,
                Passive::AfterResolveCard,
            ] {
                symbols.intern(trigger.name());
            }
            Instruction::ResolveCard {
                hero: required_symbol(raw, "hero_id", symbols)?,
                program: Arc::new(program),
            }
        }
        "resolve_card_text" => Instruction::CardText {
            hero: required_symbol(raw, "hero_id", symbols)?,
            program: Arc::new(compile_primary_program(steps, symbols)?),
        },
        "compute_hex" => Instruction::ComputeHex {
            origin: optional_symbol(raw, "origin_key", symbols)?,
            target: symbols.intern(raw["target_key"].as_str().unwrap_or("")),
            scale: integer(raw, "scale", 1)?,
            output: symbols.intern(raw["output_key"].as_str().unwrap_or("computed_hex")),
        },
        "push_unit" => {
            let mut child = combat_template("passive")?;
            child["trigger"] = json!("after_push");
            Instruction::Push {
                spec: crate::displacement::PushSpec {
                    target: optional_symbol(raw, "target_id", symbols)?,
                    key: optional_symbol(raw, "target_key", symbols)?,
                    source: raw
                        .get("source_hex")
                        .filter(|v| !v.is_null())
                        .map(|v| {
                            serde_json::from_value(v.clone())
                                .map_err(|e| unsupported(e.to_string()))
                        })
                        .transpose()?,
                    source_key: optional_symbol(raw, "source_key", symbols)?,
                    distance: integer(raw, "distance", 1)?,
                    distance_key: optional_symbol(raw, "distance_key", symbols)?,
                    collision: optional_symbol(raw, "collision_output_key", symbols)?,
                    ignore: flag(raw, "ignore_obstacles", false)?,
                    victim_output: symbols.intern("push_victim_id"),
                },
                passive: compile_step(&child, steps, symbols)?,
            }
        }
        "check_lane_push" | "minion_battle" => {
            let lane = raw["lane_id"].as_str().map(|s| symbols.intern(s));
            Instruction::Battle(if raw["type"] == "check_lane_push" {
                crate::battle::BattleStep::Check(lane)
            } else {
                crate::battle::BattleStep::Minions(lane)
            })
        }
        "lane_push" => {
            Instruction::Battle(crate::battle::BattleStep::Push(crate::battle::PushSpec {
                lane: required_symbol(raw, "lane_id", symbols)?,
                loser: required_symbol(raw, "losing_team", symbols)?,
                target: raw["target_zone_id"].as_str().map(|s| symbols.intern(s)),
                skip_wave: flag(raw, "skip_wave_counter", false)?,
            }))
        }
        "choose_minion_removal" => Instruction::Battle(crate::battle::BattleStep::Casualties(
            crate::battle::RemovalSpec {
                loser: required_symbol(raw, "losing_team", symbols)?,
                zone: required_symbol(raw, "zone_id", symbols)?,
                remaining: integer(raw, "remaining_to_remove", 0)?,
            },
        )),
        "resolve_displacement" => {
            let items = raw["displacements"]
                .as_array()
                .ok_or_else(|| unsupported("displacement queue missing"))?
                .iter()
                .map(|v| {
                    let pair = v
                        .as_array()
                        .filter(|v| v.len() == 2)
                        .ok_or_else(|| unsupported("invalid displacement entry"))?;
                    let id = symbols.intern(
                        pair[0]
                            .as_str()
                            .ok_or_else(|| unsupported("invalid displaced unit"))?,
                    );
                    let hex = serde_json::from_value(pair[1].clone())
                        .map_err(|e| unsupported(format!("displacement hex: {e}")))?;
                    Ok((id, hex))
                })
                .collect::<Result<Vec<_>>>()?;
            Instruction::Battle(crate::battle::BattleStep::Displace(items))
        }
        "return_minion_to_zone" => {
            let processed = raw["processed_minion_ids"]
                .as_array()
                .ok_or_else(|| unsupported("minion return processed list missing"))?
                .iter()
                .map(|id| {
                    id.as_str()
                        .map(|id| symbols.intern(id))
                        .ok_or_else(|| unsupported("invalid processed minion"))
                })
                .collect::<Result<Vec<_>>>()?;
            let mut movement = combat_template("move_unit")?;
            movement["is_mandatory"] = json!(false);
            movement["is_movement_action"] = json!(false);
            let mut placement = combat_template("place_unit")?;
            placement["is_mandatory"] = json!(false);
            Instruction::ReturnMinions {
                processed,
                movement: compile_step(&movement, steps, symbols)?,
                placement: compile_step(&placement, steps, symbols)?,
            }
        }
        "check_adjacency" => Instruction::CheckAdjacency {
            a: optional_symbol(raw, "unit_a_id", symbols)?,
            b: optional_symbol(raw, "unit_b_id", symbols)?,
            a_key: optional_symbol(raw, "unit_a_key", symbols)?,
            b_key: optional_symbol(raw, "unit_b_key", symbols)?,
            output: required_symbol(raw, "output_key", symbols)?,
        },
        "record_hex" => Instruction::RecordHex {
            unit: optional_symbol(raw, "unit_id", symbols)?,
            key: optional_symbol(raw, "unit_key", symbols)?,
            output: required_symbol(raw, "output_key", symbols)?,
        },
        "place_unit" => Instruction::Place {
            unit: optional_symbol(raw, "unit_id", symbols)?,
            key: optional_symbol(raw, "unit_key", symbols)?,
            destination: raw
                .get("target_hex_arg")
                .filter(|v| !v.is_null())
                .map(|v| serde_json::from_value(v.clone()).map_err(|e| unsupported(e.to_string())))
                .transpose()?,
            destination_key: symbols
                .intern(raw["destination_key"].as_str().unwrap_or("target_hex")),
        },
        "swap_units" => Instruction::Swap {
            a: optional_symbol(raw, "unit_a_id", symbols)?,
            b: optional_symbol(raw, "unit_b_id", symbols)?,
            a_key: optional_symbol(raw, "unit_a_key", symbols)?,
            b_key: optional_symbol(raw, "unit_b_key", symbols)?,
        },
        "create_effect" => Instruction::CreateEffect(Arc::new(
            crate::effect_steps::EffectStepSpec::compile(raw, symbols).map_err(unsupported)?,
        )),
        "log_message" => {
            let message = raw["message"]
                .as_str()
                .ok_or_else(|| unsupported("log message missing"))?;
            if message.contains(['{', '}']) {
                return Err(unsupported("formatted debug log not compiled"));
            }
            Instruction::Log
        }
        "force_discard_or_defeat" => {
            let key = required_symbol(raw, "victim_key", symbols)?;
            let mut select = combat_template("select_card")?;
            select["prompt"] = json!("template");
            select["output_key"] = json!("card_to_discard");
            select["decision_purpose"] = json!("discard");
            select["context_hero_id_key"] = raw["victim_key"].clone();
            select["override_player_id_key"] = raw["victim_key"].clone();
            let mut discard = combat_template("discard_card")?;
            discard["card_key"] = json!("card_to_discard");
            discard["hero_key"] = raw["victim_key"].clone();
            Instruction::DiscardOrDefeat {
                key,
                killer: optional_symbol(raw, "killer_id", symbols)?,
                killer_key: optional_symbol(raw, "killer_key", symbols)?,
                immunity: optional_symbol(raw, "immunity_source_id", symbols)?,
                select: compile_step(&select, steps, symbols)?,
                discard: compile_step(&discard, steps, symbols)?,
                defeat: compile_step(&combat_template("defeat")?, steps, symbols)?,
            }
        }
        "mine_path_choice" => Instruction::MineChoice {
            unit: optional_symbol(raw, "unit_id", symbols)?,
            key: optional_symbol(raw, "unit_key", symbols)?,
            destination: symbols.intern(raw["destination_key"].as_str().unwrap_or("target_hex")),
            range: i32::try_from(integer(raw, "range_val", 1)?)
                .map_err(|_| unsupported("mine range outside i32"))?,
            output: symbols.intern(raw["output_key"].as_str().unwrap_or("triggered_mine_ids")),
            prompt: symbols.intern("Choose which mines to move through"),
        },
        "trigger_mine" => {
            let ids = if raw["mine_ids"].is_null() {
                None
            } else {
                Some(
                    raw["mine_ids"]
                        .as_array()
                        .ok_or_else(|| unsupported("invalid explicit mine list"))?
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(|s| symbols.intern(s))
                                .ok_or_else(|| unsupported("invalid explicit mine identity"))
                        })
                        .collect::<Result<Vec<_>>>()?,
                )
            };
            Instruction::TriggerMines {
                ids,
                victim: optional_symbol(raw, "victim_id", symbols)?,
                ids_key: symbols
                    .intern(raw["mine_ids_key"].as_str().unwrap_or("triggered_mine_ids")),
                victim_key: symbols.intern(raw["victim_key"].as_str().unwrap_or("mine_victim_id")),
                rollback: symbols.intern("rollback_reanchor_pending"),
                blast: symbols.intern("mine_blast"),
                discard: None,
            }
        }
        "force_discard" => {
            let victim = optional_symbol(raw, "victim_id", symbols)?;
            let key = optional_symbol(raw, "victim_key", symbols)?;
            if victim.is_none() && key.is_none() {
                return Err(unsupported("forced discard requires victim source"));
            }
            let basic = raw
                .get("card_is_basic")
                .filter(|v| !v.is_null())
                .map(|v| {
                    v.as_bool()
                        .ok_or_else(|| unsupported("invalid forced discard basic filter"))
                })
                .transpose()?;
            let mut select = combat_template("select_card")?;
            select["prompt"] = json!("template");
            select["output_key"] = json!("card_to_discard");
            select["decision_purpose"] = json!("discard");
            let mut discard = combat_template("discard_card")?;
            discard["card_key"] = json!("card_to_discard");
            Instruction::ForceDiscard {
                victim,
                key,
                basic,
                immunity: optional_symbol(raw, "immunity_source_id", symbols)?,
                select: compile_step(&select, steps, symbols)?,
                discard: compile_step(&discard, steps, symbols)?,
            }
        }
        "discard_card" => {
            let mut passive = combat_template("passive")?;
            passive["trigger"] = json!("after_card_discard");
            let passive = compile_step(&passive, steps, symbols)?;
            let played = match raw["source"].as_str() {
                None | Some("HAND") => false,
                Some("PLAYED") => true,
                _ => return Err(unsupported("discard source not compiled")),
            };
            Instruction::Discard {
                hero: optional_symbol(raw, "hero_id", symbols)?,
                hero_key: optional_symbol(raw, "hero_key", symbols)?,
                card: optional_symbol(raw, "card_id", symbols)?,
                card_key: optional_symbol(raw, "card_key", symbols)?,
                played,
                passive,
            }
        }
        "check_passive_abilities" => Instruction::Passive {
            hero: optional_symbol(raw, "hero_id", symbols)?,
            trigger: required_symbol(raw, "trigger", symbols)?,
        },
        "offer_passive" => Instruction::OfferPassive {
            spec: crate::passives::Offer {
                card: required_symbol(raw, "card_id", symbols)?,
                trigger: required_symbol(raw, "trigger", symbols)?,
                optional: flag(raw, "is_optional", true)?,
                prompt: required_symbol(raw, "prompt", symbols)?,
                hero: optional_symbol(raw, "hero_id", symbols)?,
                provider: passive_provider_symbol(raw, symbols)?,
            },
            mark: compile_step(&combat_template("mark_passive")?, steps, symbols)?,
        },
        "mark_passive_used" => Instruction::MarkPassive {
            card: required_symbol(raw, "card_id", symbols)?,
            provider: passive_provider_symbol(raw, symbols)?,
        },
        "resolve_defense_text" | "resolve_on_block_effect" => {
            let on_block = raw["type"] == "resolve_on_block_effect";
            let mut enter = combat_template("set_actor")?;
            enter["actor_key"] = json!("defender_id");
            enter["save_key"] = json!(if on_block {
                "_pre_onblock_actor"
            } else {
                "_pre_defense_actor"
            });
            let mut exit = combat_template("set_actor")?;
            exit["actor_key"] = enter["save_key"].clone();
            exit["save_key"] = json!("_discard");
            let program = DefenseProgram {
                reflect_select: None,
                reflect_discard: None,
                deflect_select: None,
                deflect_discard: None,
                enter: compile_step(&enter, steps, symbols)?,
                exit: compile_step(&exit, steps, symbols)?,
                flag: compile_step(&combat_template("context")?, steps, symbols)?,
                place: compile_step(&combat_template("place_bounty")?, steps, symbols)?,
                remove: compile_step(&combat_template("remove_bounty")?, steps, symbols)?,
            };
            Instruction::DefenseText { on_block, program }
        }
        "place_marker" => {
            let mut passive = combat_template("passive")?;
            passive["trigger"] = json!("after_place_marker");
            Instruction::PlaceMarker {
                kind: serde_json::from_value(raw["marker_type"].clone())
                    .map_err(|e| unsupported(e.to_string()))?,
                target: optional_symbol(raw, "target_id", symbols)?,
                key: optional_symbol(raw, "target_key", symbols)?,
                value: integer(raw, "value", 0)?,
                passive: compile_step(&passive, steps, symbols)?,
            }
        }
        "remove_marker" => Instruction::RemoveMarker(
            serde_json::from_value(raw["marker_type"].clone())
                .map_err(|e| unsupported(e.to_string()))?,
        ),
        "finished_expiring_effect" => Instruction::FinishedEffect,
        "restore_action_type" => Instruction::RestoreAction,
        "restore_action_context" => Instruction::RestoreContext {
            other: flag(raw, "other_hero_action", false)?,
        },
        "resolve_pre_action_movement" => {
            let recipes =
                crate::templates::instructions().map_err(unsupported)?["pre_action_movement"]
                    .as_array()
                    .ok_or_else(|| unsupported("frozen pre-action movement body missing"))?
                    .clone();
            Instruction::PreMovement {
                hero: optional_symbol(raw, "hero_id", symbols)?,
                key: optional_symbol(raw, "hero_key", symbols)?,
                children: recipes
                    .iter()
                    .map(|step| compile_step(step, steps, symbols))
                    .collect::<Result<Vec<_>>>()?,
            }
        }
        "resolve_pre_action_discard" => Instruction::PreDiscard {
            hero: optional_symbol(raw, "hero_id", symbols)?,
            key: optional_symbol(raw, "hero_key", symbols)?,
            abort: flag(raw, "abort_on_defeat", true)?,
            force: compile_step(&combat_template("pre_force_discard")?, steps, symbols)?,
            defeat: compile_step(&combat_template("defeat")?, steps, symbols)?,
        },
        "remove_unit" => Instruction::Remove {
            unit: optional_symbol(raw, "unit_id", symbols)?,
            key: optional_symbol(raw, "unit_key", symbols)?,
        },
        "trigger_game_over" => Instruction::GameOver(GameOutcome {
            team: nullable_symbol(raw, "winner", symbols)?,
            hero: nullable_symbol(raw, "individual_winner_id", symbols)?,
            condition: required_symbol(raw, "condition", symbols)?,
        }),
        "defeat_unit" => {
            let remove = compile_step(&combat_template("remove_unit")?, steps, symbols)?;
            let over = compile_step(&combat_template("game_over")?, steps, symbols)?;
            Instruction::Defeat {
                protection: None,
                victim: optional_symbol(raw, "victim_id", symbols)?,
                key: optional_symbol(raw, "victim_key", symbols)?,
                killer: nullable_symbol(raw, "killer_id", symbols)?,
                assist: integer(raw, "assist_multiplier", 1)?,
                remove,
                over,
            }
        }
        "check_minion_protection" => Instruction::MinionProtection {
            spec: crate::protection::ProtectionBinding {
                minion: required_symbol(raw, "minion_id", symbols)?,
                killer: nullable_symbol(raw, "killer_id", symbols)?,
                tried: Arc::from(
                    raw["tried_effect_ids"]
                        .as_array()
                        .ok_or_else(|| unsupported("protection tried list missing"))?
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(|id| symbols.intern(id))
                                .ok_or_else(|| unsupported("invalid protection identity"))
                        })
                        .collect::<Result<Vec<_>>>()?,
                ),
            },
            remove: compile_step(&combat_template("remove_unit")?, steps, symbols)?,
            discard: compile_step(&combat_template("discard_card")?, steps, symbols)?,
        },
        "resolve_combat" => {
            let defeat = compile_step(&combat_template("defeat")?, steps, symbols)?;
            Instruction::Combat {
                damage: integer(raw, "damage", 0)?,
                target: symbols.intern(raw["target_key"].as_str().unwrap_or("victim_id")),
                defeat,
            }
        }
        // Arena-only primary recipes may be imported as inert templates by the
        // transitional dense path. Reaching them retains the pre-pop guard.
        "co_directional_drag" => Instruction::LiveDrag,
        "move_toward_target" => Instruction::LiveMoveToward,
        "set_card_initiative" => Instruction::LiveSetInitiative,
        "schedule_action_control" => Instruction::LiveActionControl,
        "schedule_journey_return" => Instruction::LiveJourney,
        "set_context_flag" => Instruction::Set {
            key: required_symbol(raw, "key", symbols)?,
            value: Datum::from_json(raw.get("value").unwrap_or(&Value::Bool(true)), symbols)
                .map_err(unsupported)?,
        },
        "add_context_value" => Instruction::Add {
            key: required_symbol(raw, "key", symbols)?,
            amount: integer(raw, "amount", 1)?,
        },
        "set_actor" => Instruction::SetActor {
            literal: optional_symbol(raw, "actor_id", symbols)?,
            key: optional_symbol(raw, "actor_key", symbols)?,
            save: symbols.intern(raw["save_key"].as_str().unwrap_or("saved_actor_id")),
        },
        "record_target" => Instruction::Record {
            input: required_symbol(raw, "input_key", symbols)?,
            output: required_symbol(raw, "output_list_key", symbols)?,
        },
        "flip_tie_breaker_coin" => Instruction::FlipCoin,
        "retrieve_card" => Instruction::Retrieve {
            card: symbols.intern(match raw.get("card_key") {
                None => "",
                Some(Value::String(s)) => s,
                _ => return Err(unsupported("invalid retrieval card key")),
            }),
            hero_key: optional_symbol(raw, "hero_key", symbols)?,
            all: flag(raw, "retrieve_all_discarded", false)?,
        },
        "retrieve_unresolved_card" => Instruction::RetrieveUnresolved {
            hero: required_symbol(raw, "hero_id", symbols)?,
            prompt: symbols.intern("Alternative Timelines: retrieve one of your unresolved cards."),
        },
        "choose_acting_piece" => Instruction::ChoosePiece {
            hero: required_symbol(raw, "hero_id", symbols)?,
            prompt: symbols.intern("Choose which of you performs this action."),
        },
        "set_acting_piece" => Instruction::SetPiece {
            hero: required_symbol(raw, "hero_id", symbols)?,
            piece: match raw.get("piece_id") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(symbols.intern(s)),
                _ => return Err(unsupported("invalid acting-piece literal")),
            },
            key: optional_symbol(raw, "piece_key", symbols)?,
        },
        "fast_travel_sequence" => compile_travel_sequence(raw, steps, symbols)?,
        "fast_travel_unit" => Instruction::FastTravel {
            unit: optional_symbol(raw, "unit_id", symbols)?,
            destination: match raw.get("target_hex_arg") {
                None | Some(Value::Null) => None,
                Some(value) => Some(
                    serde_json::from_value(value.clone())
                        .map_err(|e| unsupported(format!("invalid travel destination: {e}")))?,
                ),
            },
            destination_key: symbols
                .intern(raw["destination_key"].as_str().unwrap_or("target_hex")),
            change: flag(raw, "require_zone_change", false)?,
        },
        "move_sequence" => compile_movement_sequence(raw, steps, symbols)?,
        "move_unit" => {
            Instruction::Move {
                mine_choice: None,
                mine_trigger: None,
                prechosen: flag(raw, "mine_path_prechosen", false)?,
                unit: optional_symbol(raw, "unit_id", symbols)?,
                unit_key: optional_symbol(raw, "unit_key", symbols)?,
                destination: match raw.get("target_hex_arg") {
                    None | Some(Value::Null) => None,
                    Some(value) => Some(serde_json::from_value(value.clone()).map_err(|e| {
                        unsupported(format!("invalid literal move destination: {e}"))
                    })?),
                },
                destination_key: symbols
                    .intern(raw["destination_key"].as_str().unwrap_or("target_hex")),
                range: i32::try_from(integer(raw, "range_val", 1)?)
                    .map_err(|_| unsupported("movement range outside i32"))?,
                through: flag(raw, "pass_through_obstacles", false)?,
                straight: flag(raw, "force_straight_line", false)?,
                crossed: optional_symbol(raw, "crossed_obstacles_output_key", symbols)?,
                success: optional_symbol(raw, "success_output_key", symbols)?,
                is_action: flag(raw, "is_movement_action", false)?,
            }
        }
        "select_step" => {
            if raw["target_type"] == "CARD" {
                Instruction::CardSelect(Arc::new(
                    CardSelectSpec::compile(raw, symbols).map_err(unsupported)?,
                ))
            } else if raw["target_type"] == "UNIT" || raw["target_type"] == "UNIT_OR_TOKEN" {
                let filters = crate::targeting::compile_effective_unit_filters(raw, symbols, false)
                    .map_err(unsupported)?;
                Instruction::Unit {
                    output: symbols.intern(raw["output_key"].as_str().unwrap_or("selection")),
                    filters,
                    tokens: raw["target_type"] == "UNIT_OR_TOKEN",
                    prompt: required_symbol(raw, "prompt", symbols)?,
                    auto: flag(raw, "auto_select_if_one", false)?,
                    owner: optional_symbol(raw, "override_player_id", symbols)?,
                    owner_key: optional_symbol(raw, "override_player_id_key", symbols)?,
                    discard: raw["decision_purpose"] == "discard",
                }
            } else if raw["target_type"] == "HEX" {
                let filters = raw["filters"]
                    .as_array()
                    .ok_or_else(|| unsupported("hex filter list missing"))?
                    .iter()
                    .map(|v| compile_filter(v, symbols).map_err(unsupported))
                    .collect::<Result<Vec<_>>>()?;
                Instruction::Hex {
                    output: symbols.intern(raw["output_key"].as_str().unwrap_or("selection")),
                    filters,
                    prompt: required_symbol(raw, "prompt", symbols)?,
                    auto: flag(raw, "auto_select_if_one", false)?,
                    owner: optional_symbol(raw, "override_player_id", symbols)?,
                    owner_key: optional_symbol(raw, "override_player_id_key", symbols)?,
                    discard: raw["decision_purpose"] == "discard",
                }
            } else {
                if raw["target_type"] != "NUMBER"
                    || raw["filters"].as_array().is_some_and(|v| !v.is_empty())
                {
                    return Err(unsupported("selection target/filter closure not compiled"));
                }
                let options = raw["number_options"]
                    .as_array()
                    .ok_or_else(|| unsupported("number options missing"))?
                    .iter()
                    .map(|v| {
                        v.as_i64()
                            .ok_or_else(|| unsupported("invalid number option"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let mut labels = Vec::new();
                if let Some(fields) = raw["number_labels"].as_object() {
                    for (number, label) in fields {
                        labels.push((
                            number
                                .parse()
                                .map_err(|_| unsupported("invalid number label index"))?,
                            symbols.intern(
                                label
                                    .as_str()
                                    .ok_or_else(|| unsupported("invalid number label"))?,
                            ),
                        ));
                    }
                }
                Instruction::Number {
                    output: symbols.intern(raw["output_key"].as_str().unwrap_or("selection")),
                    options,
                    labels,
                    prompt: required_symbol(raw, "prompt", symbols)?,
                    auto: flag(raw, "auto_select_if_one", false)?,
                    owner: optional_symbol(raw, "override_player_id", symbols)?,
                    owner_key: optional_symbol(raw, "override_player_id_key", symbols)?,
                    discard: raw["decision_purpose"] == "discard",
                }
            }
        }
        "ask_confirmation" => Instruction::Confirm {
            output: symbols.intern(raw["output_key"].as_str().unwrap_or("confirmation")),
            prompt: required_symbol(raw, "prompt", symbols)?,
            owner: optional_symbol(raw, "player_id", symbols)?,
        },
        "count" | "collect_units" => {
            let collect = raw["type"] == "collect_units";
            Instruction::Query {
                spec: crate::queries::Query::compile(raw, symbols, collect).map_err(unsupported)?,
                collect,
                output: symbols.intern(raw["output_key"].as_str().unwrap_or(if collect {
                    "collected_units"
                } else {
                    "count_result"
                })),
            }
        }
        "check_context_condition" => Instruction::ContextCondition {
            input: required_symbol(raw, "input_key", symbols)?,
            comparison: crate::queries::Comparison::parse(raw["operator"].as_str().unwrap_or(">=")),
            threshold: integer(raw, "threshold", 1)?,
            output: symbols.intern(raw["output_key"].as_str().unwrap_or("condition_met")),
        },
        "may_repeat_once" => {
            let templates = raw["steps_template"]
                .as_array()
                .ok_or_else(|| unsupported("repeat template missing"))?
                .iter()
                .map(|v| compile_step(v, steps, symbols))
                .collect::<Result<Vec<_>>>()?;
            Instruction::Repeat {
                max: integer(raw, "max_repeats", 1)?,
                prompt: required_symbol(raw, "prompt", symbols)?,
                templates,
                check: compile_step(&combat_template("check_lane_push")?, steps, symbols)?,
            }
        }
        "validate_repeat" => Instruction::ValidateRepeat {
            actor: optional_symbol(raw, "actor_id", symbols)?,
            and_key: optional_symbol(raw, "and_with_key", symbols)?,
            output: required_symbol(raw, "output_key", symbols)?,
        },
        "for_each" => {
            let templates = raw["steps_template"]
                .as_array()
                .ok_or_else(|| unsupported("ForEach template missing"))?
                .iter()
                .map(|v| compile_step(v, steps, symbols))
                .collect::<Result<Vec<_>>>()?;
            Instruction::ForEach {
                list: required_symbol(raw, "list_key", symbols)?,
                item: required_symbol(raw, "item_key", symbols)?,
                templates,
            }
        }
        other => return Err(unsupported(format!("step type not compiled: {other}"))),
    };
    let index = integer(
        raw,
        if raw["type"] == "may_repeat_once" {
            "repeats_done"
        } else {
            "current_index"
        },
        0,
    )?;
    let pending = match raw.get("pending_input") {
        None | Some(Value::Null) => None,
        Some(value) => Some(Datum::from_json(value, symbols).map_err(unsupported)?),
    };
    let request = optional_symbol(raw, "pending_request_id", symbols)?.map(RequestToken::Imported);
    let id = steps.len();
    let initial = Frame {
        instruction: id,
        object: None,
        pending,
        request,
        index,
        repeat_checked: flag(raw, "lane_push_checked", false)?,
        unit_override: None,
        generated: None,
        defeat_prompt_victim: None,
        range_override: None,
        through_override: None,
        defeat_override: None,
        outcome_override: None,
        discard_override: None,
        attack_binding: None,
        attack_followup: None,
        effect: None,
        offer_override: None,
        forced: None,
        mine_trigger: None,
        mine_snapshot: None,
        protection: None,
        mine_prechosen: false,
        pre_move: None,
        pre_processed: raw["processed_effect_ids"]
            .as_array()
            .map(|ids| {
                ids.iter()
                    .map(|id| {
                        id.as_str()
                            .map(|id| symbols.intern(id))
                            .ok_or_else(|| unsupported("invalid processed effect ID"))
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default(),
        action: None,
        zone_return: None,
        battle: None,
        continuation: None,
        multi: raw
            .get("selections")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(|s| symbols.intern(s))
                            .ok_or_else(|| unsupported("invalid saved multi-selection"))
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default(),
    };
    steps.push(CompiledStep(Arc::new(StepProgram {
        primary_template: false,
        conditions,
        instruction,
        wire: raw.clone(),
        initial,
    })));
    Ok(id)
}

pub(super) fn bind_zone_return(
    instruction: &mut Instruction,
    binding: &crate::minion_return::ReturnBinding,
) -> Result<()> {
    use crate::minion_return::ReturnBinding;
    match (instruction, binding) {
        (Instruction::ReturnMinions { processed, .. }, ReturnBinding::Processed(ids)) => {
            *processed = ids.clone()
        }
        (
            Instruction::Move {
                unit,
                destination,
                range,
                ..
            },
            ReturnBinding::Relocate {
                unit: id,
                destination: hex,
                range: Some(distance),
            },
        ) => {
            *unit = Some(*id);
            *destination = Some(*hex);
            *range = *distance;
        }
        (
            Instruction::Place {
                unit, destination, ..
            },
            ReturnBinding::Relocate {
                unit: id,
                destination: hex,
                range: None,
            },
        ) => {
            *unit = Some(*id);
            *destination = Some(*hex);
        }
        _ => return Err(unsupported("invalid minion return continuation binding")),
    }
    Ok(())
}

pub(super) fn compile_battle_program(
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<BattleProgram> {
    Ok(BattleProgram {
        remove: compile_step(&combat_template("remove_unit")?, steps, symbols)?,
        place: compile_step(&combat_template("place_unit")?, steps, symbols)?,
        victory: compile_step(&combat_template("game_over")?, steps, symbols)?,
        push: compile_step(&combat_template("lane_push")?, steps, symbols)?,
        casualties: compile_step(&combat_template("minion_removal")?, steps, symbols)?,
        displace: compile_step(&combat_template("displacement")?, steps, symbols)?,
    })
}
pub(super) fn card_error(e: crate::cards::CardError) -> EngineError {
    match e {
        crate::cards::CardError::Illegal(s) => EngineError::Rule(s.into()),
        crate::cards::CardError::Unsupported(s) => unsupported(s),
    }
}
pub(super) fn compile_flow_program(
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
    round_only: bool,
) -> Result<FlowProgram> {
    // Active-effect phase roots currently admit cleanup/upgrades/reset only.
    // Compile their reachable continuations, not unported future action flows.
    let names: &[&str] = if round_only {
        &["upgrades", "round_reset"]
    } else {
        &[
            "resolve_card",
            "confirm_resolution",
            "finalize_hero",
            "respawn",
            "tie",
            "find_actor",
            "retrieve_unresolved",
            "flip_coin",
            "return_minions",
            "check_lane_push",
            "advance_turn",
            "end_phase",
            "end_cleanup",
            "upgrades",
            "round_reset",
            "minion_battle",
        ]
    };
    let entries = names
        .iter()
        .map(|&name| Ok((name, compile_step(&combat_template(name)?, steps, symbols)?)))
        .collect::<Result<Vec<_>>>()?;
    Ok(FlowProgram { entries })
}
pub(super) fn bind_continuation(
    instruction: &mut Instruction,
    cont: &crate::planning::Continuation,
) -> Result<()> {
    use crate::{planning::Continuation as C, turn::FlowStep as F};
    match (instruction, cont) {
        (Instruction::ResolveCard { hero, .. }, C::Resolve(id))
        | (Instruction::RetrieveUnresolved { hero, .. }, C::Retrieve(id)) => *hero = *id,
        (Instruction::Flow(F::Confirm(hero)), C::Confirm(id))
        | (Instruction::Flow(F::Finalize(hero)), C::Finalize(id))
        | (Instruction::Flow(F::Respawn(hero)), C::Respawn(id)) => *hero = *id,
        (Instruction::Flow(F::Tie(tied)), C::Tie(ids)) => tied.clone_from(ids),
        (Instruction::Flow(F::Next), C::FindActor) | (Instruction::FlipCoin, C::FlipCoin) => {}
        _ => return Err(unsupported("invalid flow continuation binding")),
    }
    Ok(())
}
