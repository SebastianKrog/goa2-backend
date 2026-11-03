//! Spell lifecycle operations and observable source-compatible events.
use crate::data::{Symbol, Symbols};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpellOp {
    Cast,
    Wish,
    Prepare,
    Remove,
}

#[derive(Clone, Debug)]
pub enum SpellEvent {
    Cast {
        caster: Symbol,
        owner: Symbol,
        spell: Symbol,
    },
    Removed {
        caster: Symbol,
        owner: Symbol,
        spell: Symbol,
    },
    Prepared {
        actor: Symbol,
        returned: Vec<Symbol>,
        count: usize,
    },
    WishCount {
        caster: Symbol,
        count: i64,
        required: i64,
    },
}
impl SpellEvent {
    pub fn to_json(&self, symbols: &Symbols) -> Value {
        let (kind, actor, metadata) = match self {
            Self::Cast {
                caster,
                owner,
                spell,
            } => (
                "SPELL_CAST",
                *caster,
                json!({"spell_id":symbols.name(*spell),"owner_id":symbols.name(*owner),"caster_id":symbols.name(*caster)}),
            ),
            Self::Removed {
                caster,
                owner,
                spell,
            } => (
                "SPELL_REMOVED_FROM_SPELLBOOK",
                *caster,
                json!({"spell_id":symbols.name(*spell),"owner_id":symbols.name(*owner),"caster_id":symbols.name(*caster)}),
            ),
            Self::Prepared {
                actor,
                returned,
                count,
            } => (
                "SPELLBOOK_PREPARED",
                *actor,
                json!({"returned_spell_ids":returned.iter().map(|id|symbols.name(*id)).collect::<Vec<_>>(),"spellbook_count":count}),
            ),
            Self::WishCount {
                caster,
                count,
                required,
            } => (
                "WISH_CAST_COUNT_CHANGED",
                *caster,
                json!({"count":count,"required":required}),
            ),
        };
        json!({"event_type":kind,"actor_id":symbols.name(actor),"target_id":null,"from_hex":null,"to_hex":null,"metadata":metadata})
    }
}
