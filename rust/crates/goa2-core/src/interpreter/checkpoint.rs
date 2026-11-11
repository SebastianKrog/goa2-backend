use super::{Instruction, NativeSession, RequestToken, Result, Team, unsupported};
use crate::{
    codec::StateEnvelope,
    data::{Datum, Symbols},
};
use serde_json::{Value, json};
use std::sync::Arc;

impl NativeSession {
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
