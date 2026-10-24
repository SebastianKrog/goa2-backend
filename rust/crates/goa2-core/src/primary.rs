//! Reviewed primary-effect recipes, compiled at import and bound to native operands.
//! The reserved values occur only in frozen recipes, never in an exported game.
use crate::{
    data::{Symbol, Symbols},
    spatial::{HexFilter, Origin},
    stats::CardStats,
    targeting::UnitFilter,
};
use serde_json::Value;
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
}
impl PrimaryOperands {
    pub fn compile(s: &mut Symbols) -> Self {
        Self {
            hero: s.intern(HERO),
            card: s.intern(CARD),
        }
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
    pub fn number(value: i64, v: &PrimaryValues) -> Result<i64, String> {
        match value {
            PRIMARY => Ok(v.stats.primary_value),
            RANGE => Ok(v.stats.range),
            RADIUS => v
                .stats
                .radius
                .ok_or("required primary radius is absent".into()),
            _ => Ok(value),
        }
    }
    pub fn optional_number(value: &mut Option<i64>, v: &PrimaryValues) -> Result<(), String> {
        *value = match *value {
            Some(RADIUS) => v.stats.radius,
            Some(n) => Some(Self::number(n, v)?),
            None => None,
        };
        Ok(())
    }
    fn origin(&self, o: &mut Origin, v: &PrimaryValues) {
        self.optional_symbol(&mut o.literal, v);
    }
    pub fn hex_filter(&self, f: &mut HexFilter, v: &PrimaryValues) -> Result<(), String> {
        match f {
            HexFilter::Range {
                origin, min, max, ..
            } => {
                self.origin(origin, v);
                Self::optional_number(min, v)?;
                Self::optional_number(max, v)?;
            }
            HexFilter::Movement { origin, range, .. } => {
                self.origin(origin, v);
                *range = i32::try_from(Self::number(i64::from(*range), v)?)
                    .map_err(|_| "primary movement range outside i32")?;
            }
            HexFilter::Obstacle { exclude, .. } => self.origin(exclude, v),
            HexFilter::Straight { origin, .. }
            | HexFilter::ClearLine { origin, .. }
            | HexFilter::FastTravel { origin, .. } => self.origin(origin, v),
            HexFilter::Spawn { .. }
            | HexFilter::AdjacentSpawn { .. }
            | HexFilter::AdjacentTags(_) => {}
        }
        Ok(())
    }
    pub fn unit_filter(&self, f: &mut UnitFilter, v: &PrimaryValues) -> Result<(), String> {
        match f {
            UnitFilter::Range(f) | UnitFilter::Spatial(f) => self.hex_filter(f, v)?,
            UnitFilter::BehindEmpty(origin) => self.origin(origin, v),
            UnitFilter::Or(filters) => {
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
    pub fn patch_wire(&self, wire: &mut Value, s: &Symbols) {
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
                _ => {}
            },
            Value::Array(items) => {
                for item in items {
                    self.patch_wire(item, s);
                }
            }
            Value::Object(fields) => {
                for (_, value) in fields {
                    self.patch_wire(value, s);
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
    let manifest: Value = serde_json::from_str(include_str!("../../../oracle-manifest.json"))
        .map_err(|e| e.to_string())?;
    if source["schema_version"] != 1
        || source["source_sha256"] != manifest["source_sha256"]
        || source["operands"]
            != serde_json::json!({"hero":HERO,"card":CARD,"primary":PRIMARY,"range":RANGE,"radius":RADIUS})
    {
        return Err("primary recipe source/operand identity mismatch".into());
    }
    Ok(source)
}
