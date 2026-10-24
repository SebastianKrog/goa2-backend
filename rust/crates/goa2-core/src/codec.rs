//! Lossless offline boundary. JSON lives here, never in the search hot loop.
//! A successfully decoded envelope does not imply native transition coverage.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const ORACLE_REVISION: &str = "00becf4a68615afa410f8eabe0c841809af0427b";
const REFERENCE_KEY: &str = "$goa2_instance";

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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    pub definition: u32,
    pub fields: Map<String, Value>,
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
        if self.schema_version != 1
            || self.oracle_revision != ORACLE_REVISION
            || self.rules_version != 9
        {
            return Err("unsupported state schema, oracle revision or rules version".into());
        }
        let manifest: Value = serde_json::from_str(include_str!("../../../oracle-manifest.json"))
            .map_err(|e| e.to_string())?;
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
            // Definitions and instance fields are atomic records. Nested
            // instance references must not create cycles or implicit aliases.
            reject_reference(&Value::Object(definition.fields.clone()))?;
            reject_reference(&Value::Object(instance.fields.clone()))?;
        }
        let expanded = self.materialize()?;
        let header: StateHeader = serde_json::from_value(expanded).map_err(|e| e.to_string())?;
        if header.rules_version != self.rules_version {
            return Err("state/header rules version mismatch".into());
        }
        Ok(header)
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
fn reject_reference(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(fields) => {
            if fields.contains_key(REFERENCE_KEY) {
                return Err("reserved reference inside instance record".into());
            }
            for item in fields.values() {
                reject_reference(item)?;
            }
        }
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
        let manifest: Value =
            serde_json::from_str(include_str!("../../../oracle-manifest.json")).unwrap();
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
