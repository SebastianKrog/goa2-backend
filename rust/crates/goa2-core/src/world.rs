//! Dense native entity/card tables and independent mutable spatial state.
//! Definitions and geometry are shared. JSON is read/written only at the boundary.
use crate::{
    cards::{CardId, HeroRecord},
    codec::{InstanceKind, StateEnvelope},
    data::{Datum, Symbol, Symbols},
    geometry::{BoardGeometry, Hex, Occupancy, TileSpec},
    stats::NativeMarker,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CardLifecycle {
    Hand,
    Deck,
    Discard,
    Unresolved,
    Resolved,
    Item,
    Passive,
    Retired,
    Spellbook,
    OutsideSpellbook,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActionKind {
    Movement,
    Attack,
    Skill,
    Defense,
    DefenseSkill,
    Hold,
    Clear,
    FastTravel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MinionKind {
    Melee,
    Ranged,
    Heavy,
}
#[derive(Clone, Debug)]
pub struct CardDefinition {
    pub name: Symbol,
    pub printed_id: Symbol,
    pub effect: Symbol,
    pub has_effect: bool,
    pub effect_absent: bool,
    pub aura: crate::auras::CardAura,
    pub action_prevention: bool,
    pub has_item: bool,
    pub item: Option<crate::stats::Stat>,
    pub color: Option<Symbol>,
    pub tier: Symbol,
    pub range: Option<i64>,
    pub spell: bool,
    pub primary: Option<ActionKind>,
    pub primary_value: Option<i64>,
    pub ranged: bool,
    pub radius: Option<i64>,
    pub initiative: i64,
    pub secondary: Vec<(ActionKind, i64)>,
    pub basic: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct CardEquality {
    pub model: Symbol,
    pub revision: i64,
    pub json_metadata: bool,
}
#[derive(Clone, Debug)]
pub struct CardInstance {
    pub definition: usize,
    pub source_instance: u32,
    pub equality: Option<CardEquality>,
    pub lifecycle: CardLifecycle,
    pub facedown: bool,
    pub played_this_round: bool,
    pub active: bool,
    pub enraged: bool,
    pub passive_uses: i64,
    pub initiative_override: Option<i64>,
    pub metadata: Datum,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityKind {
    Hero,
    Minion,
    Piece,
    Token,
    Other,
}
#[derive(Clone, Debug)]
pub struct EntityDefinition {
    pub id: Symbol,
    pub name: Symbol,
    pub kind: EntityKind,
    pub owner: Option<Symbol>,
    pub team: Option<Symbol>,
    pub lane: Option<Symbol>,
    pub roster: Option<Symbol>,
    pub token_type: Option<Symbol>,
    pub immune_enemy_actions: bool,
    pub has_team: bool,
    pub passable: bool,
    pub multi_piece: bool,
    pub minion_kind: Option<MinionKind>,
}
#[derive(Clone, Debug)]
pub struct WorldDefinitions {
    pub entities: Vec<EntityDefinition>,
    pub cards: Vec<CardDefinition>,
    pub(crate) card_equality_fields: Vec<Datum>,
    pub(crate) card_model: Symbol,
    pub(crate) spell_model: Symbol,
    pub zones: crate::zones::ZoneGraph,
    pub(crate) stat_context: crate::stats::PerformingCardKeys,
    pub(crate) effect_empty: Symbol,
    pub(crate) tide_of_darkness: Symbol,
    pub(crate) illusion_token: Symbol,
    pub(crate) targeting: crate::targeting::TargetingKeys,
    pub(crate) supply_passable: Vec<(Symbol, bool)>,
    pub(crate) mine_supply: Vec<(Symbol, bool, Option<Symbol>)>,
    geometry: BoardGeometry,
    tile_keys: Vec<String>,
    tile_zones: Vec<Option<Symbol>>,
    tile_spawns: Vec<Option<bool>>,
    tile_spawn_teams: Vec<Option<Symbol>>,
    tile_hexes: Vec<Hex>,
    loaded_map: bool,
}
#[derive(Clone, Debug)]
pub struct NativeWorld {
    pub(crate) history: crate::history::CardHistory,
    pub(crate) combat: crate::combat::CombatState,
    pub definitions: Arc<WorldDefinitions>,
    pub cards: Vec<CardInstance>,
    pub card_by_instance: Vec<Option<usize>>,
    entity_by_symbol: Vec<Option<usize>>,
    entity_overrides: Arc<HashMap<usize, EntityDefinition>>,
    board_entity_overrides: Arc<HashMap<Symbol, EntityDefinition>>,
    pub(crate) locations: Vec<(Symbol, Hex)>,
    location_by_symbol: Vec<Option<usize>>,
    occupants: Vec<Option<Symbol>>,
    occupancy: Occupancy,
    /// At least one currently active card has an unported aura hook.
    pub(crate) has_uncompiled_auras: bool,
    pub(crate) acting_piece: Option<Symbol>,
    pub(crate) battle_zones: Vec<(Symbol, Symbol)>,
    pub(crate) waves: Vec<(Symbol, i64)>,
    pub(crate) heroes: Vec<HeroRecord>,
    hero_by_symbol: Vec<Option<usize>>,
    pub(crate) markers: Vec<NativeMarker>,
    pub(crate) has_effects: bool,
    pub effects: crate::effects::EffectStore,
    pub(crate) next_entity_id: i64,
}
impl NativeWorld {
    pub fn compile(source: &StateEnvelope, symbols: &mut Symbols) -> Result<Self, String> {
        if !source.object_bindings.is_empty() {
            return Err("dense world shared-object bindings are not yet compiled".into());
        }
        source.validate()?;
        let semantic = source.materialize()?;
        let mut entities = Vec::new();
        for (roster_name, team) in semantic["teams"]
            .as_object()
            .ok_or("teams are missing")?
            .iter()
        {
            let roster = symbols.intern(roster_name);
            for hero in team["heroes"].as_array().ok_or("heroes are missing")? {
                entities.push(entity(hero, EntityKind::Hero, symbols)?);
            }
            for minion in team["minions"].as_array().ok_or("minions are missing")? {
                let mut def = entity(minion, EntityKind::Minion, symbols)?;
                def.roster = Some(roster);
                entities.push(def);
            }
        }
        if let Some(misc) = semantic["misc_entities"].as_object() {
            for value in misc.values() {
                let kind = if value.get("token_type").is_some() {
                    EntityKind::Token
                } else if value["entity_kind"] == "hero_piece" {
                    EntityKind::Piece
                } else {
                    EntityKind::Other
                };
                entities.push(entity(value, kind, symbols)?);
            }
        }
        let mut supply_passable = Vec::new();
        let mut mine_supply = Vec::new();
        for tokens in semantic["token_pool"]
            .as_object()
            .ok_or("token supply missing")?
            .values()
        {
            for token in tokens.as_array().ok_or("token supply list missing")? {
                supply_passable.push((
                    symbols.intern(token["id"].as_str().ok_or("token id missing")?),
                    token["is_passable"]
                        .as_bool()
                        .ok_or("token passability missing")?,
                ));
                mine_supply.push((
                    symbols.intern(token["id"].as_str().ok_or("token id missing")?),
                    token["is_passable"]
                        .as_bool()
                        .ok_or("token passability missing")?,
                    token["owner_id"].as_str().map(|id| symbols.intern(id)),
                ));
            }
        }
        let aura_catalog = crate::templates::instructions()?["aura_effects"]
            .as_array()
            .ok_or("aura catalog missing")?;
        let mut definition_map = vec![None; source.definitions.len()];
        let mut card_equality_fields = Vec::new();
        let mut card_definitions = Vec::new();
        for (index, definition) in source.definitions.iter().enumerate() {
            if definition.kind == InstanceKind::Token {
                continue;
            }
            let fields = &definition.fields;
            let printed_id = symbols.intern(fields["id"].as_str().ok_or("card identity missing")?);
            let effect = symbols.intern(match fields.get("effect_id") {
                Some(Value::Null) => "",
                Some(Value::String(value)) => value,
                _ => return Err("card effect missing".into()),
            });
            let color = fields["color"].as_str().map(|v| symbols.intern(v));
            let tier = symbols.intern(fields["tier"].as_str().ok_or("card tier missing")?);
            let range = fields.get("range_value").and_then(Value::as_i64);
            definition_map[index] = Some(card_definitions.len());
            card_equality_fields.push(Datum::from_json(&Value::Object(fields.clone()), symbols)?);
            card_definitions.push(CardDefinition {
                name: symbols.intern(fields["name"].as_str().ok_or("card name missing")?),
                printed_id,
                effect,
                has_item: fields.get("item").is_some_and(|v| !v.is_null()),
                item: fields
                    .get("item")
                    .and_then(Value::as_str)
                    .map(crate::stats::Stat::parse)
                    .transpose()?,
                has_effect: !fields["effect_id"].as_str().unwrap_or("").is_empty(),
                effect_absent: fields["effect_id"].is_null(),
                aura: crate::auras::CardAura::compile(
                    fields["effect_id"].as_str().unwrap_or(""),
                    aura_catalog.iter().any(|id| id == &fields["effect_id"]),
                ),
                action_prevention: crate::templates::instructions()?["action_prevention_effects"]
                    .as_array()
                    .ok_or("action prevention catalog missing")?
                    .iter()
                    .any(|id| id == &fields["effect_id"]),
                color,
                tier,
                range,
                spell: definition.kind == InstanceKind::Spell,
                primary: fields
                    .get("primary_action")
                    .filter(|v| !v.is_null())
                    .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
                    .transpose()?,
                primary_value: fields.get("primary_action_value").and_then(Value::as_i64),
                ranged: fields["is_ranged"].as_bool().ok_or("ranged flag missing")?,
                radius: fields.get("radius_value").and_then(Value::as_i64),
                initiative: fields["initiative"].as_i64().ok_or("initiative missing")?,
                basic: fields["tier"] == "UNTIERED",
                secondary: fields["secondary_actions"]
                    .as_object()
                    .ok_or("secondary actions missing")?
                    .iter()
                    .map(|(key, value)| {
                        Ok((
                            serde_json::from_value(json!(key)).map_err(|e| e.to_string())?,
                            value.as_i64().ok_or("secondary value missing")?,
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?,
            });
        }
        let mut cards = Vec::new();
        let mut card_by_instance = vec![None; source.instances.len()];
        for (instance_id, instance) in source.instances.iter().enumerate() {
            let Some(definition) = definition_map[instance.definition as usize] else {
                continue;
            };
            let fields = &instance.fields;
            card_by_instance[instance_id] = Some(cards.len());
            cards.push(CardInstance {
                definition,
                equality: instance.equality.as_ref().map(|equality| CardEquality {
                    model: symbols.intern(&equality.model),
                    revision: equality.definition_revision,
                    json_metadata: equality.json_metadata,
                }),
                source_instance: u32::try_from(instance_id)
                    .map_err(|_| "too many physical instances")?,
                lifecycle: serde_json::from_value(fields["state"].clone())
                    .map_err(|e| format!("invalid card lifecycle: {e}"))?,
                facedown: fields["is_facedown"]
                    .as_bool()
                    .ok_or("facedown flag missing")?,
                played_this_round: fields["played_this_round"]
                    .as_bool()
                    .ok_or("played flag missing")?,
                active: fields["is_active_base"]
                    .as_bool()
                    .ok_or("active flag missing")?,
                enraged: fields["enraged_active_override"]
                    .as_bool()
                    .ok_or("enraged flag missing")?,
                passive_uses: fields["passive_uses_this_turn"]
                    .as_i64()
                    .ok_or("passive counter missing")?,
                initiative_override: fields.get("initiative_override").and_then(Value::as_i64),
                metadata: Datum::from_json(
                    fields.get("metadata").unwrap_or(&Value::Null),
                    symbols,
                )?,
            });
        }
        let mut heroes = Vec::new();
        for (team, value) in source.state["teams"].as_object().ok_or("teams missing")? {
            let team = symbols.intern(team);
            for (index, raw) in value["heroes"]
                .as_array()
                .ok_or("heroes missing")?
                .iter()
                .enumerate()
            {
                let hero = HeroRecord::compile(raw, team, index, symbols)?;
                for id in hero
                    .deck
                    .iter()
                    .chain(&hero.hand)
                    .chain(&hero.discard)
                    .chain(&hero.spells)
                    .chain(hero.played.iter().flatten())
                    .chain(hero.current.iter())
                    .chain(hero.extra.iter())
                    .chain(hero.ultimate.iter())
                {
                    if card_by_instance
                        .get(id.0 as usize)
                        .and_then(|v| *v)
                        .is_none()
                    {
                        return Err("hero collection contains a non-card instance".into());
                    }
                }
                heroes.push(hero);
            }
        }
        let mut markers = Vec::new();
        for (key, marker) in semantic["markers"]
            .as_object()
            .ok_or("marker registry missing")?
            .iter()
        {
            markers.push(NativeMarker {
                key: symbols.intern(key),
                source: marker["source_id"].as_str().map(|s| symbols.intern(s)),
                kind: serde_json::from_value(marker["type"].clone()).map_err(|e| e.to_string())?,
                target: marker["target_id"].as_str().map(|v| symbols.intern(v)),
                value: marker["value"].as_i64().ok_or("marker magnitude missing")?,
            });
        }
        let mut tiles = Vec::new();
        let mut occupants = Vec::new();
        let mut tile_keys = Vec::new();
        let mut tile_zones = Vec::new();
        let mut tile_spawns = Vec::new();
        let mut tile_spawn_teams = Vec::new();
        for (key, value) in semantic["board"]["tiles"]
            .as_object()
            .ok_or("board tiles missing")?
        {
            let hex =
                serde_json::from_value::<Hex>(value["hex"].clone()).map_err(|e| e.to_string())?;
            let occupant = value["occupant_id"].as_str().map(|v| symbols.intern(v));
            tiles.push(TileSpec {
                hex,
                terrain: value["is_terrain"]
                    .as_bool()
                    .ok_or("terrain flag missing")?,
                occupied: occupant.is_some(),
            });
            occupants.push(occupant);
            tile_spawns.push(match value.get("spawn_point").filter(|v| !v.is_null()) {
                None => None,
                Some(spawn) => Some(match spawn["type"].as_str() {
                    Some("MINION") => true,
                    Some("HERO") => false,
                    _ => return Err("invalid tile spawn type".into()),
                }),
            });
            tile_spawn_teams.push(
                value["spawn_point"]["team"]
                    .as_str()
                    .map(|id| symbols.intern(id)),
            );
            tile_keys.push(key.clone());
            tile_zones.push(value["zone_id"].as_str().map(|s| symbols.intern(s)));
        }
        let (geometry, occupancy) = BoardGeometry::compile(&tiles)?;
        let mut locations = Vec::new();
        for (id, hex) in semantic["entity_locations"]
            .as_object()
            .ok_or("entity locations missing")?
        {
            locations.push((
                symbols.intern(id),
                serde_json::from_value(hex.clone()).map_err(|e| e.to_string())?,
            ));
        }
        let history = crate::history::CardHistory::compile(&semantic, symbols)?;
        let combat = crate::combat::CombatState::compile(&semantic, symbols)?;
        let battle_zones = semantic["battle_zones"]
            .as_object()
            .ok_or("battle zones missing")?
            .iter()
            .map(|(lane, zone)| {
                Ok((
                    symbols.intern(lane),
                    symbols.intern(zone.as_str().ok_or("invalid battle zone")?),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let waves = semantic["wave_counters"]
            .as_object()
            .ok_or("wave counters missing")?
            .iter()
            .map(|(lane, count)| {
                Ok((
                    symbols.intern(lane),
                    count.as_i64().ok_or("invalid wave count")?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let zones = crate::zones::ZoneGraph::compile(&semantic["board"], symbols)?;
        let acting_piece = semantic["acting_piece_id"]
            .as_str()
            .map(|v| symbols.intern(v));
        let mut hero_by_symbol = vec![None; symbols.len()];
        for (i, hero) in heroes.iter().enumerate() {
            hero_by_symbol[hero.id.0] = Some(i);
        }
        let mut entity_by_symbol = vec![None; symbols.len()];
        for (i, definition) in entities.iter().enumerate() {
            if entity_by_symbol[definition.id.0].replace(i).is_some() {
                return Err("duplicate physical entity identity".into());
            }
        }
        let mut location_by_symbol = vec![None; symbols.len()];
        for (i, (id, _)) in locations.iter().enumerate() {
            location_by_symbol[id.0] = Some(i);
        }
        let mut world = Self {
            next_entity_id: semantic["next_entity_id"]
                .as_i64()
                .ok_or("invalid entity counter")?,
            combat,
            history,
            definitions: Arc::new(WorldDefinitions {
                entities,
                zones,
                stat_context: crate::stats::PerformingCardKeys::compile(symbols),
                tide_of_darkness: symbols.intern("tide_of_darkness"),
                illusion_token: symbols.intern("illusion"),
                targeting: crate::targeting::TargetingKeys::compile(symbols),
                supply_passable,
                mine_supply,
                effect_empty: symbols.intern(""),
                cards: card_definitions,
                card_equality_fields,
                card_model: symbols.intern("goa2.domain.models.card.Card"),
                spell_model: symbols.intern("goa2.domain.models.spell.SpellCard"),
                geometry,
                tile_keys,
                tile_zones,
                tile_spawns,
                tile_spawn_teams,
                tile_hexes: tiles.iter().map(|t| t.hex).collect(),
                loaded_map: semantic["board"]["map_id"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty()),
            }),
            cards,
            card_by_instance,
            entity_by_symbol,
            entity_overrides: Arc::new(HashMap::new()),
            board_entity_overrides: Arc::new(HashMap::new()),
            locations,
            location_by_symbol,
            occupants,
            occupancy,
            has_uncompiled_auras: false,
            acting_piece,
            battle_zones,
            waves,
            heroes,
            hero_by_symbol,
            markers,
            effects: crate::effects::EffectStore::compile(&semantic["active_effects"], symbols)?,
            has_effects: semantic["active_effects"]
                .as_array()
                .is_some_and(|v| !v.is_empty()),
        };
        world.refresh_card_auras();
        Ok(world)
    }
    pub fn hero_index(&self, id: Symbol) -> Option<usize> {
        self.hero_by_symbol.get(id.0).and_then(|v| *v).or_else(|| {
            self.entity(id)
                .filter(|entity| entity.kind == EntityKind::Piece)
                .and_then(|entity| entity.owner)
                .and_then(|owner| self.hero_by_symbol.get(owner.0).and_then(|v| *v))
        })
    }
    pub fn hero(&self, id: Symbol) -> Option<&HeroRecord> {
        self.hero_index(id).map(|i| &self.heroes[i])
    }
    pub fn card(&self, id: CardId) -> Option<&CardInstance> {
        self.card_by_instance
            .get(id.0 as usize)
            .and_then(|v| *v)
            .map(|i| &self.cards[i])
    }
    pub(crate) fn card_mut(&mut self, id: CardId) -> Option<&mut CardInstance> {
        self.card_by_instance
            .get(id.0 as usize)
            .and_then(|v| *v)
            .map(|i| &mut self.cards[i])
    }
    pub fn export_cards(&self, source: &mut StateEnvelope, symbols: &Symbols) {
        for hero in &self.heroes {
            hero.export_collections(&mut source.state, symbols);
        }
        for card in &self.cards {
            let instance = &mut source.instances[card.source_instance as usize];
            instance.equality = card.equality.map(|value| crate::codec::InstanceEquality {
                model: symbols.name(value.model).into(),
                definition_revision: value.revision,
                json_metadata: value.json_metadata,
            });
            let fields = &mut instance.fields;
            fields.insert("state".into(), json!(card.lifecycle));
            fields.insert("is_facedown".into(), json!(card.facedown));
            fields.insert("played_this_round".into(), json!(card.played_this_round));
            fields.insert("is_active_base".into(), json!(card.active));
            fields.insert("enraged_active_override".into(), json!(card.enraged));
            fields.insert("passive_uses_this_turn".into(), json!(card.passive_uses));
            fields.insert("metadata".into(), card.metadata.to_json(symbols));
            if let Some(value) = card.initiative_override {
                fields.insert("initiative_override".into(), json!(value));
            } else {
                fields.shift_remove("initiative_override");
            }
        }
    }
    pub(crate) fn entity_index(&self, id: Symbol) -> Option<usize> {
        self.entity_by_symbol.get(id.0).and_then(|index| *index)
    }
    pub fn entity(&self, id: Symbol) -> Option<&EntityDefinition> {
        if let Some(entity) = self.board_entity_overrides.get(&id) {
            return Some(entity);
        }
        self.entity_by_symbol
            .get(id.0)
            .and_then(|v| *v)
            .map(|index| {
                self.entity_overrides
                    .get(&index)
                    .unwrap_or(&self.definitions.entities[index])
            })
    }
    pub(crate) fn upsert_board_entity(
        &mut self,
        id: Symbol,
        name: Symbol,
        owner: Option<Symbol>,
    ) -> Result<(), String> {
        let mut entity = if let Some(row) = self.entity(id) {
            if row.kind != EntityKind::Other {
                return Err("board entity identity collides with a unit/token".into());
            }
            row.clone()
        } else {
            EntityDefinition {
                id,
                name,
                kind: EntityKind::Other,
                owner: None,
                team: None,
                lane: None,
                roster: None,
                token_type: None,
                immune_enemy_actions: false,
                has_team: false,
                passable: false,
                multi_piece: false,
                minion_kind: None,
            }
        };
        entity.name = name;
        entity.owner = owner;
        Arc::make_mut(&mut self.board_entity_overrides).insert(id, entity);
        Ok(())
    }
    pub(crate) fn set_token_properties(
        &mut self,
        id: Symbol,
        owner: Option<Symbol>,
        immune: bool,
    ) -> Result<(), String> {
        let index = self
            .entity_by_symbol
            .get(id.0)
            .and_then(|v| *v)
            .ok_or("token entity missing")?;
        let mut row = self.entity(id).ok_or("token entity missing")?.clone();
        if row.kind != EntityKind::Token {
            return Err("token property target is not a token".into());
        }
        row.owner = owner;
        row.immune_enemy_actions = immune;
        Arc::make_mut(&mut self.entity_overrides).insert(index, row);
        Ok(())
    }
    pub fn position(&self, id: Symbol) -> Option<Hex> {
        self.location_by_symbol
            .get(id.0)
            .and_then(|v| *v)
            .map(|index| self.locations[index].1)
    }
    pub fn resolved_actor(&self, id: Symbol) -> Symbol {
        if self.entity(id).is_some_and(|entity| entity.multi_piece)
            && let Some(piece) = self.acting_piece
            && self
                .entity(piece)
                .is_some_and(|entity| entity.owner == Some(id))
        {
            return piece;
        }
        id
    }
    /// Owner-level multi-piece presence never uses a bound acting piece or a
    /// stray owner location; single-piece entities use their direct location.
    pub fn piece_ids(&self, id: Symbol) -> impl Iterator<Item = Symbol> + '_ {
        let multi = self.entity(id).is_some_and(|e| e.multi_piece);
        let pieces = if multi {
            self.hero(id).map(|h| h.pieces.as_slice())
        } else {
            None
        };
        let direct = if multi {
            None
        } else {
            self.position(id).map(|_| id)
        };
        direct.into_iter().chain(
            pieces
                .into_iter()
                .flatten()
                .copied()
                .filter(|p| self.position(*p).is_some()),
        )
    }
    pub fn positions(&self, id: Symbol) -> impl Iterator<Item = Hex> + '_ {
        let multi = self.entity(id).is_some_and(|e| e.multi_piece);
        let pieces = if multi {
            self.hero(id).map(|h| h.pieces.as_slice())
        } else {
            None
        };
        let direct = if multi { None } else { self.position(id) };
        direct.into_iter().chain(
            pieces
                .into_iter()
                .flatten()
                .filter_map(|p| self.position(*p)),
        )
    }
    pub fn has_board_presence(&self, id: Symbol) -> bool {
        self.positions(id).next().is_some()
    }
    pub fn resolved_position(&self, id: Symbol) -> Option<Hex> {
        self.position(id)
            .or_else(|| self.position(self.resolved_actor(id)))
    }
    pub fn zone_at(&self, hex: Hex) -> Option<Symbol> {
        if let Some(id) = self.definitions.geometry.id(hex) {
            self.definitions.tile_zones[id.index()]
        } else {
            self.definitions.zones.zone_at(hex)
        }
    }
    pub fn occupant(&self, hex: Hex) -> Option<Symbol> {
        self.definitions
            .geometry
            .id(hex)
            .and_then(|id| self.occupants[id.index()])
    }
    pub fn reachable(&self, origin: Hex, range: i32, through: bool) -> Vec<bool> {
        let query = crate::geometry::MovementQuery {
            start: origin,
            ends: self.tile_hexes().to_vec(),
            max_steps: range,
            ignore_obstacles: through,
            pass_through_obstacles: through,
        };
        let mut values = self
            .definitions
            .geometry
            .stateful_paths(&self.occupancy, &query);
        for (index, hex) in self.tile_hexes().iter().enumerate() {
            if *hex == origin {
                values[index] = true;
            }
        }
        values
    }
    pub fn ensure_move_closure(&self, is_action: bool) -> Result<(), String> {
        if is_action && self.has_uncompiled_auras {
            return Err("card aura action restrictions are not compiled".into());
        }
        Ok(())
    }
    pub fn path(&self, start: Hex, end: Hex, range: i32, through: bool) -> bool {
        self.definitions
            .geometry
            .stateful_path(&self.occupancy, start, end, range, through)
    }
    pub(crate) fn tile_index(&self, hex: Hex) -> Option<usize> {
        self.definitions.geometry.id(hex).map(|id| id.index())
    }
    pub fn on_map(&self, hex: Hex) -> bool {
        self.definitions.geometry.id(hex).is_some()
    }
    pub fn terrain(&self, hex: Hex) -> bool {
        self.definitions.geometry.terrain(hex)
    }
    pub fn obstacle(&self, hex: Hex) -> bool {
        self.definitions.geometry.obstacle(hex, &self.occupancy)
    }

    pub fn spawn_at(&self, hex: Hex, minion_only: bool) -> bool {
        self.definitions
            .geometry
            .id(hex)
            .and_then(|id| self.definitions.tile_spawns[id.index()])
            .is_some_and(|minion| !minion_only || minion)
    }
    pub(crate) fn minion_spawn_team(&self, hex: Hex) -> Option<Symbol> {
        self.spawn_at(hex, true)
            .then(|| {
                self.definitions
                    .geometry
                    .id(hex)
                    .and_then(|id| self.definitions.tile_spawn_teams[id.index()])
            })
            .flatten()
    }
    pub fn tile_in_battle_zone(&self, hex: Hex) -> bool {
        self.definitions
            .geometry
            .id(hex)
            .and_then(|id| self.definitions.tile_zones[id.index()])
            .is_some_and(|zone| self.battle_zones.iter().any(|(_, z)| *z == zone))
    }
    pub fn has_loaded_map(&self) -> bool {
        self.definitions.loaded_map
    }
    pub fn place_entity(&mut self, id: Symbol, end: Hex) -> Result<(), String> {
        if self.position(id).is_some() {
            return self.move_entity(id, end);
        }
        let target = self.definitions.geometry.id(end);
        if self.definitions.loaded_map && target.is_none() {
            return Err("place destination is outside loaded map".into());
        }
        if let Some(tile) = target
            && self.occupants[tile.index()].is_some_and(|other| other != id)
        {
            return Err("place destination is occupied".into());
        }
        if id.0 >= self.location_by_symbol.len() {
            self.location_by_symbol.resize(id.0 + 1, None);
        }
        self.location_by_symbol[id.0] = Some(self.locations.len());
        self.locations.push((id, end));
        if let Some(tile) = target {
            self.occupants[tile.index()] = Some(id);
            self.definitions
                .geometry
                .set_occupied(&mut self.occupancy, end, true)?;
        }
        Ok(())
    }
    pub fn move_entity(&mut self, id: Symbol, end: Hex) -> Result<(), String> {
        let geometry = &self.definitions.geometry;
        let target = geometry.id(end);
        if self.definitions.loaded_map && target.is_none() {
            return Err("move destination is outside loaded map".into());
        }
        if let Some(tile) = target
            && self.occupants[tile.index()].is_some_and(|other| other != id)
        {
            return Err("move destination is occupied".into());
        }
        if let Some(old) = self.position(id)
            && let Some(tile) = geometry.id(old)
            && self.occupants[tile.index()] == Some(id)
        {
            self.occupants[tile.index()] = None;
            geometry.set_occupied(&mut self.occupancy, old, false)?;
        }
        if let Some(index) = self.location_by_symbol.get(id.0).and_then(|v| *v) {
            self.locations[index].1 = end;
        } else {
            return Err("moving an unplaced entity is not a movement transition".into());
        }
        if let Some(tile) = target {
            self.occupants[tile.index()] = Some(id);
            geometry.set_occupied(&mut self.occupancy, end, true)?;
        }
        Ok(())
    }
    pub fn remove_entity(&mut self, id: Symbol) -> Result<Option<Hex>, String> {
        let Some(index) = self.location_by_symbol.get(id.0).and_then(|v| *v) else {
            return Ok(None);
        };
        let from = self.locations.remove(index).1;
        self.location_by_symbol[id.0] = None;
        for (index, (uid, _)) in self.locations.iter().enumerate() {
            self.location_by_symbol[uid.0] = Some(index);
        }
        if let Some(tile) = self.definitions.geometry.id(from)
            && self.occupants[tile.index()] == Some(id)
        {
            self.occupants[tile.index()] = None;
            self.definitions
                .geometry
                .set_occupied(&mut self.occupancy, from, false)?;
        }
        Ok(Some(from))
    }
    pub fn export_spatial(&self, state: &mut Value, symbols: &Symbols) {
        state["active_effects"] = self.effects.export(symbols);
        state["next_entity_id"] = json!(self.next_entity_id);
        state["battle_zones"] = Value::Object(
            self.battle_zones
                .iter()
                .map(|(l, z)| (symbols.name(*l).into(), json!(symbols.name(*z))))
                .collect(),
        );
        state["wave_counters"] = Value::Object(
            self.waves
                .iter()
                .map(|(l, n)| (symbols.name(*l).into(), json!(n)))
                .collect(),
        );
        state["acting_piece_id"] = self
            .acting_piece
            .map_or(Value::Null, |p| json!(symbols.name(p)));
        state["entity_locations"] = Value::Object(
            self.locations
                .iter()
                .map(|(id, hex)| (symbols.name(*id).to_owned(), json!(hex)))
                .collect(),
        );
        for (i, key) in self.definitions.tile_keys.iter().enumerate() {
            state["board"]["tiles"][key]["occupant_id"] =
                self.occupants[i].map_or(Value::Null, |id| Value::from(symbols.name(id)));
        }
    }
    pub fn tile_hexes(&self) -> &[Hex] {
        &self.definitions.tile_hexes
    }
}
fn entity(
    raw: &Value,
    kind: EntityKind,
    symbols: &mut Symbols,
) -> Result<EntityDefinition, String> {
    Ok(EntityDefinition {
        id: symbols.intern(raw["id"].as_str().ok_or("entity identity missing")?),
        name: symbols.intern(raw["name"].as_str().ok_or("entity name missing")?),
        kind,
        roster: None,
        token_type: raw["token_type"].as_str().map(|s| symbols.intern(s)),
        immune_enemy_actions: raw["is_immune_to_enemy_actions"].as_bool().unwrap_or(false),
        owner: raw["owner_hero_id"]
            .as_str()
            .or_else(|| raw["owner_id"].as_str())
            .map(|v| symbols.intern(v)),
        team: raw["team"].as_str().map(|v| symbols.intern(v)),
        lane: raw["lane_id"].as_str().map(|v| symbols.intern(v)),
        has_team: raw.get("team").is_some(),
        passable: raw["is_passable"].as_bool().unwrap_or(false),
        multi_piece: raw["piece_supply"].as_i64().is_some_and(|v| v > 0),
        minion_kind: if kind == EntityKind::Minion {
            Some(serde_json::from_value(raw["type"].clone()).map_err(|e| e.to_string())?)
        } else {
            None
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cases() -> Value {
        serde_json::from_str(include_str!("../../../tests/fixtures/states.json")).unwrap()
    }
    #[test]
    fn physical_card_instances_and_scenario_definitions_compile_without_merging() {
        let cases = cases();
        let source: StateEnvelope =
            serde_json::from_value(cases["cases"][2]["envelope"].clone()).unwrap();
        let mut symbols = Symbols::default();
        let world = NativeWorld::compile(&source, &mut symbols).unwrap();
        let count = source
            .instances
            .iter()
            .filter(|instance| {
                source.definitions[instance.definition as usize].kind != InstanceKind::Token
            })
            .count();
        assert_eq!(world.cards.len(), count);
        let overrides: Vec<_> = world
            .definitions
            .cards
            .iter()
            .filter(|definition| definition.range == Some(17))
            .collect();
        assert_eq!(overrides.len(), 1);
        let printed = overrides[0].printed_id;
        assert!(
            world
                .definitions
                .cards
                .iter()
                .any(|definition| definition.printed_id == printed && definition.range != Some(17))
        );
        let mut seen = std::collections::HashSet::new();
        for id in world.card_by_instance.iter().flatten() {
            assert!(
                seen.insert(*id),
                "distinct physical source instances cannot merge"
            );
        }
        assert_eq!(seen.len(), world.cards.len());
    }
    #[test]
    fn lifecycle_workers_share_definitions_but_keep_card_records_and_collections_private() {
        let corpus: Value = serde_json::from_str(&crate::fixture_cache::text(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/models.json"),
        ))
        .unwrap();
        let source: StateEnvelope =
            serde_json::from_value(corpus["cards"][0]["envelope"].clone()).unwrap();
        let mut symbols = Symbols::default();
        let world = NativeWorld::compile(&source, &mut symbols).unwrap();
        let hero = symbols.lookup("hero_arien").unwrap();
        let card = world.hero(hero).unwrap().hand[0];
        let mut child = world.clone();
        assert!(Arc::ptr_eq(&world.definitions, &child.definitions));
        child.play_card(hero, card).unwrap();
        assert_eq!(world.card(card).unwrap().lifecycle, CardLifecycle::Hand);
        assert_eq!(
            child.card(card).unwrap().lifecycle,
            CardLifecycle::Unresolved
        );
        assert!(world.hero(hero).unwrap().hand.contains(&card));
        assert!(!child.hero(hero).unwrap().hand.contains(&card));
        let mut unchanged = source.clone();
        world.export_cards(&mut unchanged, &symbols);
        assert_eq!(
            unchanged.materialize().unwrap(),
            source.materialize().unwrap()
        );
    }
    #[test]
    fn duplicate_printed_card_model_equality_is_explicitly_unsupported_before_mutation() {
        let cases = cases();
        let source: StateEnvelope =
            serde_json::from_value(cases["cases"][2]["envelope"].clone()).unwrap();
        let mut symbols = Symbols::default();
        let mut world = NativeWorld::compile(&source, &mut symbols).unwrap();
        let hero = symbols.lookup("hero_arien").unwrap();
        let card = world.hero(hero).unwrap().hand[0];
        assert!(matches!(
            world.play_card(hero, card),
            Err(crate::cards::CardError::Unsupported(_))
        ));
        let mut unchanged = source.clone();
        world.export_cards(&mut unchanged, &symbols);
        assert_eq!(
            unchanged.materialize().unwrap(),
            source.materialize().unwrap()
        );
    }
    #[test]
    fn world_boundary_preserves_all_token_spell_and_piece_state() {
        let cases = cases();
        for case in cases["cases"].as_array().unwrap() {
            let source: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
            let mut symbols = Symbols::default();
            let world = NativeWorld::compile(&source, &mut symbols).unwrap();
            let mut compact = source.state.clone();
            world.export_spatial(&mut compact, &symbols);
            let mut exported = source.clone();
            exported.state = compact;
            assert_eq!(exported.materialize().unwrap(), case["expected"]);
        }
    }
}
