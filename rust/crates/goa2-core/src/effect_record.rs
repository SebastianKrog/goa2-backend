//! Shared typed effect record readers for import and live construction.
use crate::{
    data::{Datum, Symbol, Symbols},
    geometry::Hex,
    world::{ActionKind, MinionKind},
};

#[derive(Clone, Copy)]
pub(crate) struct Record<'a> {
    pub fields: &'a [(Symbol, Datum)],
    pub symbols: &'a Symbols,
}
impl<'a> Record<'a> {
    pub fn new(value: &'a Datum, symbols: &'a Symbols) -> Result<Self, String> {
        let Datum::Object(fields) = value else {
            return Err("effect record must have fields".into());
        };
        Ok(Self { fields, symbols })
    }
    pub fn get(self, name: &str) -> &'a Datum {
        self.symbols
            .lookup(name)
            .and_then(|key| self.fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v))
            .unwrap_or(&Datum::Null)
    }
    pub fn text(self, name: &str) -> Option<&'a str> {
        match self.get(name) {
            Datum::Text(id) => Some(self.symbols.name(*id)),
            _ => None,
        }
    }
    pub fn symbol(self, name: &str) -> Result<Option<Symbol>, String> {
        match self.get(name) {
            Datum::Null => Ok(None),
            Datum::Text(id) => Ok(Some(*id)),
            _ => Err(format!("invalid effect {name}")),
        }
    }
    pub fn required_symbol(self, name: &str) -> Result<Symbol, String> {
        self.symbol(name)?
            .ok_or_else(|| format!("effect {name} missing"))
    }
    pub fn integer(self, name: &str) -> Result<i64, String> {
        match self.get(name) {
            Datum::Int(value) => Ok(*value),
            _ => Err(format!("invalid effect {name}")),
        }
    }
    pub fn optional_integer(self, name: &str) -> Result<Option<i64>, String> {
        if matches!(self.get(name), Datum::Null) {
            Ok(None)
        } else {
            self.integer(name).map(Some)
        }
    }
    pub fn flag(self, name: &str) -> Result<bool, String> {
        match self.get(name) {
            Datum::Bool(value) => Ok(*value),
            _ => Err(format!("invalid effect {name}")),
        }
    }
    pub fn list(self, name: &str) -> Result<&'a [Datum], String> {
        match self.get(name) {
            Datum::List(value) => Ok(value),
            _ => Err(format!("invalid effect list {name}")),
        }
    }
    pub fn ids(self, name: &str) -> Result<Vec<Symbol>, String> {
        self.list(name)?
            .iter()
            .map(|value| match value {
                Datum::Text(id) => Ok(*id),
                _ => Err(format!("invalid effect identity in {name}")),
            })
            .collect()
    }
    pub fn record(self, name: &str) -> Result<Self, String> {
        Self::new(self.get(name), self.symbols)
    }
    pub fn hex(self, name: &str) -> Result<Option<Hex>, String> {
        let value = self.get(name);
        if matches!(value, Datum::Null) {
            return Ok(None);
        }
        let keys = ["q", "r", "s"].map(|key| self.symbols.lookup(key));
        let [Some(q), Some(r), Some(s)] = keys else {
            return Err("effect Hex coordinates missing".into());
        };
        value.hex([q, r, s], self.symbols).map(Some)
    }
    pub fn actions(self, name: &str) -> Result<Vec<ActionKind>, String> {
        self.ids(name)?
            .into_iter()
            .map(|id| match self.symbols.name(id) {
                "MOVEMENT" => Ok(ActionKind::Movement),
                "ATTACK" => Ok(ActionKind::Attack),
                "SKILL" => Ok(ActionKind::Skill),
                "DEFENSE" => Ok(ActionKind::Defense),
                "DEFENSE_SKILL" => Ok(ActionKind::DefenseSkill),
                "HOLD" => Ok(ActionKind::Hold),
                "CLEAR" => Ok(ActionKind::Clear),
                "FAST_TRAVEL" => Ok(ActionKind::FastTravel),
                _ => Err("invalid effect action".into()),
            })
            .collect()
    }
    pub fn minions(self, name: &str) -> Result<Vec<MinionKind>, String> {
        self.ids(name)?
            .into_iter()
            .map(|id| match self.symbols.name(id) {
                "MELEE" => Ok(MinionKind::Melee),
                "RANGED" => Ok(MinionKind::Ranged),
                "HEAVY" => Ok(MinionKind::Heavy),
                _ => Err("invalid effect minion type".into()),
            })
            .collect()
    }
}
