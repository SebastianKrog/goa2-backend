//! Lossless offline boundary. JSON lives here, never in the search hot loop.
//! A successfully decoded envelope does not imply native transition coverage.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;

pub const ORACLE_REVISION: &str = "00becf4a68615afa410f8eabe0c841809af0427b";
const REFERENCE_KEY: &str = "$goa2_instance";

pub(crate) fn oracle_manifest() -> Result<&'static Value, String> {
    static MANIFEST: std::sync::OnceLock<Result<Value, String>> = std::sync::OnceLock::new();
    MANIFEST
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../../oracle-manifest.json"))
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceKind {
    Card,
    Spell,
    Token,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub kind: InstanceKind,
    pub fields: Map<String, Value>,
}
/// Python model equality includes this state although normal JSON omits it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceEquality {
    pub model: String,
    pub definition_revision: i64,
    pub json_metadata: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    pub definition: u32,
    pub fields: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equality: Option<InstanceEquality>,
}

/// Paths refer to the compact state tree, retaining physical instance references.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ObjectPathPart {
    Key(String),
    Index(usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    List,
    Dict,
    Model,
    FrozenModel,
    Step,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectBinding {
    pub kind: ObjectKind,
    pub paths: Vec<Vec<ObjectPathPart>>,
    /// The source deepcopy returns this immutable object unchanged (plain Hex).
    #[serde(default, skip_serializing_if = "is_false")]
    pub deepcopy_shared: bool,
}
fn is_false(value: &bool) -> bool {
    !value
}

pub fn object_at_path<'a>(root: &'a Value, path: &[ObjectPathPart]) -> Result<&'a Value, String> {
    let mut value = root;
    for part in path {
        value = match part {
            ObjectPathPart::Key(key) => value.as_object().and_then(|fields| fields.get(key)),
            ObjectPathPart::Index(index) => value.as_array().and_then(|items| items.get(*index)),
        }
        .ok_or("shared object path does not exist or has wrong container type")?;
    }
    Ok(value)
}

/// Immutable board dictionary keys may alias a visible Hex or remain private.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardKey {
    pub hex: crate::geometry::Hex,
    pub path: Option<Vec<ObjectPathPart>>,
}

/// Native transport identity history, separate from rule/search state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResume {
    pub version: u32,
    pub request_high_water: u64,
    pub step_high_water: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateEnvelope {
    pub schema_version: u32,
    pub oracle_revision: String,
    pub source_sha256: String,
    pub rules_version: u32,
    pub definitions: Vec<Definition>,
    pub instances: Vec<Instance>,
    pub state: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub object_bindings: Vec<ObjectBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_keys: Option<Vec<BoardKey>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_resume: Option<NativeResume>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Phase {
    Setup,
    Planning,
    Revelation,
    Resolution,
    Cleanup,
    LevelUp,
    GameOver,
}
#[derive(Debug, Deserialize)]
pub struct StateHeader {
    pub phase: Phase,
    pub round: u32,
    pub turn: u32,
    pub rules_version: u32,
    pub execution_stack: Vec<Value>,
    pub execution_context: Map<String, Value>,
    pub board: Map<String, Value>,
    pub teams: Map<String, Value>,
}
impl StateEnvelope {
    pub fn validate(&self) -> Result<StateHeader, String> {
        let expanded = self.validated_expansion()?;
        let header: StateHeader = serde_json::from_value(expanded).map_err(|e| e.to_string())?;
        self.check_header_rules(header.rules_version)?;
        Ok(header)
    }

    pub(crate) fn expand_for_restore(&self) -> Result<(Phase, Value), String> {
        let expanded = self.validated_expansion()?;
        let phase = Phase::deserialize(&expanded["phase"]).map_err(|e| e.to_string())?;
        for field in ["round", "turn"] {
            u32::deserialize(&expanded[field]).map_err(|e| format!("invalid {field}: {e}"))?;
        }
        self.check_header_rules(
            u32::deserialize(&expanded["rules_version"]).map_err(|e| e.to_string())?,
        )?;
        if !expanded["execution_stack"].is_array() {
            return Err("invalid execution stack".into());
        }
        for field in ["execution_context", "board", "teams"] {
            if !expanded[field].is_object() {
                return Err(format!("invalid {field}"));
            }
        }
        Ok((phase, expanded))
    }

    fn validated_expansion(&self) -> Result<Value, String> {
        if self.schema_version != 1
            || self.oracle_revision != ORACLE_REVISION
            || self.rules_version != 9
        {
            return Err("unsupported state schema, oracle revision or rules version".into());
        }
        let manifest = oracle_manifest()?;
        if manifest["source_sha256"].as_str() != Some(&self.source_sha256) {
            return Err("oracle source content mismatch".into());
        }
        if self.native_resume.is_some_and(|r| r.version != 1) {
            return Err("unsupported native resume version".into());
        }
        for instance in &self.instances {
            let definition = self
                .definitions
                .get(instance.definition as usize)
                .ok_or("definition reference outside table")?;
            if instance
                .fields
                .keys()
                .any(|key| definition.fields.contains_key(key))
            {
                return Err("definition and mutable fields overlap".into());
            }
            if let Some(equality) = &instance.equality
                && (definition.kind == InstanceKind::Token || equality.model.is_empty())
            {
                return Err("invalid physical equality metadata".into());
            }
            // Definitions and instance fields are atomic records. Nested
            // instance references must not create cycles or implicit aliases.
            reject_record(&definition.fields)?;
            reject_record(&instance.fields)?;
        }
        if let Some(keys) = &self.board_keys {
            let tiles = self.state["board"]["tiles"]
                .as_object()
                .ok_or("board-key tile dictionary missing")?;
            if keys.len() != tiles.len() {
                return Err("board-key binding count mismatch".into());
            }
            for (binding, key) in keys.iter().zip(tiles.keys()) {
                let (q, r, s) = binding.hex.coordinates();
                crate::geometry::Hex::new(q, r, s)?;
                if *key != format!("({q}, {r}, {s})") {
                    return Err("board-key coordinates mismatch".into());
                }
                if let Some(path) = &binding.path {
                    let value = object_at_path(&self.state, path)?;
                    if value["q"].as_i64() != Some(i64::from(q))
                        || value["r"].as_i64() != Some(i64::from(r))
                        || value["s"].as_i64() != Some(i64::from(s))
                    {
                        return Err("board-key alias coordinates mismatch".into());
                    }
                }
            }
        }
        self.validate_object_bindings()?;
        self.materialize()
    }

    fn check_header_rules(&self, rules_version: u32) -> Result<(), String> {
        if rules_version != self.rules_version {
            return Err("state/header rules version mismatch".into());
        }
        Ok(())
    }
    /// Visible physical identity topology, independent of table allocation order.
    /// Diagnostic/export only; never used while advancing game rules.
    pub fn instance_bindings(&self) -> Result<Vec<Vec<Vec<ObjectPathPart>>>, String> {
        fn walk(
            value: &Value,
            path: &mut Vec<ObjectPathPart>,
            indices: &mut std::collections::HashMap<u32, usize>,
            result: &mut Vec<Vec<Vec<ObjectPathPart>>>,
        ) -> Result<(), String> {
            match value {
                Value::Object(fields) if fields.contains_key(REFERENCE_KEY) => {
                    let id = fields[REFERENCE_KEY]
                        .as_u64()
                        .and_then(|id| u32::try_from(id).ok())
                        .ok_or("invalid physical reference")?;
                    let next = result.len();
                    let index = *indices.entry(id).or_insert_with(|| {
                        result.push(Vec::new());
                        next
                    });
                    result[index].push(path.clone());
                }
                Value::Object(fields) => {
                    for (key, child) in fields {
                        path.push(ObjectPathPart::Key(key.clone()));
                        walk(child, path, indices, result)?;
                        path.pop();
                    }
                }
                Value::Array(values) => {
                    for (index, child) in values.iter().enumerate() {
                        path.push(ObjectPathPart::Index(index));
                        walk(child, path, indices, result)?;
                        path.pop();
                    }
                }
                _ => {}
            }
            Ok(())
        }
        let mut result = Vec::new();
        walk(
            &self.state,
            &mut Vec::new(),
            &mut std::collections::HashMap::new(),
            &mut result,
        )?;
        Ok(result)
    }
    pub fn validate_object_bindings(&self) -> Result<(), String> {
        let mut used = HashSet::new();
        for object in &self.object_bindings {
            if object.deepcopy_shared && object.kind != ObjectKind::FrozenModel {
                return Err("shared deepcopy requires an immutable model".into());
            }
            if object.paths.is_empty() {
                return Err("object binding requires at least one path".into());
            }
            let first = object_at_path(&self.state, &object.paths[0])?;
            let valid = match object.kind {
                ObjectKind::List => first.is_array(),
                ObjectKind::Dict | ObjectKind::Model | ObjectKind::FrozenModel => first
                    .as_object()
                    .is_some_and(|f| !f.contains_key(REFERENCE_KEY)),
                ObjectKind::Step => first.as_object().is_some_and(|f| {
                    f.get("type").is_some_and(Value::is_string)
                        && f.get("step_id").is_some_and(Value::is_string)
                }),
            };
            if !valid {
                return Err("shared object kind does not match its value".into());
            }
            let ordered = serde_json::to_string(first).map_err(|e| e.to_string())?;
            for path in &object.paths {
                if !used.insert(path.clone()) {
                    return Err("shared object paths overlap or repeat".into());
                }
                if serde_json::to_string(object_at_path(&self.state, path)?)
                    .map_err(|e| e.to_string())?
                    != ordered
                {
                    return Err("shared object paths contain different ordered values".into());
                }
            }
        }
        Ok(())
    }
    pub fn materialize(&self) -> Result<Value, String> {
        self.expand(&self.state)
    }
    pub(crate) fn expand(&self, value: &Value) -> Result<Value, String> {
        match value {
            Value::Object(fields) if fields.contains_key(REFERENCE_KEY) => {
                if fields.len() != 1 {
                    return Err("malformed instance reference".into());
                }
                let index = fields[REFERENCE_KEY]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or("invalid instance index")?;
                let instance = self
                    .instances
                    .get(index)
                    .ok_or("instance reference outside table")?;
                let definition = self
                    .definitions
                    .get(instance.definition as usize)
                    .ok_or("definition reference outside table")?;
                let mut record = definition.fields.clone();
                for (key, item) in &instance.fields {
                    if record.insert(key.clone(), item.clone()).is_some() {
                        return Err("definition and mutable fields overlap".into());
                    }
                }
                Ok(Value::Object(record))
            }
            Value::Object(fields) => fields
                .iter()
                .map(|(key, item)| Ok((key.clone(), self.expand(item)?)))
                .collect::<Result<Map<_, _>, String>>()
                .map(Value::Object),
            Value::Array(items) => items
                .iter()
                .map(|item| self.expand(item))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            _ => Ok(value.clone()),
        }
    }
    pub fn require_native_execution(&self) -> Result<(), String> {
        let header = self.validate()?;
        Err(format!(
            "unsupported native transition: {:?}, {} continuation steps; codec import alone is not rule coverage",
            header.phase,
            header.execution_stack.len()
        ))
    }
}
fn reject_record(fields: &Map<String, Value>) -> Result<(), String> {
    if fields.contains_key(REFERENCE_KEY) {
        return Err("reserved reference inside instance record".into());
    }
    for item in fields.values() {
        reject_reference(item)?;
    }
    Ok(())
}
fn reject_reference(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(fields) => reject_record(fields)?,
        Value::Array(items) => {
            for item in items {
                reject_reference(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn envelope() -> StateEnvelope {
        let manifest = oracle_manifest().unwrap();
        serde_json::from_value(json!({
            "schema_version":1,"oracle_revision":ORACLE_REVISION,"source_sha256":manifest["source_sha256"],"rules_version":9,
            "definitions":[{"kind":"card","fields":{"id":"same_printed_id","range_value":1}}],
            "instances":[{"definition":0,"fields":{"state":"HAND"}},{"definition":0,"fields":{"state":"DECK"}}],
            "state":{"phase":"PLANNING","round":1,"turn":1,"rules_version":9,"execution_stack":[],"execution_context":{},"board":{},"teams":{},
                "hand":[{"$goa2_instance":0}],"pending":{"$goa2_instance":0},"independent":{"$goa2_instance":1}}
        })).unwrap()
    }
    #[test]
    fn aliases_and_independent_printed_duplicates_survive() {
        let env = envelope();
        env.validate().unwrap();
        let state = env.materialize().unwrap();
        assert_eq!(state["hand"][0], state["pending"]);
        assert_ne!(state["pending"], state["independent"]);
        let roundtrip: StateEnvelope =
            serde_json::from_value(serde_json::to_value(&env).unwrap()).unwrap();
        assert_eq!(roundtrip.materialize().unwrap(), state);
        assert!(
            env.require_native_execution()
                .unwrap_err()
                .contains("unsupported native transition")
        );
    }
    #[test]
    fn rejects_bad_references_conflicts_content_and_future_rules() {
        let mut env = envelope();
        env.instances[0].definition = 50;
        assert!(env.validate().is_err());
        let mut env = envelope();
        env.state["pending"] = json!({"$goa2_instance":100});
        assert!(env.validate().is_err());
        let mut env = envelope();
        env.instances[0]
            .fields
            .insert("id".into(), json!("override"));
        assert!(env.validate().is_err());
        let mut env = envelope();
        env.source_sha256 = "other content".into();
        assert!(env.validate().is_err());
        let mut env = envelope();
        env.rules_version = 10;
        assert!(env.validate().is_err());
        let mut env = envelope();
        env.state["rules_version"] = json!(0);
        assert!(env.validate().is_err());
    }
    #[test]
    fn native_resume_roundtrips_without_altering_semantic_state() {
        let mut env = envelope();
        let plain = env.materialize().unwrap();
        assert!(
            serde_json::to_value(&env)
                .unwrap()
                .get("native_resume")
                .is_none()
        );
        env.native_resume = Some(NativeResume {
            version: 1,
            request_high_water: 17,
            step_high_water: 99,
        });
        let wire = serde_json::to_value(&env).unwrap();
        let roundtrip: StateEnvelope = serde_json::from_value(wire).unwrap();
        roundtrip.validate().unwrap();
        assert_eq!(roundtrip.native_resume, env.native_resume);
        assert_eq!(roundtrip.materialize().unwrap(), plain);
        let mut future = roundtrip.clone();
        future.native_resume.as_mut().unwrap().version = 2;
        assert!(future.validate().unwrap_err().contains("resume version"));
        let base = serde_json::to_value(roundtrip).unwrap();
        for invalid in [
            json!({}),
            json!({"version":1,"request_high_water":-1,"step_high_water":0}),
            json!({"version":1,"request_high_water":0.5,"step_high_water":0}),
            json!({"version":1,"request_high_water":0,"step_high_water":0,"extra":true}),
        ] {
            let mut wire = base.clone();
            wire["native_resume"] = invalid;
            assert!(serde_json::from_value::<StateEnvelope>(wire).is_err());
        }
    }
    #[test]
    fn mid_action_state_is_lossless_but_explicitly_not_executable() {
        let mut env = envelope();
        env.state["phase"] = json!("RESOLUTION");
        env.state["execution_stack"] = json!([{"type":"SELECT","id":"meaningful_identity","filters":[{"type":"RANGE","max_range":2}]}]);
        env.state["execution_context"] = json!({"target_id":"hero_hanu","pending_input":null});
        assert_eq!(
            env.materialize().unwrap()["execution_stack"],
            env.state["execution_stack"]
        );
        assert!(
            env.require_native_execution()
                .unwrap_err()
                .contains("1 continuation steps")
        );
    }
}
