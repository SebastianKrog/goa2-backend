//! Project pending requests and event metadata across the player visibility boundary.
use super::{LiveGame, Players, Result, fields, flag, pick, rows, token_face_visible};
use crate::{
    interpreter::{RequestChoices, unsupported},
    stats::Stat,
};
use serde_json::{Value, json};
use std::collections::HashSet;

impl LiveGame {
    pub(super) fn client_prompt(
        &self,
        state: &Value,
        players: &Players<'_>,
        viewer: Option<&str>,
    ) -> Result<Option<Value>> {
        let Some((viewer, request)) =
            viewer.and_then(|id| self.request_for(id).map(|request| (id, request)))
        else {
            return Ok(None);
        };
        let Some(kind) = request["request_type"].as_str() else {
            return Ok(None);
        };
        let options = rows(&request["options"]);
        let mut output = json!({
            "request_id": request["id"], "type": kind,
            "prompt": request["prompt"], "player_id": request["player_id"],
        });
        for flag_name in ["can_skip", "can_rollback"] {
            if flag(&request, flag_name) {
                output[flag_name] = json!(true);
            }
        }

        match kind {
            "SELECT_UNIT" | "SELECT_UNIT_OR_TOKEN" | "SELECT_NUMBER" | "SELECT_CARD" => {
                let values = json!(options.iter().map(selection_value).collect::<Vec<_>>());
                output["valid_options"] = values.clone();
                output["candidates"] = values;
                if matches!(kind, "SELECT_NUMBER" | "SELECT_CARD")
                    && options.iter().any(|option| option["id"] != option["text"])
                {
                    output["options"] = json!(
                        options
                            .iter()
                            .map(|option| pick(option, &["id", "text"]))
                            .collect::<Vec<_>>()
                    );
                }
            }
            "SELECT_HEX" | "CHOOSE_RESPAWN_HEX" => {
                let values = json!(options.iter().map(selection_value).collect::<Vec<_>>());
                output["valid_hexes"] = values.clone();
                if kind == "SELECT_HEX" {
                    output["valid_options"] = values;
                }
            }
            "CHOOSE_RESPAWN" | "CONFIRM_PASSIVE" => {
                output["options"] = json!(
                    options
                        .iter()
                        .map(|option| &option["id"])
                        .collect::<Vec<_>>()
                );
            }
            "CHOOSE_ACTOR" => {
                output["player_ids"] = json!(
                    options
                        .iter()
                        .map(|option| &option["id"])
                        .collect::<Vec<_>>()
                );
            }
            _ if !options.is_empty() => {
                let flat = options
                    .iter()
                    .map(|option| {
                        let mut view = pick(option, &["id", "text"]);
                        for (key, value) in fields(&option["metadata"]) {
                            view[key] = value.clone();
                        }
                        view
                    })
                    .collect::<Vec<_>>();
                output["options"] = json!(flat);
            }
            _ => {}
        }
        let root = output.as_object_mut().expect("client request is an object");
        for (key, value) in fields(&request["context"]) {
            root.entry(key.clone()).or_insert_with(|| value.clone());
        }
        if let Some(mut facts) = request["decision_context"].as_object().cloned() {
            if let Some(RequestChoices::Defense(menu)) = self
                .session
                .last_request
                .as_ref()
                .map(|request| &request.choices)
            {
                // A multi-piece hero answers from the owner's seat, but the
                // attack and its minion support belong to the targeted piece.
                let minions = self
                    .session
                    .world
                    .minion_defense_contributions(menu.target, self.session.planning.actor)
                    .map_err(unsupported)?;
                let context = &state["execution_context"];
                let defense_stat = if menu.initiative {
                    Stat::Initiative
                } else {
                    Stat::Defense
                };
                let mut combat = serde_json::Map::new();
                for (key, value) in [
                    (
                        "attacker_id",
                        context["attacker_id"].as_str().map(|value| json!(value)),
                    ),
                    (
                        "is_basic",
                        context["attack_is_basic"]
                            .as_bool()
                            .map(|value| json!(value)),
                    ),
                    (
                        "is_ranged",
                        context["attack_is_ranged"]
                            .as_bool()
                            .map(|value| json!(value)),
                    ),
                ] {
                    if let Some(value) = value {
                        combat.insert(key.into(), value);
                    }
                }
                combat.insert("defense_stat".into(), json!(defense_stat));
                for (key, friendly) in [("friendly_minions", true), ("enemy_minions", false)] {
                    combat.insert(
                        key.into(),
                        Value::Object(
                            minions
                                .counts(friendly)
                                .map(|(kind, count)| (kind.into(), json!(count)))
                                .collect(),
                        ),
                    );
                }
                facts.insert("combat".into(), Value::Object(combat));
                facts.insert("source_card_id".into(), self.defense_source(state));
            }
            let offered = |id: &str| options.iter().any(|option| option["id"] == id);
            let private = HiddenCards::new(players, Some(viewer));
            facts.retain(|key, value| {
                if key == "source_card_id" {
                    return value
                        .as_str()
                        .is_some_and(|id| offered(id) || private.visible(id) == Some(true));
                }
                !value.is_null()
            });
            if let Some(choices) = facts.get_mut("choice_facts").and_then(Value::as_object_mut) {
                choices.retain(|id, _| offered(id));
                for choice in choices.values_mut().filter_map(Value::as_object_mut) {
                    choice.retain(|_, value| !value.is_null());
                }
            }
            output["decision_context"] = Value::Object(facts);
        }
        Ok(Some(output))
    }

    fn defense_source(&self, state: &Value) -> Value {
        let context = &state["execution_context"];
        if let Some(source) = context.get("attack_source_card_id") {
            return source.clone();
        }
        let session = &self.session;
        let world = &session.world;
        let symbol = |value: &Value| value.as_str().and_then(|id| session.symbols.lookup(id));
        let performing = symbol(&context["performing_card_id"]).and_then(|printed| {
            symbol(&context["performing_card_owner_id"])
                .and_then(|owner| world.card_for_hero_printed(owner, printed))
                .or_else(|| world.card_by_printed(printed))
        });
        let card = performing.or_else(|| {
            symbol(&context["attacker_id"])
                .filter(|actor| !session.symbols.name(*actor).is_empty())
                .or(session.planning.actor)
                .and_then(|actor| world.hero(actor)?.current)
        });
        card.and_then(|card| world.card(card))
            .map(|card| {
                json!(
                    session
                        .symbols
                        .name(world.definitions.cards[card.definition].printed_id)
                )
            })
            .unwrap_or(Value::Null)
    }
}

fn selection_value(option: &Value) -> Value {
    for key in ["hex", "raw"] {
        if let Some(value) = option["metadata"].get(key) {
            return value.clone();
        }
    }
    option["id"]
        .as_str()
        .and_then(|id| id.parse::<i64>().ok())
        .map_or_else(|| option["id"].clone(), |number| json!(number))
}

struct HiddenCards<'a> {
    hidden: HashSet<&'a str>,
    known: HashSet<&'a str>,
    names: HashSet<&'a str>,
}

impl<'a> HiddenCards<'a> {
    fn new(players: &Players<'a>, viewer: Option<&str>) -> Self {
        let mut cards = Self {
            hidden: HashSet::new(),
            known: HashSet::new(),
            names: HashSet::new(),
        };
        for &hero in &players.0 {
            let own = hero["id"].as_str().is_some_and(|id| viewer == Some(id));
            for (field, private_zone, owner_exception) in [
                ("hand", true, true),
                ("current_turn_card", false, true),
                ("extra_turn_card", false, true),
                ("played_cards", false, false),
                ("discard_pile", false, false),
                ("ultimate_card", false, true),
                ("spells", false, true),
                ("deck", true, true),
            ] {
                let value = &hero[field];
                let entries = value
                    .as_array()
                    .map_or_else(|| std::slice::from_ref(value), Vec::as_slice);
                for card in entries {
                    let Some(id) = card["id"].as_str() else {
                        continue;
                    };
                    let prepared = field == "spells" && card["state"] == "SPELLBOOK";
                    let own_face = own && owner_exception && (field != "spells" || prepared);
                    if cards.known.insert(id)
                        && (private_zone || prepared || flag(card, "is_facedown"))
                        && !own_face
                    {
                        cards.hidden.insert(id);
                        if let Some(name) = card["name"].as_str() {
                            cards.names.insert(name);
                        }
                    }
                }
            }
        }
        cards
    }

    fn visible(&self, id: &str) -> Option<bool> {
        self.known.contains(id).then(|| !self.hidden.contains(id))
    }

    fn sanitize(&self, value: &mut Value, card_field: bool) {
        match value {
            Value::String(name)
                if self.hidden.contains(name.as_str())
                    || (card_field && self.names.contains(name.as_str())) =>
            {
                *value = Value::Null;
            }
            Value::Object(fields) => {
                for (key, value) in fields {
                    self.sanitize(
                        value,
                        card_field || key.contains("card") || key.contains("spell"),
                    );
                }
            }
            Value::Array(items) => {
                for item in &mut *items {
                    self.sanitize(item, card_field);
                }
                if card_field {
                    items.retain(|item| !item.is_null());
                }
            }
            _ => {}
        }
    }
}

pub(super) fn visible_events(
    events: &[Value],
    state: &Value,
    players: &Players<'_>,
    viewer: Option<&str>,
) -> Vec<Value> {
    let private = HiddenCards::new(players, viewer);
    events
        .iter()
        .map(|source| {
            let mut event = source.clone();
            match source["event_type"].as_str() {
                // These engine events expressly expose a card face to every viewer.
                Some("GUESSED_CARD_REVEALED" | "CARD_REVEALED") => return event,
                _ => {}
            }
            private.sanitize(&mut event["metadata"], false);
            if source["event_type"] != "MINE_TRIGGERED"
                && let Some(token) = source["target_id"]
                    .as_str()
                    .and_then(|id| state["misc_entities"].get(id))
                && token["entity_kind"] == "token"
                && !token_face_visible(token, viewer)
                && let Some(kind) = event["metadata"].get_mut("token_type")
            {
                *kind = json!("mine");
            }
            event
        })
        .collect()
}
