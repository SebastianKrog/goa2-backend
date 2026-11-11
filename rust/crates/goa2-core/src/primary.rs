//! Reviewed primary-effect recipes, compiled at import and bound to native operands.
//! Catalog bindings are resolved before a primary value reaches the game state.
pub(crate) use crate::primary_operand::Operand;
use crate::{
    data::{Symbol, Symbols},
    spatial::{HexFilter, Origin},
    stats::CardStats,
    targeting::UnitFilter,
};
use serde_json::Value;
pub const HERO: &str = "native-primary-hero";
pub const CARD: &str = "native-primary-card";
pub const RANGE: i64 = 1234567892;
pub const DEFENSE_LOG: &str = "native-primary-defense-log";
pub const SKILL_LOG: &str = "native-primary-skill-log";
#[derive(Clone, Debug, Hash)]
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
    numeric: &'static [Operand],
}

const FIRST_BINDING: i64 = i32::MIN as i64;

fn binding_index(value: i64) -> Option<usize> {
    usize::try_from(value.checked_sub(FIRST_BINDING)?).ok()
}

impl PrimaryOperands {
    pub fn compile(s: &mut Symbols) -> Result<Self, String> {
        Ok(Self {
            hero: s.intern(HERO),
            card: s.intern(CARD),
            numeric: &compiled_catalog()?.numeric,
        })
    }

    fn operand(&self, value: i64) -> Option<Operand> {
        self.numeric.get(binding_index(value)?).copied()
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
        self.operand(value)
            .map_or(Ok(value), |operand| operand.required(&v.stats))
    }

    pub fn optional_number(
        &self,
        value: &mut Option<i64>,
        v: &PrimaryValues,
    ) -> Result<(), String> {
        if let Some(operand) = value.and_then(|n| self.operand(n)) {
            *value = operand.resolve(&v.stats)?;
        }
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
            | HexFilter::MaxEmptySpacesInLine { origin, .. }
            | HexFilter::ClearLine { origin, .. }
            | HexFilter::SameDirection { origin, .. }
            | HexFilter::LineOfSight { origin, .. } => self.origin(origin, v),
            HexFilter::FastTravel {
                origin,
                source_card,
                ..
            } => {
                self.origin(origin, v);
                self.optional_symbol(source_card, v);
            }
            HexFilter::CoMover { .. }
            | HexFilter::AdjacentContext(_)
            | HexFilter::Between { .. }
            | HexFilter::SpawnTeam(_)
            | HexFilter::BattleZone
            | HexFilter::Exclude(_)
            | HexFilter::Spawn { .. }
            | HexFilter::AdjacentSpawn { .. }
            | HexFilter::AdjacentTags(_)
            | HexFilter::Reject
            | HexFilter::Terrain(_)
            | HexFilter::AdjacentTerrain(_) => {}
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
    pub fn patch_wire(
        &self,
        wire: &mut Value,
        s: &Symbols,
        slots: &PrimaryOperands,
    ) -> Result<(), String> {
        match wire {
            Value::String(text) if text == HERO => *wire = Value::from(s.name(self.hero)),
            Value::String(text) if text == CARD => *wire = Value::from(s.name(self.card)),
            Value::String(text) if text == DEFENSE_LOG => {
                *wire = Value::from(format!("{} Defends (Primary).", s.name(self.hero)))
            }
            Value::String(text) if text == SKILL_LOG => {
                *wire = Value::from(format!("Skill '{}' did nothing.", s.name(self.name)))
            }
            Value::Number(n) => {
                if let Some(operand) = n.as_i64().and_then(|value| slots.operand(value)) {
                    *wire = operand
                        .resolve(&self.stats)?
                        .map_or(Value::Null, Value::from);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.patch_wire(item, s, slots)?;
                }
            }
            Value::Object(fields) => {
                for (_, value) in fields {
                    self.patch_wire(value, s, slots)?;
                }
            }
            _ => {}
        }
        Ok(())
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
    let source: Value = serde_json::from_str(include_str!(
        "../../../content/catalogs/primary-effects.json"
    ))
    .map_err(|e| e.to_string())?;
    let manifest = crate::codec::oracle_manifest()?;
    if source["schema_version"] != 3
        || source["source_sha256"] != manifest["source_sha256"]
        || source["operands"]
            != serde_json::json!({"hero":HERO,"card":CARD,"primary":{"$operand":{"kind":"primary"}},"range":{"$operand":{"kind":"range"}},"radius":{"$operand":{"kind":"radius"}}})
    {
        return Err("primary recipe source/operand identity mismatch".into());
    }
    Ok(source)
}

pub(crate) struct CompiledCatalog {
    pub recipes: Value,
    pub primary: i64,
    pub range: i64,
    numeric: Vec<Operand>,
}

#[cfg(test)]
impl CompiledCatalog {
    pub(crate) fn binding(&self, operand: Operand) -> i64 {
        FIRST_BINDING
            + self
                .numeric
                .iter()
                .position(|value| *value == operand)
                .unwrap() as i64
    }
}

pub(crate) fn compiled_catalog() -> Result<&'static CompiledCatalog, String> {
    static CATALOG: std::sync::OnceLock<Result<CompiledCatalog, String>> =
        std::sync::OnceLock::new();
    CATALOG
        .get_or_init(compile_catalog)
        .as_ref()
        .map_err(Clone::clone)
}

fn compile_catalog() -> Result<CompiledCatalog, String> {
    fn lower(
        value: &mut Value,
        numeric: &mut Vec<Operand>,
        literals: &mut Vec<i64>,
    ) -> Result<(), String> {
        if let Some(operand) = Operand::parse(value)? {
            let index = match numeric.iter().position(|item| *item == operand) {
                Some(index) => index,
                None => {
                    numeric.push(operand);
                    numeric.len() - 1
                }
            };
            *value = Value::from(FIRST_BINDING + index as i64);
            return Ok(());
        }
        match value {
            Value::Object(fields) => {
                for field in fields.values_mut() {
                    lower(field, numeric, literals)?;
                }
            }
            Value::Array(fields) => {
                for field in fields {
                    lower(field, numeric, literals)?;
                }
            }
            Value::Number(value) => literals.extend(value.as_i64()),
            _ => {}
        }
        Ok(())
    }

    // Instruction fields stay compact. These private indices refer to the
    // immutable operand table and never appear in generated or saved data.
    let mut numeric = vec![Operand::Primary, Operand::Range];
    let mut literals = Vec::new();
    let mut recipes = catalog()?["recipes"].clone();
    lower(&mut recipes, &mut numeric, &mut literals)?;
    if literals
        .into_iter()
        .any(|value| binding_index(value).is_some_and(|index| index < numeric.len()))
    {
        return Err("primary operand binding collides with a recipe literal".into());
    }
    Ok(CompiledCatalog {
        recipes,
        primary: FIRST_BINDING,
        range: FIRST_BINDING + 1,
        numeric,
    })
}
