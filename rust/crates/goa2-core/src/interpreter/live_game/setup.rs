use super::{ExecutionBudget, GameTransition, LiveGame, Result, invalid, seed::SetupRng};
use crate::{
    codec::{
        Definition, Instance, ORACLE_REVISION, ObjectBinding, ObjectKind, ObjectPathPart,
        StateEnvelope, oracle_manifest,
    },
    geometry::Hex,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
};

#[derive(Clone, Copy, Default, Debug, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum GameMode {
    Quick,
    #[default]
    Long,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SetupTeam {
    Red,
    Blue,
}

impl SetupTeam {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Red => "RED",
            Self::Blue => "BLUE",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GameSetup {
    pub map: String,
    pub red_heroes: Vec<String>,
    pub blue_heroes: Vec<String>,
    pub seed: u64,
    #[serde(default)]
    pub mode: GameMode,
    #[serde(default)]
    pub cheats_enabled: bool,
    pub tie_breaker: Option<SetupTeam>,
}

/// Versioned static definitions generated from the pinned rules content.
/// Load once at service startup and reuse for every game.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameContent {
    version: u32,
    oracle_revision: String,
    source_sha256: String,
    maps: HashMap<String, StateEnvelope>,
    heroes: HashMap<String, HeroTemplate>,
    tokens: Vec<TokenTemplate>,
    minions: HashMap<String, HashMap<String, Value>>,
}

#[derive(Deserialize)]
struct HeroTemplate {
    model: Value,
    pieces: Map<String, Value>,
    bindings: Vec<ObjectBinding>,
    definitions: Vec<Definition>,
    instances: Vec<Instance>,
    token_types: Vec<String>,
}

#[derive(Deserialize)]
struct TokenTemplate {
    kind: String,
    count: usize,
    definition: Definition,
    instance: Instance,
}

#[derive(Deserialize)]
struct Spawn {
    location: Hex,
    team: SetupTeam,
    #[serde(rename = "type")]
    kind: SpawnKind,
    minion_type: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
enum SpawnKind {
    Hero,
    Minion,
}

impl GameContent {
    pub fn read(reader: impl Read) -> Result<Self> {
        let content: Self = serde_json::from_reader(reader)
            .map_err(|error| invalid(&format!("invalid setup content: {error}")))?;
        let manifest = oracle_manifest().map_err(super::super::unsupported)?;
        if content.version != crate::versions::CONTENT
            || content.oracle_revision != ORACLE_REVISION
            || manifest["source_sha256"].as_str() != Some(&content.source_sha256)
        {
            return Err(invalid("setup content is incompatible with this engine"));
        }
        Ok(content)
    }

    pub fn identity(&self) -> Value {
        json!({
            "version": self.version,
            "oracle_revision": self.oracle_revision,
            "source_sha256": self.source_sha256,
            "map_count": self.maps.len(),
            "hero_count": self.heroes.len(),
        })
    }

    pub fn hero_names(&self) -> Vec<&str> {
        let mut names = self.heroes.keys().map(String::as_str).collect::<Vec<_>>();
        names.sort_unstable();
        names
    }

    pub fn has_hero(&self, name: &str) -> bool {
        self.heroes.contains_key(name)
    }

    pub fn map_names(&self) -> Vec<&str> {
        let mut names = self.maps.keys().map(String::as_str).collect::<Vec<_>>();
        names.sort_unstable();
        names
    }

    fn assemble(&self, setup: &GameSetup) -> Result<StateEnvelope> {
        let mut envelope = self
            .maps
            .get(&setup.map)
            .ok_or_else(|| invalid("unknown game map"))?
            .clone();
        let roster = [
            (SetupTeam::Red, &setup.red_heroes),
            (SetupTeam::Blue, &setup.blue_heroes),
        ];
        let players = setup.red_heroes.len() + setup.blue_heroes.len();
        if roster
            .iter()
            .any(|(_, team)| team.is_empty() || team.len() > 5)
        {
            return Err(invalid("each team needs one to five heroes"));
        }
        let board = &envelope.state["board"];
        let spawns: Vec<Spawn> = serde_json::from_value(board["spawn_points"].clone())
            .map_err(|_| invalid("invalid map spawn definitions"))?;
        let lanes = board["lanes"]
            .as_object()
            .ok_or_else(|| invalid("map lanes missing"))?;
        let (waves, lives) = counters(setup.mode, players, lanes.len())?;
        let mut battle_zones = Map::new();
        let mut wave_counters = Map::new();
        let mut minion_spawns = Vec::new();
        for (lane, zones) in lanes {
            let zones = zones
                .as_array()
                .filter(|zones| !zones.is_empty())
                .ok_or_else(|| invalid("map lane has no zones"))?;
            let starting = &board["starting_battle_zones"][lane];
            let zone = if zones.contains(starting) {
                starting
            } else {
                &zones[zones.len() / 2]
            };
            let zone = zone
                .as_str()
                .ok_or_else(|| invalid("invalid battle zone"))?;
            battle_zones.insert(lane.clone(), json!(zone));
            wave_counters.insert(lane.clone(), json!(waves));
            let hexes = board["zones"][zone]["hexes"]
                .as_array()
                .ok_or_else(|| invalid("battle zone has no hexes"))?;
            for hex in hexes {
                let hex: Hex = serde_json::from_value(hex.clone())
                    .map_err(|_| invalid("invalid battle zone hex"))?;
                if let Some(spawn) = spawns.iter().find(|spawn| spawn.location == hex)
                    && spawn.kind == SpawnKind::Minion
                    && spawn.minion_type.is_some()
                {
                    minion_spawns.push((lane.clone(), spawn));
                }
            }
        }

        let mut roster_names = HashSet::new();
        let mut required_tokens = HashSet::new();
        for (team, names) in roster {
            let team_spawns = spawns
                .iter()
                .enumerate()
                .filter(|(_, spawn)| spawn.kind == SpawnKind::Hero && spawn.team == team)
                .collect::<Vec<_>>();
            envelope.state["teams"][team.name()]["life_counters"] = json!(lives);
            envelope.state["teams"][team.name()]["starting_life_counters"] = json!(lives);
            for (index, name) in names.iter().enumerate() {
                if !roster_names.insert(name) {
                    return Err(invalid("a hero can only appear once in a game"));
                }
                let template = self
                    .heroes
                    .get(name)
                    .ok_or_else(|| invalid("unknown roster hero"))?;
                required_tokens.extend(template.token_types.iter().map(String::as_str));
                let entity = insert_hero(&mut envelope, template, team, index)?;
                let (hex, source) = hero_spawn(&envelope, &team_spawns)?;
                place(&mut envelope, &entity, hex, source)?;
            }
        }
        let mut next_id = 1;
        install_tokens(
            &mut envelope,
            &self.tokens,
            &required_tokens,
            setup.seed,
            &mut next_id,
        )?;
        for (lane, spawn) in minion_spawns {
            let team = spawn.team.name();
            let kind = spawn.minion_type.as_deref().unwrap();
            let id = format!("minion_{next_id}");
            next_id += 1;
            let mut minion = self
                .minions
                .get(team)
                .and_then(|rows| rows.get(kind))
                .ok_or_else(|| invalid("unknown minion type"))?
                .clone();
            minion["id"] = json!(id);
            minion["lane_id"] = json!(lane);
            let minions = envelope.state["teams"][team]["minions"]
                .as_array_mut()
                .ok_or_else(|| invalid("invalid team minion list"))?;
            let index = minions.len();
            minions.push(minion);
            let model_path = vec![
                key("teams"),
                key(team),
                key("minions"),
                ObjectPathPart::Index(index),
            ];
            let mut markers_path = model_path.clone();
            markers_path.push(key("markers"));
            for (kind, path) in [
                (ObjectKind::Model, model_path),
                (ObjectKind::List, markers_path),
            ] {
                envelope.object_bindings.push(ObjectBinding {
                    kind,
                    paths: vec![path],
                    deepcopy_shared: false,
                });
            }
            let hex_path = vec![
                key("board"),
                key("tiles"),
                key(&tile_key(spawn.location)),
                key("hex"),
            ];
            place(&mut envelope, &id, spawn.location, Some(hex_path))?;
        }

        let tie = setup.tie_breaker.unwrap_or_else(|| {
            [SetupTeam::Red, SetupTeam::Blue][SetupRng::integer(setup.seed).below(2)]
        });
        envelope.state["battle_zones"] = Value::Object(battle_zones);
        envelope.state["wave_counters"] = Value::Object(wave_counters);
        envelope.state["rng_seed"] = json!(setup.seed);
        envelope.state["tie_breaker_team"] = json!(tie);
        envelope.state["game_type"] = json!(setup.mode);
        envelope.state["cheats_enabled"] = json!(setup.cheats_enabled);
        envelope.state["next_entity_id"] = json!(next_id);
        Ok(envelope)
    }
}

impl LiveGame {
    /// Create a new, untimed rules-v9 game using the service's static content.
    pub fn create(
        content: &GameContent,
        setup: &GameSetup,
        budget: &ExecutionBudget<'_>,
    ) -> Result<(Self, GameTransition)> {
        Self::import(content.assemble(setup)?, budget)
    }
}

fn counters(mode: GameMode, players: usize, lanes: usize) -> Result<(usize, usize)> {
    match (lanes, mode) {
        (1, GameMode::Quick) => Ok((
            3,
            match players {
                0..=2 => 3,
                3..=5 => 4,
                _ => 5,
            },
        )),
        (1, GameMode::Long) => Ok((5, if players <= 5 { 6 } else { 8 })),
        (2, _) => Ok((7, if players <= 8 { 6 } else { 7 })),
        _ => Err(invalid("map needs one or two lanes")),
    }
}

fn key(name: &str) -> ObjectPathPart {
    ObjectPathPart::Key(name.to_owned())
}

fn tile_key(hex: Hex) -> String {
    let (q, r, s) = hex.coordinates();
    format!("({q}, {r}, {s})")
}

fn hero_spawn(
    envelope: &StateEnvelope,
    spawns: &[(usize, &Spawn)],
) -> Result<(Hex, Option<Vec<ObjectPathPart>>)> {
    let tiles = &envelope.state["board"]["tiles"];
    for (index, spawn) in spawns {
        if tiles
            .get(tile_key(spawn.location))
            .is_some_and(|tile| tile["occupant_id"].is_null())
        {
            return Ok((
                spawn.location,
                Some(vec![
                    key("board"),
                    key("spawn_points"),
                    ObjectPathPart::Index(*index),
                    key("location"),
                ]),
            ));
        }
    }
    for (_, spawn) in spawns {
        for hex in spawn.location.neighbors() {
            if tiles
                .get(tile_key(hex))
                .is_some_and(|tile| tile["is_terrain"] == false && tile["occupant_id"].is_null())
            {
                return Ok((hex, None));
            }
        }
    }
    Err(invalid("the map has no free hero spawn for this team"))
}

fn place(
    envelope: &mut StateEnvelope,
    id: &str,
    hex: Hex,
    source: Option<Vec<ObjectPathPart>>,
) -> Result<()> {
    let tile = envelope.state["board"]["tiles"]
        .get_mut(tile_key(hex))
        .filter(|tile| tile["occupant_id"].is_null())
        .ok_or_else(|| invalid("initial entity position is unavailable"))?;
    tile["occupant_id"] = json!(id);
    envelope.state["entity_locations"][id] = json!(hex);
    envelope.state["last_turn_positions"][id] = json!(hex);
    let paths = [
        vec![key("last_turn_positions"), key(id)],
        vec![key("entity_locations"), key(id)],
    ];
    if let Some(source) = source {
        let binding = envelope
            .object_bindings
            .iter_mut()
            .find(|binding| binding.paths.contains(&source))
            .ok_or_else(|| invalid("map is missing a spawn coordinate binding"))?;
        binding.paths.extend(paths);
    } else {
        envelope.object_bindings.push(ObjectBinding {
            kind: ObjectKind::FrozenModel,
            paths: paths.into(),
            deepcopy_shared: true,
        });
    }
    Ok(())
}

fn insert_hero(
    envelope: &mut StateEnvelope,
    template: &HeroTemplate,
    team: SetupTeam,
    index: usize,
) -> Result<String> {
    let mut model = template.model.clone();
    let id = model["id"]
        .as_str()
        .ok_or_else(|| invalid("hero template has no ID"))?
        .to_owned();
    let instance_offset = envelope.instances.len() as u64;
    offset_instances(&mut model, instance_offset);
    let definition_offset = envelope.definitions.len() as u32;
    envelope
        .definitions
        .extend(template.definitions.iter().cloned());
    envelope
        .instances
        .extend(template.instances.iter().cloned().map(|mut instance| {
            instance.definition += definition_offset;
            instance
        }));
    model["team"] = json!(team);
    envelope.state["teams"][team.name()]["heroes"]
        .as_array_mut()
        .ok_or_else(|| invalid("invalid team hero list"))?
        .push(model);
    for (id, model) in &template.pieces {
        let mut piece = model.clone();
        piece["team"] = json!(team);
        envelope.state["misc_entities"][id] = piece;
    }
    for binding in &template.bindings {
        let mut binding = binding.clone();
        for path in &mut binding.paths {
            if let [ObjectPathPart::Key(root), side, _, slot, ..] = path.as_mut_slice()
                && root == "teams"
            {
                *side = key(team.name());
                *slot = ObjectPathPart::Index(index);
            }
        }
        envelope.object_bindings.push(binding);
    }
    if template.pieces.is_empty() {
        Ok(id)
    } else {
        Ok(format!("{id}_piece_1"))
    }
}

fn offset_instances(value: &mut Value, offset: u64) {
    match value {
        Value::Object(fields) => {
            if let Some(index) = fields.get_mut("$goa2_instance") {
                if let Some(id) = index.as_u64() {
                    *index = json!(offset + id);
                }
            } else {
                for child in fields.values_mut() {
                    offset_instances(child, offset);
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                offset_instances(child, offset);
            }
        }
        _ => {}
    }
}

fn install_tokens(
    envelope: &mut StateEnvelope,
    templates: &[TokenTemplate],
    required: &HashSet<&str>,
    seed: u64,
    next_id: &mut usize,
) -> Result<()> {
    const BLAST: &str = "mine_blast";
    const DUD: &str = "mine_dud";
    let mut pool = Map::new();
    for template in templates {
        if required.contains(template.kind.as_str()) {
            pool.insert(template.kind.clone(), Value::Array(Vec::new()));
            envelope.object_bindings.push(ObjectBinding {
                kind: ObjectKind::List,
                paths: vec![vec![key("token_pool"), key(&template.kind)]],
                deepcopy_shared: false,
            });
        }
    }
    if required.iter().any(|kind| !pool.contains_key(*kind)) {
        return Err(invalid("roster needs a missing token supply"));
    }
    envelope.state["token_pool"] = Value::Object(pool);
    for template in templates {
        let kind = template.kind.as_str();
        if !required.contains(kind) {
            *next_id += template.count;
        } else if kind == BLAST {
            let dud = templates
                .iter()
                .find(|row| row.kind == DUD)
                .filter(|_| required.contains(DUD))
                .ok_or_else(|| invalid("mine supply needs both subtypes"))?;
            let mut mines = vec![template; template.count];
            mines.extend(std::iter::repeat_n(dud, dud.count));
            let mut rng = SetupRng::text(&format!("goa2:mine-supply:{seed}"));
            for i in (1..mines.len()).rev() {
                mines.swap(i, rng.below(i as u32 + 1));
            }
            for mine in mines {
                install_token(envelope, mine, "mine", next_id)?;
            }
        } else if kind != DUD {
            for _ in 0..template.count {
                install_token(envelope, template, kind, next_id)?;
            }
        }
    }
    Ok(())
}

fn install_token(
    envelope: &mut StateEnvelope,
    template: &TokenTemplate,
    prefix: &str,
    next_id: &mut usize,
) -> Result<()> {
    let id = format!("{prefix}_{next_id}");
    *next_id += 1;
    let mut definition = template.definition.clone();
    definition.fields.insert("id".into(), json!(id));
    let mut instance = template.instance.clone();
    instance.definition = envelope.definitions.len() as u32;
    let reference = json!({"$goa2_instance": envelope.instances.len()});
    envelope.definitions.push(definition);
    envelope.instances.push(instance);
    envelope.state["misc_entities"][&id] = reference.clone();
    envelope.state["token_pool"][&template.kind]
        .as_array_mut()
        .ok_or_else(|| invalid("invalid token pool"))?
        .push(reference);
    Ok(())
}
