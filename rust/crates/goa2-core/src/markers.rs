//! Singleton marker mutations and copied-action substitution policy.
use crate::{
    data::{Datum, Registers, Symbol, Symbols},
    stats::{MarkerKind, NativeMarker},
    world::NativeWorld,
};
use serde_json::{Value, json};
#[derive(Clone, Debug)]
pub enum MarkerEvent {
    Placed {
        kind: Symbol,
        actor: Symbol,
        target: Symbol,
        value: i64,
    },
    Removed {
        kind: Symbol,
    },
}
impl MarkerEvent {
    pub fn to_json(&self, symbols: &Symbols) -> Value {
        match self {
            Self::Placed {
                kind,
                actor,
                target,
                value,
            } => {
                json!({"event_type":"MARKER_PLACED","actor_id":symbols.name(*actor),"target_id":symbols.name(*target),"from_hex":null,"to_hex":null,"metadata":{"marker_type":symbols.name(*kind),"value":value}})
            }
            Self::Removed { kind } => {
                json!({"event_type":"MARKER_REMOVED","actor_id":null,"target_id":null,"from_hex":null,"to_hex":null,"metadata":{"marker_type":symbols.name(*kind)}})
            }
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct MarkerPlacement {
    pub kind: MarkerKind,
    pub target: Symbol,
    pub value: i64,
}
#[derive(Clone, Debug)]
pub struct MarkerRules {
    keys: [Symbol; 3],
    system: Symbol,
    target: Symbol,
    skip: Symbol,
    substitution: Symbol,
}
impl MarkerRules {
    pub fn compile(s: &mut Symbols) -> Self {
        Self {
            keys: ["venom", "poison", "bounty"].map(|k| s.intern(k)),
            system: s.intern("system"),
            target: s.intern("marker_target_id"),
            skip: s.intern("skip_markers"),
            substitution: s.intern("substitution_actor_id"),
        }
    }
    pub fn place(
        &self,
        world: &mut NativeWorld,
        ctx: &mut Registers,
        symbols: &Symbols,
        actor: Option<Symbol>,
        placement: MarkerPlacement,
    ) -> Option<MarkerEvent> {
        let MarkerPlacement {
            kind,
            target,
            value,
        } = placement;
        if Self::suppressed(
            world,
            actor,
            ctx.get(self.skip).is_some_and(|v| v.truthy(symbols)),
            ctx.get(self.substitution).and_then(|value| {
                if let Datum::Text(value) = value {
                    Some(*value)
                } else {
                    None
                }
            }),
        ) {
            return None;
        }
        let source = actor
            .filter(|a| !symbols.name(*a).is_empty())
            .unwrap_or(self.system);
        let key = self.key(kind);
        if let Some(marker) = world.markers.iter_mut().find(|m| m.key == key) {
            marker.target = Some(target);
            marker.source = Some(source);
            marker.value = value;
        } else {
            world.markers.push(NativeMarker {
                key,
                source: Some(source),
                kind,
                target: Some(target),
                value,
            });
        }
        ctx.set(self.target, Datum::Text(target));
        Some(MarkerEvent::Placed {
            kind: key,
            actor: source,
            target,
            value,
        })
    }
    pub fn remove(&self, world: &mut NativeWorld, kind: MarkerKind) -> Option<MarkerEvent> {
        let key = self.key(kind);
        let marker = world.markers.iter_mut().find(|m| m.key == key)?;
        marker.target = None;
        marker.source = None;
        marker.value = 0;
        Some(MarkerEvent::Removed { kind: key })
    }
    pub(crate) fn suppressed(
        world: &NativeWorld,
        actor: Option<Symbol>,
        skip: bool,
        substitution: Option<Symbol>,
    ) -> bool {
        skip && actor.is_some_and(|a| substitution == Some(world.hero(a).map_or(a, |h| h.id)))
    }
    fn key(&self, kind: MarkerKind) -> Symbol {
        self.keys[match kind {
            MarkerKind::Venom => 0,
            MarkerKind::Poison => 1,
            MarkerKind::Bounty => 2,
        }]
    }
}
