use super::{
    Instruction, NativeSession, PassiveBody, RequestToken, Result, Team, bind_passive_owner,
    combat_template, compile_battle_program, compile_flow_program, compile_projectile,
    compile_step, object_execution, unsupported,
};
use crate::{
    actions::ActionRules,
    attack::AttackRules,
    codec::{Phase, StateEnvelope},
    combat::CombatRules,
    data::{Datum, Registers, Symbols},
    markers::MarkerRules,
    primary::PrimaryOperands,
    reactions::ReactionRules,
    world::NativeWorld,
};
use serde_json::{Value, json};
use std::sync::Arc;

impl NativeSession {
    /// Prepare process-wide rule data before starting a timed search request.
    pub fn prepare_search() -> Result<()> {
        crate::templates::instructions().map_err(unsupported)?;
        crate::primary::catalog().map_err(unsupported)?;
        object_execution::primary_operand_catalog()?;
        object_execution::shared_primary_programs()?;
        Ok(())
    }

    pub fn compile(source: StateEnvelope) -> Result<Self> {
        let (phase, mut expanded) = source.expand_for_restore().map_err(unsupported)?;
        if !matches!(
            phase,
            Phase::Planning | Phase::Resolution | Phase::Cleanup | Phase::LevelUp | Phase::GameOver
        ) {
            return Err(unsupported("phase flow not yet compiled"));
        }
        let primary = (!source.object_bindings.is_empty())
            .then(object_execution::shared_primary_programs)
            .transpose()?;
        let mut objects = object_execution::BoundObjects::compile_validated(
            &source,
            primary.map_or_else(Symbols::default, |programs| programs.symbols.clone()),
        )?;
        let raw = &source.state;
        let has_effects = raw["active_effects"]
            .as_array()
            .is_some_and(|v| !v.is_empty());
        let effect_steps_only = matches!(
            phase,
            Phase::Planning | Phase::Resolution | Phase::Cleanup | Phase::LevelUp
        ) && raw["execution_stack"].as_array().is_some_and(|stack| {
            stack.iter().all(|step| {
                matches!(
                    step["type"].as_str(),
                    Some(
                        "create_effect"
                            | "remove_token"
                            | "check_adjacency"
                            | "compute_distance"
                            | "place_unit"
                            | "swap_units"
                            | "set_actor"
                            | "validate_repeat"
                            | "move_unit"
                            | "move_sequence"
                            | "mine_path_choice"
                            | "trigger_mine"
                            | "force_discard"
                            | "discard_card"
                            | "check_passive_abilities"
                            | "offer_passive"
                            | "mark_passive_used"
                            | "defeat_unit"
                            | "resolve_combat"
                            | "remove_unit"
                            | "trigger_game_over"
                            | "force_discard_or_defeat"
                            | "check_minion_protection"
                            | "reaction_window"
                            | "resolve_defense_text"
                            | "resolve_on_block_effect"
                            | "restore_action_type"
                            | "place_marker"
                            | "remove_marker"
                            | "attack_sequence"
                            | "multi_select"
                            | "count"
                            | "collect_units"
                            | "check_context_condition"
                            | "check_target_defeated"
                            | "resolve_pre_action_movement"
                            | "resolve_pre_action_discard"
                            | "return_minion_to_zone"
                            | "minion_battle"
                            | "choose_minion_removal"
                            | "check_lane_push"
                            | "lane_push"
                            | "resolve_displacement"
                            | "end_phase_cleanup"
                            | "resolve_upgrades"
                            | "round_reset"
                            | "ask_confirmation"
                            | "set_context_flag"
                            | "log_message"
                            | "finished_expiring_effect"
                    )
                ) || (step["type"] == "select_step"
                    && matches!(
                        step["target_type"].as_str(),
                        Some("HEX" | "NUMBER" | "CARD" | "UNIT" | "UNIT_OR_TOKEN")
                    ))
            })
        });
        let inert_terminal = phase == Phase::GameOver
            && raw["execution_stack"]
                .as_array()
                .is_some_and(|stack| stack.is_empty());
        if objects.is_none() && has_effects && !effect_steps_only && !inert_terminal {
            return Err(unsupported(
                "active-effect gameplay/phase closure not yet compiled",
            ));
        }
        let mut symbols = objects
            .as_ref()
            .map_or_else(Symbols::default, |objects| (*objects.arena.symbols).clone());
        let selection_key = symbols.intern("selection");
        let action_stack_key = symbols.intern("action_context_stack");
        let yes = symbols.intern("YES");
        let skip = symbols.intern("SKIP");
        let no_actor = symbols.intern("None");
        symbols.intern("system");
        let coordinates = ["q", "r", "s"].map(|name| symbols.intern(name));
        let planning = if has_effects {
            crate::planning::Planning::compile_effect_boundary(&source, phase, &mut symbols)
        } else {
            crate::planning::Planning::compile_flow(&source, phase, &mut symbols)
        }
        .map_err(unsupported)?;
        let team = match raw["tie_breaker_team"].as_str() {
            Some("RED") => Team::Red,
            Some("BLUE") => Team::Blue,
            // An unset live value remains null in the authoritative arena.
            // Source priority checks use RED equality and otherwise favor BLUE;
            // flipping that unset coin also follows the BLUE -> RED branch.
            None if raw["tie_breaker_team"].is_null() && objects.is_some() => Team::Blue,
            _ => return Err(unsupported("invalid tie-breaker team")),
        };
        let combat_rules = CombatRules::compile(&mut symbols);
        let marker_rules = MarkerRules::compile(&mut symbols);
        let attack_rules = AttackRules::compile(&mut symbols);
        let action_rules = ActionRules::compile(&mut symbols);
        let primary_operands = PrimaryOperands::compile_with_live(
            &mut symbols,
            object_execution::primary_operand_catalog()?,
        )
        .map_err(unsupported)?;
        let reaction_rules = Arc::new(ReactionRules::compile(&mut symbols).map_err(unsupported)?);
        let mut context_items = Vec::new();
        for (key, value) in raw["execution_context"]
            .as_object()
            .ok_or_else(|| unsupported("invalid execution context"))?
        {
            if objects.is_none() {
                context_items.push((
                    symbols.intern(key),
                    Datum::from_json(value, &mut symbols).map_err(unsupported)?,
                ));
            }
        }
        // Primary programs occupy the same prefix in every arena-backed session.
        // Live continuations are appended and can patch templates via copy-on-write.
        let mut steps = primary.map_or_else(Vec::new, |programs| programs.steps.clone());
        let mut stack = Vec::new();
        for (index, wire) in raw["execution_stack"]
            .as_array()
            .ok_or_else(|| unsupported("invalid execution stack"))?
            .iter()
            .enumerate()
        {
            let id = if objects.is_some() {
                object_execution::BoundObjects::compile_wire(wire, &mut steps, &mut symbols)?
            } else {
                compile_step(wire, &mut steps, &mut symbols)?
            };
            let mut frame = steps[id].initial.clone();
            if let Some(objects) = &mut objects {
                let object = objects.step(index)?;
                objects.bind_program(object, id, &steps)?;
                frame.object = Some(object);
                frame.pending = None;
            }
            stack.push(frame);
        }
        if let (Some(objects), Some(primary)) = (&mut objects, primary) {
            objects.bind_stored_programs(&source, &mut steps, &mut symbols)?;
            objects.compile_scheduler(&mut steps, &mut symbols, primary)?;
        }
        if objects.is_none()
            && steps
                .iter()
                .any(|step| matches!(step.instruction, Instruction::RestoreContext { .. }))
        {
            return Err(unsupported(
                "action-context restoration requires live object bindings",
            ));
        }
        let may_upgrade_cards = steps.iter().any(|s| {
            matches!(
                s.instruction,
                Instruction::Flow(crate::turn::FlowStep::Cleanup | crate::turn::FlowStep::Upgrades)
            )
        });
        let has_flow = objects.is_none()
            && ((!has_effects && phase == Phase::Planning)
                || steps
                    .iter()
                    .any(|step| matches!(step.instruction, Instruction::Flow(_))));
        let flow_program = if has_flow {
            Some(Arc::new(compile_flow_program(
                &mut steps,
                &mut symbols,
                has_effects,
            )?))
        } else {
            None
        };
        let boundary =
            crate::turn::Boundary::compile(&source, &mut symbols).map_err(unsupported)?;
        let round =
            crate::round::RoundState::compile(&source, &mut symbols).map_err(unsupported)?;
        let round_rules = if has_flow || objects.is_some() {
            Some(Arc::new(
                crate::round::RoundRules::compile(&mut symbols).map_err(unsupported)?,
            ))
        } else {
            None
        };
        // The arena owns pending execution for shared-object sessions. The
        // world only needs the active effects without their finishing programs.
        if objects.is_some()
            && let Some(effects) = expanded["active_effects"].as_array_mut()
        {
            for effect in effects {
                effect["finishing_steps"] = json!([]);
            }
        }
        let world =
            NativeWorld::compile_expanded(&source, &expanded, &mut symbols).map_err(unsupported)?;
        drop(expanded);
        // Projectile selection is a dependency of a possible on-block body,
        // not of every defense window. Do not compile unreachable targeting.
        if steps.iter().any(|s| {
            matches!(
                s.instruction,
                Instruction::DefenseText { on_block: true, .. }
            )
        }) {
            let mut projectile = |effect: &str, deflect: bool| -> Result<Option<(usize, usize)>> {
                let possible = if has_effects && !may_upgrade_cards {
                    // Bounded roots without upgrades consume held cards;
                    // discard is retained for an imported on-block frame.
                    world.heroes.iter().any(|hero| {
                        hero.hand
                            .iter()
                            .chain(&hero.discard)
                            .chain(hero.played.iter().flatten())
                            .chain(hero.current.iter())
                            .any(|id| {
                                world.card(*id).is_some_and(|card| {
                                    symbols.name(world.definitions.cards[card.definition].effect)
                                        == effect
                                })
                            })
                    })
                } else {
                    // Cleanup/upgrade and full ordinary roots can introduce deck cards.
                    world
                        .definitions
                        .cards
                        .iter()
                        .any(|d| symbols.name(d.effect) == effect)
                };
                if possible {
                    Ok(Some(compile_projectile(deflect, &mut steps, &mut symbols)?))
                } else {
                    Ok(None)
                }
            };
            let reflect = projectile("reflect_projectiles", false)?;
            let deflect = projectile("deflect_projectiles", true)?;
            for step in steps.iter_mut().filter(|step| {
                matches!(
                    step.instruction,
                    Instruction::DefenseText { on_block: true, .. }
                )
            }) {
                if let Instruction::DefenseText {
                    on_block: true,
                    program,
                } = &mut step.instruction
                {
                    program.reflect_select = reflect.map(|p| p.0);
                    program.reflect_discard = reflect.map(|p| p.1);
                    program.deflect_select = deflect.map(|p| p.0);
                    program.deflect_discard = deflect.map(|p| p.1);
                }
            }
        }
        let mut passive_bodies = Vec::new();
        let wild = symbols.intern("wild_hunt");
        let gun = symbols.intern("big_sodding_gun");
        let before_action = symbols.intern("before_action");
        let after_push = symbols.intern("after_push");
        let needs_trigger = |trigger| {
            (has_flow && !has_effects)
                || steps.iter().any(|step| match &step.instruction {
                    Instruction::Passive { trigger: kind, .. } => *kind == trigger,
                    Instruction::OfferPassive { spec, .. } => spec.trigger == trigger,
                    _ => false,
                })
        };
        let needs_wild = needs_trigger(before_action);
        let needs_gun = needs_trigger(after_push);
        if needs_wild
            && (world.definitions.cards.iter().any(|d| d.effect == wild)
                || world
                    .effects
                    .instances
                    .iter()
                    .any(|e| e.granted_passive == Some(wild)))
        {
            let recipes = crate::templates::instructions().map_err(unsupported)?["passive_bodies"]
                ["wild_hunt"]
                .as_array()
                .ok_or_else(|| unsupported("frozen Wild Hunt recipe missing"))?
                .clone();
            for hero in &world.heroes {
                let mut templates = Vec::new();
                for recipe in &recipes {
                    let mut recipe = recipe.clone();
                    bind_passive_owner(&mut recipe, symbols.name(hero.id));
                    templates.push(compile_step(&recipe, &mut steps, &mut symbols)?);
                }
                passive_bodies.push(PassiveBody {
                    effect: wild,
                    owner: hero.id,
                    templates,
                });
            }
        }
        if needs_gun
            && (world.definitions.cards.iter().any(|d| d.effect == gun)
                || world
                    .effects
                    .instances
                    .iter()
                    .any(|e| e.granted_passive == Some(gun)))
        {
            let template = compile_step(
                &combat_template("discard_or_defeat")?,
                &mut steps,
                &mut symbols,
            )?;
            for hero in &world.heroes {
                passive_bodies.push(PassiveBody {
                    effect: gun,
                    owner: hero.id,
                    templates: vec![template],
                });
            }
        }
        let needs_protection = steps
            .iter()
            .any(|s| matches!(s.instruction, Instruction::MinionProtection { .. }))
            || steps
                .iter()
                .any(|s| matches!(s.instruction, Instruction::Defeat { .. }))
                && (world.effects.instances.iter().any(|e| {
                    e.definition.behavior.kind == crate::effect_rules::Kind::MinionProtection
                }) || steps
                    .iter()
                    .any(|s| matches!(s.instruction, Instruction::CreateEffect(_))));
        let protection_program = if needs_protection {
            let program = compile_step(
                &combat_template("minion_protection")?,
                &mut steps,
                &mut symbols,
            )?;
            for step in steps
                .iter_mut()
                .filter(|step| matches!(step.instruction, Instruction::Defeat { .. }))
            {
                if let Instruction::Defeat { protection, .. } = &mut step.instruction {
                    *protection = Some(program);
                }
            }
            Some(program)
        } else {
            None
        };
        let may_create_effects = steps.iter().any(|step| {
            !step.primary_template && matches!(step.instruction, Instruction::CreateEffect(_))
        });
        let may_deploy = steps
            .iter()
            .any(|s| matches!(s.instruction, Instruction::Place { .. }))
            && (world.definitions.entities.iter().any(|e| e.passable)
                || world.definitions.supply_passable.iter().any(|(_, p, _)| *p));
        let move_links = steps
            .iter()
            .enumerate()
            .filter_map(|(i, step)| {
                if let Instruction::Move {
                    destination_key,
                    range,
                    prechosen,
                    ..
                } = step.instruction
                {
                    (world.has_deployed_passable() || prechosen || may_deploy).then_some((
                        i,
                        destination_key,
                        range,
                        prechosen,
                    ))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for (i, destination, range, prechosen) in move_links {
            let mut choice = combat_template("mine_path_choice")?;
            choice["destination_key"] = json!(symbols.name(destination));
            choice["range_val"] = json!(range);
            let choice = compile_step(&choice, &mut steps, &mut symbols)?;
            let trigger =
                compile_step(&combat_template("trigger_mine")?, &mut steps, &mut symbols)?;
            // These potential IDs specialize only the discard linker. Each
            // runtime child replaces them with its actual literal queue.
            let mut possible_ids = world
                .definitions
                .entities
                .iter()
                .filter(|e| {
                    e.token_type.is_some() && (may_deploy || world.position(e.id).is_some())
                })
                .map(|e| e.id)
                .collect::<Vec<_>>();
            if prechosen
                && let Some(ids) = raw["execution_context"]["triggered_mine_ids"].as_array()
            {
                possible_ids.extend(
                    ids.iter()
                        .filter_map(Value::as_str)
                        .map(|id| symbols.intern(id)),
                );
            }
            if let Instruction::TriggerMines { ids, .. } = &mut steps[trigger].instruction {
                *ids = Some(possible_ids);
            }

            if let Instruction::Move {
                mine_choice,
                mine_trigger,
                ..
            } = &mut steps[i].instruction
            {
                *mine_choice = Some(choice);
                *mine_trigger = Some(trigger);
            }
        }
        let mine_links = steps
            .iter()
            .enumerate()
            .filter_map(|(index, step)| {
                if let Instruction::TriggerMines {
                    ids,
                    blast,
                    victim_key,
                    ..
                } = &step.instruction
                {
                    let possible = ids.as_ref().map_or_else(
                        || {
                            world
                                .definitions
                                .entities
                                .iter()
                                .any(|e| e.token_type == Some(*blast))
                        },
                        |ids| {
                            ids.iter().any(|id| {
                                world
                                    .entity(*id)
                                    .is_some_and(|e| e.token_type == Some(*blast))
                            })
                        },
                    );
                    Some((index, *victim_key, possible))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let primary_blast_guard = mine_links.iter().any(|(_, _, possible)| *possible);
        for (index, key, possible) in mine_links {
            if !possible {
                continue;
            }
            if may_create_effects {
                return Err(unsupported(
                    "blast mine dynamically-created effect closure not compiled",
                ));
            }
            let mut forced = combat_template("force_discard")?;
            forced["victim_key"] = json!(symbols.name(key));
            let child = compile_step(&forced, &mut steps, &mut symbols)?;
            if let Instruction::TriggerMines { discard, .. } = &mut steps[index].instruction {
                *discard = Some(child);
            }
        }
        let passive_rules = Arc::new(
            crate::passives::PassiveRules::compile(&world, &mut symbols).map_err(unsupported)?,
        );
        let passive_triggers = steps
            .iter()
            .filter_map(|s| {
                if let Instruction::Passive { trigger, .. } = s.instruction {
                    Some(trigger)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let needs_passive_offer = if has_effects {
            let mut needed = false;
            for hero in &world.heroes {
                for trigger in &passive_triggers {
                    let offers = passive_rules
                        .scan(&world, Some(hero.id), Some(hero.id), *trigger)
                        .map_err(unsupported)?;
                    for offer in &offers {
                        if let Some((_, effect)) = passive_rules.resolve(&world, hero.id, *offer)
                            && !passive_bodies
                                .iter()
                                .any(|body| body.effect == effect && body.owner == hero.id)
                        {
                            return Err(unsupported(
                                "eligible effect-backed passive body not compiled",
                            ));
                        }
                    }
                    needed |= !offers.is_empty();
                }
            }
            needed
        } else {
            !passive_triggers.is_empty()
        };
        let passive_offer = if needs_passive_offer {
            Some(compile_step(
                &combat_template("offer_passive")?,
                &mut steps,
                &mut symbols,
            )?)
        } else {
            None
        };
        let turn_rules = (has_flow || objects.is_some())
            .then(|| Arc::new(crate::turn::TurnRules::compile(&world, &mut symbols)));
        let has_battle = steps
            .iter()
            .any(|s| matches!(s.instruction, Instruction::Battle(_)));
        let battle_program = if has_battle {
            Some(Arc::new(compile_battle_program(&mut steps, &mut symbols)?))
        } else {
            None
        };
        let battle_rules =
            has_battle.then(|| Arc::new(crate::battle::BattleRules::compile(&world, &mut symbols)));
        let return_rules = Arc::new(crate::minion_return::ReturnRules::compile(
            &world,
            &mut symbols,
        ));
        for step in &steps {
            if matches!(
                step.instruction,
                Instruction::Defeat { .. } | Instruction::Combat { .. }
            ) {
                for hero in &world.heroes {
                    world
                        .lifecycle_hero(hero.id)
                        .map_err(|e| unsupported(e.to_string()))?;
                }
            }
            if matches!(
                step.instruction,
                Instruction::FastTravel { .. } | Instruction::TravelSequence { .. }
            ) {
                world.ensure_fast_travel_closure().map_err(unsupported)?;
            }
            if let Instruction::ChoosePiece { hero, .. } = step.instruction
                && world
                    .hero(hero)
                    .is_some_and(|h| world.entity(h.id).is_some_and(|e| e.multi_piece))
                && world.has_uncompiled_auras
            {
                return Err(unsupported(
                    "acting-piece action-prevention aura closure not compiled",
                ));
            }
            if let Instruction::RetrieveUnresolved { hero, .. } = step.instruction
                && world.hero_index(hero).is_some()
            {
                world
                    .lifecycle_hero(hero)
                    .map_err(|e| unsupported(e.to_string()))?;
            }
            if let Instruction::Move { is_action, .. } = step.instruction {
                world.ensure_move_closure(is_action).map_err(unsupported)?;
            }
        }
        let mut context = Registers::new(&symbols);
        for (key, value) in context_items {
            context.set(key, value);
        }
        if context
            .get(action_stack_key)
            .is_some_and(|v| v.truthy(&symbols))
        {
            return Err(unsupported(
                "nested action-context restoration not yet compiled",
            ));
        }
        let mut next_request = steps
            .iter()
            .filter_map(|step| match step.initial.request {
                Some(RequestToken::Imported(id)) => symbols
                    .name(id)
                    .strip_prefix("native-request-")
                    .and_then(|v| v.parse::<u64>().ok()),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let mut next_step = steps
            .iter()
            .filter_map(|step| {
                step.wire["step_id"]
                    .as_str()
                    .and_then(|id| id.strip_prefix("native-step-"))
                    .and_then(|id| id.split('-').next())
                    .and_then(|id| id.parse::<u64>().ok())
            })
            .max()
            .unwrap_or(0);
        if let Some(resume) = source.native_resume {
            if resume.request_high_water < next_request || resume.step_high_water < next_step {
                return Err(unsupported(
                    "native resume counters precede live continuation identities",
                ));
            }
            next_request = resume.request_high_water;
            next_step = resume.step_high_water;
        }
        if next_request == u64::MAX {
            return Err(unsupported("request generation counter exhausted"));
        }
        if next_step == u64::MAX {
            return Err(unsupported("step generation counter exhausted"));
        }
        if objects.is_some() {
            // Live pending payloads belong only to arena step objects. Avoid
            // retaining/cloning a stale aggregate in immutable frame templates.
            for step in steps
                .iter_mut()
                .filter(|step| step.initial.pending.is_some())
            {
                step.initial.pending = None;
            }
        }
        symbols.freeze_compiled();
        let symbols = Arc::new(symbols);
        if let Some(objects) = &mut objects {
            objects.arena.symbols = symbols.clone();
        }
        Ok(Self {
            source: Arc::new(source),
            steps: Arc::new(steps),
            symbols,
            context,
            count_observer: crate::data::FilterObservation::default(),
            objects: objects.map(Box::new),
            world,
            combat_rules,
            marker_rules,
            attack_rules,
            track_attack_source: false,
            action_rules,
            primary_operands,
            return_rules,
            battle_rules,
            battle_program,
            reaction_rules,
            passive_rules,
            passive_offer,
            protection_program,
            primary_blast_guard,
            passive_bodies: Arc::new(passive_bodies),
            stack,
            planning,
            boundary,
            round,
            round_rules,
            turn_rules,
            flow_program,
            team,
            next_request,
            next_step,
            last_request: None,
            sampled_guess_steps: None,
            selection_key,
            action_stack_key,
            yes,
            skip,
            no_actor,
            coordinates,
        })
    }

    pub fn selection_json(&self, selection: &Datum) -> Value {
        selection.to_json(&self.symbols)
    }

    pub fn import_selection(&mut self, value: &Value) -> Result<Datum> {
        let value =
            Datum::from_json(value, Arc::make_mut(&mut self.symbols)).map_err(unsupported)?;
        if let Some(objects) = &mut self.objects {
            objects.arena.symbols = self.symbols.clone();
        }
        Ok(value)
    }
    pub fn request_token(&self, token: RequestToken) -> String {
        match token {
            RequestToken::Imported(id) => self.symbols.name(id).into(),
            RequestToken::Generated(id) => format!("native-request-{id}"),
        }
    }
    pub fn import_request_token(&mut self, value: &str) -> RequestToken {
        if let Some(token) = self.stack.last().and_then(|frame| frame.request)
            && self.request_token(token) == value
        {
            return token;
        }
        if let Some(id) = value
            .strip_prefix("native-request-")
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|id| value == format!("native-request-{id}"))
        {
            RequestToken::Generated(id)
        } else {
            let id = Arc::make_mut(&mut self.symbols).intern(value);
            if let Some(objects) = &mut self.objects {
                objects.arena.symbols = self.symbols.clone();
            }
            RequestToken::Imported(id)
        }
    }
    /// Trace-only model identities; never part of rule state or persisted IDs.
    pub fn generated_step_objects(
        &self,
        bindings: &[crate::codec::ObjectBinding],
    ) -> Result<Vec<(usize, usize)>> {
        self.objects
            .as_ref()
            .ok_or_else(|| unsupported("trace object arena missing"))?
            .arena
            .generated_step_objects(bindings)
            .map_err(unsupported)
    }
    /// Compact outer boundary preserves physical instance aliases on restore.
    pub fn export_envelope(&self) -> Result<StateEnvelope> {
        let mut source = if let Some(objects) = &self.objects {
            objects.arena.boundary(&self.source).map_err(unsupported)?
        } else {
            (*self.source).clone()
        };
        source.native_resume = Some(crate::codec::NativeResume {
            version: crate::versions::NATIVE_RESUME,
            request_high_water: self.next_request,
            step_high_water: self.next_step,
        });
        if self.objects.is_some() {
            return Ok(source);
        }
        self.planning
            .export_fields(&mut source, &self.symbols)
            .map_err(unsupported)?;
        let state = source.state.as_object_mut().unwrap();
        state.insert(
            "execution_context".into(),
            self.context.to_json(&self.symbols),
        );
        state.insert(
            "tie_breaker_team".into(),
            json!(if self.team == Team::Red {
                "RED"
            } else {
                "BLUE"
            }),
        );
        state.insert(
            "execution_stack".into(),
            Value::Array(
                self.stack
                    .iter()
                    .map(|frame| {
                        let mut wire = self.steps[frame.instruction].wire.clone();
                        if let Some(values) = frame.effect.as_ref().and_then(|e| e.primary.as_ref())
                        {
                            values.patch_wire(&mut wire, &self.symbols, &self.primary_operands);
                            if matches!(&self.steps[frame.instruction].instruction,Instruction::Attack{spec,..} if spec.card_ranged){wire["is_ranged"]=json!(values.ranged);}
                        }
                        if let Some(cont)=&frame.continuation {patch_continuation(&mut wire,cont,&self.symbols);}
                        if let Some(battle)=&frame.battle {battle.patch_wire(&mut wire,&self.symbols);}
                        if let Some(binding) = &frame.protection {
                            wire["minion_id"] = json!(self.symbols.name(binding.minion));
                            wire["killer_id"] = binding.killer.map_or(Value::Null, |id| json!(self.symbols.name(id)));
                            wire["tried_effect_ids"] = json!(binding.tried.iter().map(|id| self.symbols.name(*id)).collect::<Vec<_>>());
                        }
                        if frame.mine_prechosen { wire["mine_path_prechosen"] = json!(true); }
                        if let Some((ids, victim)) = &frame.mine_snapshot {
                            wire["mine_ids"] = json!(ids.iter().map(|id| self.symbols.name(*id)).collect::<Vec<_>>());
                            wire["victim_id"] = json!(self.symbols.name(*victim));
                        }
                        if let Some((victim,key,source))=frame.mine_trigger {
                wire["victim_id"]=victim.map(|id|self.symbols.name(id)).map_or(Value::Null,|id|json!(id));
                wire["victim_key"]=json!(self.symbols.name(key));
                wire["immunity_source_id"]=source.map(|id|self.symbols.name(id)).map_or(Value::Null,|id|json!(id));
            }
            if let Some(binding) = &frame.zone_return {
                            match binding {
                                crate::minion_return::ReturnBinding::Processed(ids) => {
                                    wire["processed_minion_ids"] = json!(ids.iter().map(|id| self.symbols.name(*id)).collect::<Vec<_>>());
                                }
                                crate::minion_return::ReturnBinding::Relocate { unit, destination, range } => {
                                    wire["unit_id"] = json!(self.symbols.name(*unit));
                                    wire["target_hex_arg"] = json!(destination);
                                    if let Some(range) = range { wire["range_val"] = json!(range); }
                                }
                            }
                        }
                        if let Some(action) = &frame.action {
                            action.patch_wire(&mut wire, &self.symbols);
                        }
                        if let Some(id) = frame.generated {
                            wire["step_id"] = json!(format!("native-step-{id}"));
                            if let Some(templates) =
                                wire.get_mut("steps_template").and_then(Value::as_array_mut)
                            {
                                for (index, template) in templates.iter_mut().enumerate() {
                                    template["step_id"] =
                                        json!(format!("native-step-{id}-template-{index}"));
                                }
                            }
                        }
                        if let Some(unit) = frame.unit_override {
                            let value = unit.map_or(Value::Null, |id| json!(self.symbols.name(id)));
                            if wire["type"] == "move_unit"
                                || wire["type"] == "mine_path_choice"
                                || wire["type"] == "fast_travel_unit"
                                || wire["type"] == "remove_unit"
                            {
                                wire["unit_id"] = value;
                            } else if wire["type"] == "select_step"
                                && let Some(filters) = wire["filters"].as_array_mut()
                            {
                                for filter in filters {
                                    let key = match filter["type"].as_str() {
                                        Some("occupied_filter") => Some("exclude_id"),
                                        Some(
                                            "movement_path_filter"
                                            | "fast_travel_destination_filter",
                                        ) => Some("unit_id"),
                                        Some(
                                            "in_straight_line_filter" | "straight_line_path_filter",
                                        ) => Some("origin_id"),
                                        _ => None,
                                    };
                                    if let Some(key) = key {
                                        filter[key] = value.clone();
                                    }
                                }
                            }
                        }
                        if let Some(offer) = frame.offer_override {
                            wire["active_effect_id"] = offer.provider.map_or(Value::Null, |id| json!(self.symbols.name(id)));
                            wire["card_id"] = json!(self.symbols.name(offer.card));
                            if wire["type"] == "offer_passive" {
                                wire["trigger"] = json!(self.symbols.name(offer.trigger));
                                wire["is_optional"] = json!(offer.optional);
                                wire["prompt"] = json!(self.symbols.name(offer.prompt));
                                wire["hero_id"] = offer.hero.map_or(Value::Null, |h| json!(self.symbols.name(h)));
                            }
                        }
                        if let Some(prompt) = frame.defeat_prompt_victim {
                            wire["prompt"] = json!(format!("{}, select a card to discard (or be Defeated).", self.symbols.name(prompt)));
                        }
                        if let Some(range) = frame.range_override {
                            if wire["type"] == "move_unit" || wire["type"] == "mine_path_choice" {
                                wire["range_val"] = json!(range);
                            } else if wire["type"] == "select_step" {
                                wire["prompt"] =
                                    json!(format!("Select Movement Destination (Range {range})"));
                                if let Some(filters) = wire["filters"].as_array_mut() {
                                    for filter in filters {
                                        if filter["type"] == "movement_path_filter" {
                                            filter["range_val"] = json!(range);
                                        }
                                        if filter["type"] == "range_filter" {
                                            filter["min_range"] = json!(range);
                                            filter["max_range"] = json!(range);
                                        }
                                    }
                                }
                            }
                        }
                        if let Some((hero,range)) = frame.pre_move {
                            if wire["type"] == "move_unit" {
                                wire["unit_id"] = json!(self.symbols.name(hero)); wire["range_val"] = json!(range);
                            } else if wire["type"] == "select_step" {
                                wire["prompt"] = json!(format!("Pre-action movement (up to {range} space{})", if range > 1 { "s" } else { "" }));
                                for filter in wire["filters"].as_array_mut().expect("compiled pre-action movement filters") {
                                    if filter["type"] == "occupied_filter" { filter["exclude_id"] = json!(self.symbols.name(hero)); }
                                    if filter["type"] == "movement_path_filter" { filter["unit_id"] = json!(self.symbols.name(hero)); filter["range_val"] = json!(range); }
                                }
                            }
                        }
                        if wire["type"] == "resolve_pre_action_discard" {
                            wire["processed_effect_ids"] = json!(frame.pre_processed.iter().map(|id| self.symbols.name(*id)).collect::<Vec<_>>());
                        }
                        if let Some(forced) = frame.forced.as_ref() {
                            if wire["type"] == "select_step" {
                                wire["context_hero_id"] = json!(self.symbols.name(forced.victim));
                                wire["override_player_id"] =
                                    json!(self.symbols.name(forced.victim));
                                wire["card_is_basic"] = json!(forced.basic);
                                if forced.shields {
                                    wire["card_containers"] = json!(["HAND", "PLAYED"]);
                                    wire["restrict_played_to_shields"] = json!(true);
                                    wire["card_is_basic"] = Value::Null;
                                    wire["allowed_card_ids"] = forced.allowed.as_ref().map_or(Value::Null, |ids| json!(ids.iter().map(|id| self.symbols.name(*id)).collect::<Vec<_>>()));
                                }
                                wire["prompt"] = json!(format!(
                                    "{}, select a card to discard.",
                                    self.symbols.name(forced.victim)
                                ));
                            } else if wire["type"] == "discard_card" {
                                wire["hero_id"] = json!(self.symbols.name(forced.victim));
                            }
                        }
                        if let Some(through) = frame.through_override {
                            if wire["type"] == "move_unit" {wire["pass_through_obstacles"] = json!(through);}
                            if let Some(filters) = wire.get_mut("filters").and_then(Value::as_array_mut) {
                                for filter in filters {
                                    if filter.get("pass_through_obstacles").is_some() {filter["pass_through_obstacles"] = json!(through);}
                                }
                            }
                        }
                        if let Some(effect) = &frame.effect {
                            if effect.primary.is_none() && wire.get("source_card_id").is_some() {
                                wire["source_card_id"] = json!(self.symbols.name(effect.card));
                            }
                            if let Some(filters) = wire
                                .get_mut("filters")
                                .and_then(Value::as_array_mut)
                                .filter(|_| effect.primary.is_none())
                            {
                                for filter in filters {
                                    if filter.get("source_card_id").is_some() {
                                        filter["source_card_id"] =
                                            json!(self.symbols.name(effect.card));
                                    }
                                }
                            }
                            if let Some(range) = effect.range
                                && let Some(filters) = wire["filters"].as_array_mut()
                            {
                                for filter in filters {
                                    if filter["type"] == "range_filter" {
                                        filter["max_range"] = json!(range);
                                    }
                                }
                            }
                            if let Some((key, value)) = &effect.context {
                                wire["key"] = json!(self.symbols.name(*key));
                                wire["value"] = value.to_json(&self.symbols);
                            }
                        }
                        if wire["type"] == "place_marker"
                            && let Some(Some(target)) = frame.unit_override
                        {
                            wire["target_id"] = json!(self.symbols.name(target));
                        }
                        if let Some(key) = frame.attack_followup {
                            wire["target_id_key"] = json!(self.symbols.name(key));
                            wire["target_output_key"] = Value::Null;
                            wire["active_if_key"] = json!(self.symbols.name(key));
                        }
                        if let Some(binding) = frame.attack_binding {
                            match wire["type"].as_str() {
                                Some("select_step") => {
                                    wire["output_key"] = json!(self.symbols.name(binding.key));
                                    wire["filters"][0]["max_range"] = json!(binding.range);
                                }
                                Some("reaction_window") => {
                                    wire["target_player_key"] =
                                        json!(self.symbols.name(binding.key))
                                }
                                Some("resolve_combat") => {
                                    wire["target_key"] = json!(self.symbols.name(binding.key));
                                    wire["damage"] = json!(binding.damage);
                                }
                                _ => {}
                            }
                        }
                        if let Some((hero, card)) = frame.discard_override {
                            wire["hero_id"] = json!(self.symbols.name(hero));
                            wire["card_id"] = json!(self.symbols.name(card));
                        }
                        if wire["type"] == "check_passive_abilities"
                            && let Some(Some(hero)) = frame.unit_override
                        {
                            wire["hero_id"] = json!(self.symbols.name(hero));
                        }
                        if let Some(binding) = frame.defeat_override {
                            wire["victim_id"] = json!(self.symbols.name(binding.victim));
                            wire["killer_id"] =
                                json!(binding.killer.map(|id| self.symbols.name(id)));
                        }
                        if let Some(outcome) = frame.outcome_override {
                            wire["winner"] = json!(outcome.team.map(|id| self.symbols.name(id)));
                            wire["individual_winner_id"] =
                                json!(outcome.hero.map(|id| self.symbols.name(id)));
                            wire["condition"] = json!(self.symbols.name(outcome.condition));
                        }
                        wire["pending_input"] = frame
                            .pending
                            .as_ref()
                            .map_or(Value::Null, |value| value.to_json(&self.symbols));
                        wire["pending_request_id"] = frame
                            .request
                            .map_or(Value::Null, |id| Value::from(self.request_token(id)));
                        if matches!(
                            self.steps[frame.instruction].instruction,
                            Instruction::Multi { .. }
                        ) {
                            wire["selections"] = json!(
                                frame
                                    .multi
                                    .iter()
                                    .map(|id| self.symbols.name(*id))
                                    .collect::<Vec<_>>()
                            );
                        }
                        if matches!(self.steps[frame.instruction].instruction, Instruction::Repeat { .. }) {
                            wire["repeats_done"] = json!(frame.index);
                            wire["lane_push_checked"] = json!(frame.repeat_checked);
                        }
                        if matches!(
                            self.steps[frame.instruction].instruction,
                            Instruction::ForEach { .. }
                        ) {
                            wire["current_index"] = json!(frame.index);
                        }
                        wire
                    })
                    .collect(),
            ),
        );
        self.boundary.export(&mut source.state, &self.symbols);
        self.round.export(&mut source.state, &self.symbols);
        self.world.export_combat(&mut source.state, &self.symbols);
        self.world.export_spatial(&mut source.state, &self.symbols);
        self.world.export_cards(&mut source, &self.symbols);
        Ok(source)
    }

    pub fn semantic_state(&self) -> Result<Value> {
        self.export_envelope()?.materialize().map_err(unsupported)
    }
}

fn patch_continuation(wire: &mut Value, cont: &crate::planning::Continuation, symbols: &Symbols) {
    use crate::planning::Continuation as C;
    match cont {
        C::Resolve(id) | C::Confirm(id) | C::Finalize(id) | C::Respawn(id) | C::Retrieve(id) => {
            wire["hero_id"] = json!(symbols.name(*id))
        }
        C::Tie(ids) => {
            wire["tied_hero_ids"] =
                json!(ids.iter().map(|id| symbols.name(*id)).collect::<Vec<_>>())
        }
        _ => {}
    }
}
