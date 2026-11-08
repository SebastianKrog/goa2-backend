//! Player-facing projections of a single authoritative revision.
use super::{GameTransition, LiveGame, Result, invalid};
use crate::{codec::Phase, stats::Stat};
use serde_json::{Map, Value, json};
use std::{cmp::Ordering, collections::HashSet};

mod wire;
use wire::visible_events;

impl LiveGame {
    /// Build the existing frontend's game response for an authenticated hero
    /// or a spectator. The private snapshot stays inside the rules process.
    pub fn client_view(&self, viewer: Option<&str>) -> Result<Value> {
        self.client_view_at(viewer, self.clock_timestamp())
    }

    pub fn client_view_at(&self, viewer: Option<&str>, now_ms: u64) -> Result<Value> {
        self.project_view(viewer, now_ms, false)
    }

    /// Full board and card projection for staff debugging and finished shares.
    /// Live player and spectator requests must use the scoped client view.
    pub fn private_replay_projection(&self) -> Result<Value> {
        self.project_view(None, self.clock_timestamp(), true)
    }

    fn project_view(&self, viewer: Option<&str>, now_ms: u64, reveal_all: bool) -> Result<Value> {
        self.check_viewer(viewer)?;
        let state = self.session.semantic_state()?;
        let players = Players::new(&state);
        let second_card_open = match viewer {
            Some(hero) if self.position().phase == Phase::Planning => self
                .session
                .planning_choices(self.hero(hero)?)?
                .iter()
                .any(|choice| matches!(choice, super::DecisionMove::FinishPlanning { .. })),
            _ => false,
        };
        let view = json!({
            "phase": state["phase"],
            "round": state["round"],
            "turn": state["turn"],
            "current_actor_id": state["current_actor_id"],
            "unresolved_hero_ids": state["unresolved_hero_ids"],
            "unresolved_cards": self.unresolved_cards(&state, &players, viewer, reveal_all)?,
            "active_zone_id": single_active_zone(&state),
            "battle_zones": state["battle_zones"],
            "battle_zone_minion_counts": battle_zone_minion_counts(&state),
            "wave_counters": state["wave_counters"],
            "cheats_enabled": state["cheats_enabled"],
            "tie_breaker_team": state["tie_breaker_team"],
            "teams": team_views(&state, viewer, second_card_open, reveal_all),
            "board": board_view(&state),
            "effects": effects_view(&state),
            "markers": marker_views(&state),
            "tokens": token_views(&state, viewer, reveal_all),
            "board_entities": board_entities(&state),
            "hero_pieces": hero_pieces(&state),
            "card_guess": card_guess(&state, &players),
            "card_reveal": card_reveal(&state, &players),
            "starting_position": viewer
                .map(|hero| self.session.starting_position_view(self.hero(hero)?))
                .transpose()?.flatten(),
            "time_control": self.time_control_view(),
            "clock": self.clock_view(now_ms),
        });
        Ok(json!({
            "view": view,
            "input_request": if reveal_all { None } else { self.client_prompt(&state, &players, viewer)? },
            "awaiting_input": self.responders(),
            "winner": winner(&state),
            "hero_names": {},
        }))
    }

    /// A published revision already sits at a decision boundary. Legacy REST
    /// clients can ask for that result again without advancing the journal.
    pub fn client_boundary_result(&self, viewer: Option<&str>) -> Result<Value> {
        let position = self.position();
        let previous_phase = position.phase;
        let transition = GameTransition {
            position,
            events: Vec::new(),
            previous_phase,
        };
        let mut result = self.client_result(&transition, viewer)?;
        result["revision"] = json!(transition.position.revision);
        Ok(result)
    }

    /// Project events against the exact state that produced this transition.
    pub fn client_result(
        &self,
        transition: &GameTransition,
        viewer: Option<&str>,
    ) -> Result<Value> {
        self.check_viewer(viewer)?;
        if transition.position != self.position() {
            return Err(invalid("event transition belongs to another revision"));
        }
        let state = self.session.semantic_state()?;
        let players = Players::new(&state);
        let result_type = if transition.position.phase == Phase::GameOver {
            "GAME_OVER"
        } else if self.session.last_request.is_some() {
            "INPUT_NEEDED"
        } else if transition.previous_phase != transition.position.phase {
            "PHASE_CHANGED"
        } else {
            "ACTION_COMPLETE"
        };
        Ok(json!({
            "result_type": result_type,
            "current_phase": transition.position.phase,
            "events": visible_events(&transition.events, &state, &players, viewer),
            "input_request": self.client_prompt(&state, &players, viewer)?,
            "awaiting_input": self.responders(),
            "winner": winner(&state),
        }))
    }

    fn check_viewer(&self, viewer: Option<&str>) -> Result<()> {
        if let Some(hero) = viewer {
            self.hero(hero)?;
        }
        Ok(())
    }

    /// Players who can currently make a planning or prompted decision.
    pub fn decision_players(&self) -> Result<Vec<&str>> {
        if self.session.planning.phase != Phase::Planning {
            return Ok(self.responders());
        }
        let mut players = Vec::new();
        for hero in &self.session.world.heroes {
            if !self.session.planning_choices(hero.id)?.is_empty() {
                players.push(self.session.symbols.name(hero.id));
            }
        }
        Ok(players)
    }

    fn responders(&self) -> Vec<&str> {
        let Some(request) = &self.session.last_request else {
            return Vec::new();
        };
        let mut ids = self
            .session
            .world
            .heroes
            .iter()
            .filter(|hero| self.session.resolution_request_addresses(request, hero.id))
            .map(|hero| self.session.symbols.name(hero.id))
            .collect::<Vec<_>>();
        if matches!(request.choices, super::super::RequestChoices::Upgrades(_)) {
            ids.sort_unstable();
        }
        ids
    }

    fn unresolved_cards(
        &self,
        state: &Value,
        players: &Players<'_>,
        viewer: Option<&str>,
        reveal_all: bool,
    ) -> Result<Vec<Value>> {
        if state["phase"] != "RESOLUTION" {
            return Ok(Vec::new());
        }
        let mut ids: Vec<_> = rows(&state["unresolved_hero_ids"])
            .iter()
            .filter_map(Value::as_str)
            .collect();
        if let Some(acting) = state["resolution_owner_id"]
            .as_str()
            .or_else(|| state["current_actor_id"].as_str())
            && !ids.contains(&acting)
        {
            ids.insert(0, acting);
        }
        let mut entries = Vec::new();
        for id in ids {
            let Some(hero) = players.hero(id) else {
                continue;
            };
            let card = &hero["current_turn_card"];
            if card.is_null() {
                continue;
            }
            let base = if flag(card, "is_facedown") && !reveal_all {
                0
            } else {
                card_initiative(card).as_i64().unwrap_or(0)
            };
            let initiative = self
                .session
                .world
                .computed_stat(self.hero(id)?, Stat::Initiative, base)
                .map_err(super::super::unsupported)?;
            let favored = hero["team"] == state["tie_breaker_team"];
            entries.push((
                initiative,
                favored,
                json!({
                    "hero_id": id,
                    "initiative": initiative,
                    "card": card_view(card, reveal_all || viewer == Some(id)),
                }),
            ));
        }
        let reversed = self
            .session
            .world
            .effects
            .initiative_reversed(self.session.round.round, self.session.world.combat.turn);
        entries.sort_by(|left, right| {
            let priority = if reversed {
                left.0.cmp(&right.0)
            } else {
                right.0.cmp(&left.0)
            };
            if priority == Ordering::Equal {
                right.1.cmp(&left.1)
            } else {
                priority
            }
        });
        Ok(entries.into_iter().map(|(_, _, entry)| entry).collect())
    }
}

struct Players<'a>(Vec<&'a Value>);

impl<'a> Players<'a> {
    fn new(state: &'a Value) -> Self {
        Self(
            fields(&state["teams"])
                .flat_map(|(_, team)| rows(&team["heroes"]))
                .collect(),
        )
    }

    fn hero(&self, id: &str) -> Option<&'a Value> {
        self.0.iter().copied().find(|hero| hero["id"] == id)
    }
}

fn fields(value: &Value) -> impl Iterator<Item = (&String, &Value)> {
    value.as_object().into_iter().flatten()
}

fn rows(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn flag(value: &Value, key: &str) -> bool {
    value[key].as_bool().unwrap_or(false)
}

fn pick(value: &Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .map(|&key| (key.into(), value[key].clone()))
            .collect(),
    )
}

fn winner(state: &Value) -> &Value {
    if state["individual_winner_id"].is_null() {
        &state["winner"]
    } else {
        &state["individual_winner_id"]
    }
}

fn single_active_zone(state: &Value) -> Option<&Value> {
    let zones = state["battle_zones"].as_object()?;
    (zones.len() == 1).then(|| zones.values().next()).flatten()
}

fn team_views(
    state: &Value,
    viewer: Option<&str>,
    second_card_open: bool,
    reveal_all: bool,
) -> Map<String, Value> {
    fields(&state["teams"]).map(|(id, team)| (id.clone(), json!({
        "color": team["color"],
        "life_counters": team["life_counters"],
        "starting_life_counters": team["starting_life_counters"],
        "heroes": rows(&team["heroes"]).iter().map(|hero| hero_view(state, hero, viewer, second_card_open, reveal_all)).collect::<Vec<_>>(),
        "minions": rows(&team["minions"]).iter().map(|minion| {
            let heavy = minion["type"] == "HEAVY";
            json!({"id": minion["id"], "type": minion["type"], "team": minion["team"],
                   "value": if heavy { 4 } else { 2 }, "is_heavy": heavy})
        }).collect::<Vec<_>>(),
    }))).collect()
}

fn hero_view(
    state: &Value,
    hero: &Value,
    viewer: Option<&str>,
    second_card_open: bool,
    reveal_all: bool,
) -> Value {
    let id = hero["id"].as_str().unwrap_or_default();
    let own = reveal_all || viewer == Some(id);
    let upgrade_snapshot = &state["upgrade_reveal_snapshot"][id];
    let items = if own || upgrade_snapshot.is_null() {
        &hero["items"]
    } else {
        upgrade_snapshot
    };
    let first = &state["pending_inputs"][id];
    let second = &state["pending_second_cards"][id];
    let planning = state["phase"] == "PLANNING";
    let extra = if planning && !second.is_null() {
        first
    } else {
        &hero["extra_turn_card"]
    };
    let private_cards = |field| {
        let cards = rows(&hero[field]);
        if own {
            json!(
                cards
                    .iter()
                    .map(|card| card_view(card, true))
                    .collect::<Vec<_>>()
            )
        } else {
            json!({"count": cards.len()})
        }
    };
    let mut view = pick(
        hero,
        &[
            "id",
            "name",
            "title",
            "team",
            "level",
            "gold",
            "wish_cast_count",
            "rune_slots",
        ],
    );
    view["items"] = items.clone();
    view["hand"] = if own {
        private_cards("hand")
    } else {
        json!([])
    };
    view["hand_size"] = json!(rows(&hero["hand"]).len());
    view["deck"] = private_cards("deck");
    let spells = rows(&hero["spells"]);
    let (prepared, cast): (Vec<_>, Vec<_>) = spells
        .iter()
        .partition(|spell| spell["state"] == "SPELLBOOK");
    view["spellbook"] = if spells.is_empty() {
        Value::Null
    } else if own {
        json!(
            prepared
                .into_iter()
                .map(|card| card_view(card, true))
                .collect::<Vec<_>>()
        )
    } else {
        json!({"count": prepared.len()})
    };
    view["cast_spells"] = json!(
        cast.into_iter()
            .map(|card| card_view(card, true))
            .collect::<Vec<_>>()
    );
    view["played_cards"] = json!(
        rows(&hero["played_cards"])
            .iter()
            .map(|card| card_view(card, reveal_all))
            .collect::<Vec<_>>()
    );
    view["discard_pile"] = json!(
        rows(&hero["discard_pile"])
            .iter()
            .map(|card| card_view(card, reveal_all))
            .collect::<Vec<_>>()
    );
    view["current_turn_card"] = card_view(&hero["current_turn_card"], own);
    view["extra_turn_card"] = card_view(extra, own);
    view["can_commit_second_card"] = json!(own && second_card_open);
    view["ultimate_card"] = card_view(&hero["ultimate_card"], own);
    view
}

fn card_initiative(card: &Value) -> &Value {
    card.get("initiative_override")
        .filter(|value| !value.is_null())
        .unwrap_or(&card["initiative"])
}

fn card_view(card: &Value, can_see_face: bool) -> Value {
    if card.is_null() {
        return Value::Null;
    }
    let active = flag(card, "is_active_base")
        || (flag(card, "enraged_active_override") && card["state"] == "RESOLVED");
    if !can_see_face && flag(card, "is_facedown") {
        return json!({
            "tier": "UNTIERED", "color": null, "primary_action": null, "primary_action_value": null,
            "secondary_actions": if card["secondary_actions"].get("HOLD").is_some() { json!({"HOLD": 0}) } else { json!({}) },
            "effect_id": null, "effect_text": "", "initiative": 0, "state": card["state"],
            "is_facedown": true, "item": null, "is_active": active,
        });
    }
    let mut view = pick(
        card,
        &[
            "id",
            "name",
            "image_id",
            "tier",
            "color",
            "primary_action",
            "primary_action_value",
            "secondary_actions",
            "effect_id",
            "effect_text",
            "state",
            "is_facedown",
            "is_ranged",
            "range_value",
            "radius_value",
            "item",
        ],
    );
    view["initiative"] = card_initiative(card).clone();
    view["is_active"] = json!(active);
    if let Some(rank) = card.get("spell_rank") {
        view["spell_rank"] = rank.clone();
    }
    view
}

fn board_view(state: &Value) -> Value {
    let board = &state["board"];
    let tiles: Map<String, Value> = fields(&board["tiles"])
        .map(|(_, tile)| {
            let hex = &tile["hex"];
            let key = format!("{}_{}_{}", hex["q"], hex["r"], hex["s"]);
            (
                key,
                pick(
                    tile,
                    &["hex", "zone_id", "is_terrain", "occupant_id", "spawn_point"],
                ),
            )
        })
        .collect();
    let zones: Map<String, Value> = fields(&board["zones"])
        .map(|(key, zone)| {
            (
                key.clone(),
                pick(zone, &["id", "neighbors", "spawn_points"]),
            )
        })
        .collect();
    json!({"map": board["map_id"], "tiles": tiles, "zones": zones, "entity_locations": state["entity_locations"]})
}

fn battle_zone_minion_counts(state: &Value) -> Map<String, Value> {
    let board = &state["board"];
    fields(&state["battle_zones"])
        .filter_map(|(lane, zone)| {
            let zone = zone.as_str()?;
            if board["zones"].get(zone).is_none() {
                return None;
            }
            let counts = ["RED", "BLUE"].map(|team| {
                let mut seen = HashSet::new();
                rows(&state["teams"][team]["minions"])
                    .iter()
                    .filter_map(|minion| minion["id"].as_str())
                    .filter(|id| seen.insert(*id))
                    .filter(|id| {
                        let hex = &state["entity_locations"][*id];
                        if hex.is_null() {
                            return false;
                        }
                        let key = format!("({}, {}, {})", hex["q"], hex["r"], hex["s"]);
                        board["tiles"][key]["zone_id"] == zone
                    })
                    .count()
            });
            Some((
                lane.clone(),
                json!({"zone_id": zone, "counts": {"RED": counts[0], "BLUE": counts[1]}}),
            ))
        })
        .collect()
}

fn effects_view(state: &Value) -> Vec<Value> {
    rows(&state["active_effects"])
        .iter()
        .map(|effect| {
            let mut view = pick(
                effect,
                &[
                    "id",
                    "source_card_id",
                    "duration",
                    "is_active",
                    "stat_type",
                    "stat_value",
                    "split_axis",
                    "split_value",
                    "named_color",
                ],
            );
            view["type"] = effect["effect_type"].clone();
            let scope = &effect["scope"];
            view["scope"] = pick(scope, &["shape", "range", "origin_id", "affects"]);
            view["scope"]["origin"] = scope["origin_hex"].clone();
            view
        })
        .collect()
}

fn marker_views(state: &Value) -> Map<String, Value> {
    fields(&state["markers"])
        .map(|(id, marker)| {
            (
                id.clone(),
                pick(marker, &["target_id", "value", "source_id"]),
            )
        })
        .collect()
}

fn token_face_visible(token: &Value, viewer: Option<&str>) -> bool {
    !flag(token, "is_facedown") || viewer.is_some_and(|hero| token["owner_id"] == hero)
}

fn token_views(state: &Value, viewer: Option<&str>, reveal_all: bool) -> Vec<Value> {
    let mut tokens = fields(&state["token_pool"])
        .flat_map(|(_, pool)| rows(pool))
        .filter_map(|token| {
            let id = token["id"].as_str()?;
            let location = state["entity_locations"].get(id)?;
            let mut view = pick(
                token,
                &[
                    "id",
                    "name",
                    "token_type",
                    "owner_id",
                    "is_facedown",
                    "is_passable",
                ],
            );
            view["hex"] = location.clone();
            if !reveal_all && !token_face_visible(token, viewer) {
                view["name"] = json!("Mine");
                view["token_type"] = json!("mine");
            }
            Some((id, view))
        })
        .collect::<Vec<_>>();
    tokens.sort_unstable_by_key(|(id, _)| *id);
    tokens.into_iter().map(|(_, view)| view).collect()
}

fn board_entities(state: &Value) -> Vec<Value> {
    fields(&state["misc_entities"])
        .filter(|(_, entity)| entity["entity_kind"] == "turret")
        .map(|(id, entity)| {
            let mut view = pick(
                entity,
                &["id", "name", "entity_kind", "owner_id", "is_obstacle"],
            );
            view["hex"] = state["entity_locations"][id].clone();
            view
        })
        .collect()
}

fn hero_pieces(state: &Value) -> Map<String, Value> {
    fields(&state["misc_entities"])
        .filter(|(_, entity)| entity["entity_kind"] == "hero_piece")
        .map(|(id, piece)| {
            (
                id.clone(),
                json!({"owner_hero_id": piece["owner_hero_id"], "team": piece["team"],
                           "position": state["entity_locations"][id]}),
            )
        })
        .collect()
}

fn public_face(card: &Value) -> Value {
    let mut view = card_view(card, true);
    view["is_facedown"] = json!(false);
    view
}

fn card_guess(state: &Value, players: &Players<'_>) -> Option<Value> {
    let guess = &state["card_guess"];
    let attempts = rows(&guess["attempts"]);
    if attempts.is_empty() {
        return None;
    }
    let attempts = attempts
        .iter()
        .map(|attempt| {
            let mut view = pick(
                attempt,
                &[
                    "attempt",
                    "victim_id",
                    "guessed_color",
                    "actual_color",
                    "correct",
                ],
            );
            let card = if attempt["correct"].is_null() {
                None
            } else {
                attempt["victim_id"]
                    .as_str()
                    .and_then(|id| players.hero(id))
                    .and_then(|hero| {
                        find_card(hero, &attempt["card_id"], &["hand", "discard_pile"])
                    })
            };
            view["card"] = card.map_or(Value::Null, public_face);
            view
        })
        .collect::<Vec<_>>();
    Some(json!({"guesser_id": guess["guesser_id"], "attempts": attempts}))
}

fn card_reveal(state: &Value, players: &Players<'_>) -> Option<Value> {
    let reveal = &state["card_reveal"];
    let hero = players.hero(reveal["owner_id"].as_str()?)?;
    let card = find_card(
        hero,
        &reveal["card_id"],
        &["hand", "discard_pile", "played_cards"],
    )?;
    let mut view = pick(
        reveal,
        &["revealer_id", "target_unit_id", "owner_id", "tier_value"],
    );
    view["card"] = public_face(card);
    view["discarded"] = json!(card["state"] == "DISCARD");
    Some(view)
}

fn find_card<'a>(hero: &'a Value, id: &Value, containers: &[&str]) -> Option<&'a Value> {
    id.as_str()?;
    containers
        .iter()
        .flat_map(|field| rows(&hero[*field]))
        .find(|card| card["id"] == *id)
}
