//! Physical card references and typed hero collections. A card ID indexes the
//! global instance table, never a printed card name or a copied card record.
use crate::{
    data::{Symbol, Symbols},
    stats::Stat,
};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CardId(pub u32);
#[derive(Clone, Debug, Hash)]
pub struct HeroRecord {
    pub id: Symbol,
    pub team: Symbol,
    pub team_index: usize,
    pub level: i64,
    pub gold: i64,
    pub deck: Vec<CardId>,
    pub hand: Vec<CardId>,
    pub played: Vec<Option<CardId>>,
    pub discard: Vec<CardId>,
    pub spells: Vec<CardId>,
    pub current: Option<CardId>,
    pub extra: Option<CardId>,
    pub ultimate: Option<CardId>,
    pub resolved_turns: i64,
    pub items: [i64; 6],
    pub items_order: Vec<Stat>,
    pub pieces: Vec<Symbol>,
    pub uncompiled_auras: bool,
}
impl HeroRecord {
    pub fn card_in_turn_slot(&self, turn: i64) -> Option<CardId> {
        turn.checked_sub(1)
            .and_then(|slot| usize::try_from(slot).ok())
            .and_then(|slot| self.played.get(slot))
            .copied()
            .flatten()
    }
    pub fn compile(
        raw: &Value,
        team: Symbol,
        team_index: usize,
        symbols: &mut Symbols,
    ) -> Result<Self, String> {
        let id = symbols.intern(raw["id"].as_str().ok_or("hero identity missing")?);
        let list = |key: &str| -> Result<Vec<CardId>, String> {
            raw[key]
                .as_array()
                .ok_or_else(|| format!("hero collection missing: {key}"))?
                .iter()
                .map(|v| reference(v)?.ok_or("null card in non-null collection".into()))
                .collect()
        };
        let played = raw["played_cards"]
            .as_array()
            .ok_or("played collection missing")?
            .iter()
            .map(reference)
            .collect::<Result<Vec<_>, _>>()?;
        let mut items = [0; 6];
        let mut items_order = Vec::new();
        for (name, value) in raw["items"].as_object().ok_or("item table missing")? {
            let stat = Stat::parse(name)?;
            items[stat.index()] = value.as_i64().ok_or("non-integer item bonus")?;
            items_order.push(stat);
        }
        let supply = raw["piece_supply"].as_u64().ok_or("piece supply missing")?;
        if supply > 1024 {
            return Err("unsupported piece supply".into());
        }
        let name = symbols.name(id).to_owned();
        let pieces = (1..=supply)
            .map(|i| symbols.intern(&format!("{name}_piece_{i}")))
            .collect();
        Ok(Self {
            id,
            team,
            team_index,
            level: raw["level"].as_i64().ok_or("hero level missing")?,
            gold: raw["gold"].as_i64().ok_or("hero gold missing")?,
            deck: list("deck")?,
            hand: list("hand")?,
            discard: list("discard_pile")?,
            spells: list("spells")?,
            played,
            current: reference(&raw["current_turn_card"])?,
            extra: reference(&raw["extra_turn_card"])?,
            ultimate: reference(&raw["ultimate_card"])?,
            resolved_turns: raw["resolved_turn_count"]
                .as_i64()
                .ok_or("resolved turn count missing")?,
            items,
            items_order,
            pieces,
            uncompiled_auras: false,
        })
    }
    pub fn export_collections(&self, state: &mut Value, symbols: &Symbols) {
        let hero = &mut state["teams"][symbols.name(self.team)]["heroes"][self.team_index];
        hero["gold"] = json!(self.gold);
        hero["level"] = json!(self.level);
        hero["items"] = Value::Object(
            self.items_order
                .iter()
                .map(|s| (s.name().into(), json!(self.items[s.index()])))
                .collect(),
        );
        hero["deck"] = Value::Array(self.deck.iter().map(|id| wire(Some(*id))).collect());
        hero["hand"] = Value::Array(self.hand.iter().map(|id| wire(Some(*id))).collect());
        hero["played_cards"] = Value::Array(self.played.iter().map(|id| wire(*id)).collect());
        hero["discard_pile"] =
            Value::Array(self.discard.iter().map(|id| wire(Some(*id))).collect());
        hero["spells"] = Value::Array(self.spells.iter().map(|id| wire(Some(*id))).collect());
        hero["current_turn_card"] = wire(self.current);
        hero["extra_turn_card"] = wire(self.extra);
        hero["ultimate_card"] = wire(self.ultimate);
        hero["resolved_turn_count"] = json!(self.resolved_turns);
    }
}
pub fn reference(value: &Value) -> Result<Option<CardId>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let fields = value.as_object().ok_or("card reference is not an object")?;
    if fields.len() != 1 {
        return Err("compact physical-card references required for native hero state".into());
    }
    let id = fields
        .get("$goa2_instance")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or("invalid physical card reference")?;
    Ok(Some(CardId(id)))
}
pub(crate) fn wire(id: Option<CardId>) -> Value {
    id.map_or(Value::Null, |id| json!({"$goa2_instance":id.0}))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardError {
    Illegal(&'static str),
    Unsupported(&'static str),
}
impl std::fmt::Display for CardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CardError {}
