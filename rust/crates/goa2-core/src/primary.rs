//! Reviewed primary-effect recipes, compiled at import and bound to native operands.
//! The reserved values occur only in frozen recipes, never in an exported game.
use crate::{
    data::{Symbol, Symbols},
    spatial::{HexFilter, Origin},
    stats::CardStats,
    targeting::UnitFilter,
};
use serde_json::Value;
use std::sync::Arc;
pub const HERO: &str = "native-primary-hero";
pub const CARD: &str = "native-primary-card";
pub const PRIMARY: i64 = 1234567891;
pub const RANGE: i64 = 1234567892;
pub const RADIUS: i64 = 1234567893;
pub const DEFENSE_LOG: &str = "native-primary-defense-log";
pub const SKILL_LOG: &str = "native-primary-skill-log";
#[derive(Clone, Debug)]
pub struct PrimaryValues {
    pub hero: Symbol,
    pub card: Symbol,
    pub name: Symbol,
    pub stats: CardStats,
    pub ranged: bool,
}
#[derive(Clone, Debug)]
pub struct PrimaryOperands {
    hero: Symbol,
    card: Symbol,
    radius_defaults: Arc<[(i64, i64)]>,
}
impl PrimaryOperands {
    pub fn compile(s: &mut Symbols) -> Result<Self, String> {
        let radius_defaults = catalog()?["radius_fallbacks"]
            .as_object()
            .ok_or("primary radius fallbacks missing")?
            .iter()
            .map(|(slot, value)| {
                Ok((
                    slot.parse::<i64>().map_err(|e| e.to_string())?,
                    value.as_i64().ok_or("primary radius fallback invalid")?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            hero: s.intern(HERO),
            card: s.intern(CARD),
            radius_defaults: Arc::from(radius_defaults),
        })
    }
    pub fn radius_default(&self, slot: i64, radius: Option<i64>) -> Option<i64> {
        self.radius_defaults
            .iter()
            .find(|(key, _)| *key == slot)
            .map(|(_, default)| radius.filter(|v| *v != 0).unwrap_or(*default))
    }
    pub fn symbol(&self, value: Symbol, v: &PrimaryValues) -> Symbol {
        if value == self.hero {
            v.hero
        } else if value == self.card {
            v.card
        } else {
            value
        }
    }
    pub fn optional_symbol(&self, value: &mut Option<Symbol>, v: &PrimaryValues) {
        if let Some(id) = value {
            *id = self.symbol(*id, v);
        }
    }
    pub fn number(&self, value: i64, v: &PrimaryValues) -> Result<i64, String> {
        match value {
            PRIMARY => Ok(v.stats.primary_value),
            RANGE => Ok(v.stats.range),
            RADIUS => v
                .stats
                .radius
                .ok_or("required primary radius is absent".into()),
            _ => Ok(self.radius_default(value, v.stats.radius).unwrap_or(value)),
        }
    }
    pub fn optional_number(
        &self,
        value: &mut Option<i64>,
        v: &PrimaryValues,
    ) -> Result<(), String> {
        *value = match *value {
            Some(RADIUS) => v.stats.radius,
            Some(n) => Some(self.number(n, v)?),
            None => None,
        };
        Ok(())
    }
    fn origin(&self, o: &mut Origin, v: &PrimaryValues) {
        self.optional_symbol(&mut o.literal, v);
    }
    fn count_match(
        &self,
        count: &mut crate::targeting::CandidateCount,
        v: &PrimaryValues,
    ) -> Result<(), String> {
        count.min = self.number(count.min, v)?;
        self.optional_number(&mut count.max, v)?;
        for filter in &mut count.filters {
            self.unit_filter(filter, v)?;
        }
        Ok(())
    }
    pub fn hex_filter(&self, f: &mut HexFilter, v: &PrimaryValues) -> Result<(), String> {
        match f {
            HexFilter::CountMatch(count) => self.count_match(count, v)?,
            HexFilter::And(filters) | HexFilter::Or(filters) => {
                for filter in filters {
                    self.hex_filter(filter, v)?;
                }
            }
            HexFilter::Batch(search) => {
                for filters in &mut search.filters {
                    for filter in filters {
                        self.hex_filter(filter, v)?;
                    }
                }
            }
            HexFilter::Farthest(search) => self.origin(&mut search.origin, v),
            HexFilter::Range {
                origin, min, max, ..
            } => {
                self.origin(origin, v);
                self.optional_number(min, v)?;
                self.optional_number(max, v)?;
            }
            HexFilter::Movement { origin, range, .. } => {
                self.origin(origin, v);
                *range = i32::try_from(self.number(i64::from(*range), v)?)
                    .map_err(|_| "primary movement range outside i32")?;
            }
            HexFilter::Obstacle { exclude, .. } => self.origin(exclude, v),
            HexFilter::Relative { origin, .. }
            | HexFilter::BehindTarget { origin, .. }
            | HexFilter::Straight { origin, .. }
            | HexFilter::ClearLine { origin, .. } => self.origin(origin, v),
            HexFilter::FastTravel {
                origin,
                source_card,
                ..
            } => {
                self.origin(origin, v);
                self.optional_symbol(source_card, v);
            }
            HexFilter::AdjacentContext(_)
            | HexFilter::Between { .. }
            | HexFilter::SpawnTeam(_)
            | HexFilter::BattleZone
            | HexFilter::Exclude(_)
            | HexFilter::Spawn { .. }
            | HexFilter::AdjacentSpawn { .. }
            | HexFilter::AdjacentTags(_)
            | HexFilter::Reject
            | HexFilter::Terrain(_) => {}
        }
        Ok(())
    }
    pub fn unit_filter(&self, f: &mut UnitFilter, v: &PrimaryValues) -> Result<(), String> {
        match f {
            UnitFilter::CountMatch(count) => self.count_match(count, v)?,
            UnitFilter::Cards { min, max, .. } => {
                self.optional_number(min, v)?;
                self.optional_number(max, v)?;
            }
            UnitFilter::BatchRemoval(search) => {
                for filters in &mut search.batch.filters {
                    for filter in filters {
                        self.hex_filter(filter, v)?;
                    }
                }
            }
            UnitFilter::Range(f) | UnitFilter::Spatial(f) => self.hex_filter(f, v)?,
            UnitFilter::BehindEmpty(origin) => self.origin(origin, v),
            UnitFilter::Or(filters) | UnitFilter::And(filters) => {
                for filter in filters {
                    self.unit_filter(filter, v)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
impl PrimaryValues {
    /// Boundary-only substitution. Step execution binds typed operands instead.
    pub fn patch_wire(&self, wire: &mut Value, s: &Symbols, slots: &PrimaryOperands) {
        match wire {
            Value::String(text) if text == HERO => *wire = Value::from(s.name(self.hero)),
            Value::String(text) if text == CARD => *wire = Value::from(s.name(self.card)),
            Value::String(text) if text == DEFENSE_LOG => {
                *wire = Value::from(format!("{} Defends (Primary).", s.name(self.hero)))
            }
            Value::String(text) if text == SKILL_LOG => {
                *wire = Value::from(format!("Skill '{}' did nothing.", s.name(self.name)))
            }
            Value::Number(n) => match n.as_i64() {
                Some(PRIMARY) => *wire = Value::from(self.stats.primary_value),
                Some(RANGE) => *wire = Value::from(self.stats.range),
                Some(RADIUS) => *wire = self.stats.radius.map_or(Value::Null, Value::from),
                Some(n) => {
                    if let Some(value) = slots.radius_default(n, self.stats.radius) {
                        *wire = Value::from(value);
                    }
                }
                None => {}
            },
            Value::Array(items) => {
                for item in items {
                    self.patch_wire(item, s, slots);
                }
            }
            Value::Object(fields) => {
                for (_, value) in fields {
                    self.patch_wire(value, s, slots);
                }
            }
            _ => {}
        }
    }
}
pub fn catalog() -> Result<&'static Value, String> {
    static CATALOG: std::sync::OnceLock<Result<Value, String>> = std::sync::OnceLock::new();
    CATALOG
        .get_or_init(load_catalog)
        .as_ref()
        .map_err(Clone::clone)
}
fn load_catalog() -> Result<Value, String> {
    let source: Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/primary-effects.json"))
            .map_err(|e| e.to_string())?;
    let manifest = crate::codec::oracle_manifest()?;
    if source["schema_version"] != 2
        || source["source_sha256"] != manifest["source_sha256"]
        || source["operands"]
            != serde_json::json!({"hero":HERO,"card":CARD,"primary":PRIMARY,"range":RANGE,"radius":RADIUS})
    {
        return Err("primary recipe source/operand identity mismatch".into());
    }
    Ok(source)
}
