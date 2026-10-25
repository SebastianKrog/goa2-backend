//! Typed active-effect storage and lifecycle. Behavioral consumers are separate.
use crate::{
    data::{Datum, Registers, Symbol, Symbols},
    world::NativeWorld,
};
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duration {
    ThisTurn,
    NextTurn,
    ThisRound,
    Passive,
}
impl Duration {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "THIS_TURN" => Ok(Self::ThisTurn),
            "NEXT_TURN" => Ok(Self::NextTurn),
            "THIS_ROUND" => Ok(Self::ThisRound),
            "PASSIVE" => Ok(Self::Passive),
            _ => Err("unknown effect duration".into()),
        }
    }
}
#[derive(Clone, Debug)]
pub struct EffectDefinition {
    pub(crate) behavior: crate::effect_rules::Behavior,
    /// Complete typed payload, immutable and shared by worker clones.
    pub(crate) fields: Vec<(Symbol, Datum)>,
    pub kind: Symbol,
    pub affects: Symbol,
    pub(crate) scope: Vec<(Symbol, Datum)>,
    origin_key: Symbol,
    global_scope: bool,
    fixed_origin: bool,
    static_barrier: bool,
    attack: Symbol,
    pub finishing: Vec<Datum>,
    pub after_cards_played: bool,
}
#[derive(Clone, Debug)]
pub struct EffectInstance {
    pub definition: Arc<EffectDefinition>,
    pub id: Symbol,
    pub source: Symbol,
    pub card: Option<Symbol>,
    pub(crate) linked_card: Option<Symbol>,
    pub token: Option<Symbol>,
    pub duration: Duration,
    pub created_turn: i64,
    pub created_round: i64,
    pub active: bool,
    pub(crate) granted_passive: Option<Symbol>,
    pub(crate) passive_uses: i64,
    pub(crate) remaining_bounty: Option<i64>,
    pub scope_origin: Option<Symbol>,
    pub barrier_origin: Option<Symbol>,
    pub origin_basic: Option<bool>,
    pub(crate) origin_action: Option<Symbol>,
}
impl EffectInstance {
    fn compile(value: &Value, symbols: &mut Symbols) -> Result<Self, String> {
        let fields = value.as_object().ok_or("effect must be an object")?;
        let mut text = |key: &str| -> Result<Symbol, String> {
            Ok(symbols.intern(
                fields
                    .get(key)
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("effect {key} missing"))?,
            ))
        };
        let id = text("id")?;
        let source = text("source_id")?;
        let kind = fields
            .get("effect_type")
            .and_then(Value::as_str)
            .ok_or("effect type missing")?;
        let duration = Duration::parse(
            fields
                .get("duration")
                .and_then(Value::as_str)
                .ok_or("effect duration missing")?,
        )?;
        let card = optional_symbol(value, "source_card_id", symbols)?;
        let linked_card = card.filter(|id| !symbols.name(*id).is_empty());
        let token = optional_symbol(value, "token_id", symbols)?;
        let kind_name = kind.to_owned();
        let kind = symbols.intern(&kind_name);
        let scope_value = &value["scope"];
        let global_scope = match scope_value["shape"].as_str() {
            Some("global") => true,
            Some("point" | "radius" | "adjacent" | "line" | "zone") => false,
            _ => return Err("invalid effect scope shape".into()),
        };
        let affects = symbols.intern(
            scope_value["affects"]
                .as_str()
                .ok_or("effect scope affects missing")?,
        );
        let scope_origin = optional_symbol(scope_value, "origin_id", symbols)?;
        let barrier_origin = optional_symbol(value, "barrier_origin_id", symbols)?;
        let origin_basic = match value.get("origin_attack_is_basic") {
            None | Some(Value::Null) => None,
            Some(Value::Bool(v)) => Some(*v),
            _ => return Err("invalid originating attack flag".into()),
        };
        let origin_action = optional_symbol(value, "origin_action_type", symbols)?;
        let attack = symbols.intern("ATTACK");
        let origin_key = symbols.intern("origin_id");
        let scope = scope_value
            .as_object()
            .ok_or("effect scope missing")?
            .iter()
            .map(|(k, v)| Ok((symbols.intern(k), Datum::from_json(v, symbols)?)))
            .collect::<Result<_, String>>()?;
        let payload = fields
            .iter()
            .map(|(k, v)| Ok((symbols.intern(k), Datum::from_json(v, symbols)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let finishing = value["finishing_steps"]
            .as_array()
            .ok_or("effect finishing steps missing")?
            .iter()
            .map(|v| Datum::from_json(v, symbols))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            definition: Arc::new(EffectDefinition {
                behavior: crate::effect_rules::Behavior::compile(value, symbols)?,
                fields: payload,
                kind,
                affects,
                scope,
                origin_key,
                global_scope,
                fixed_origin: !scope_value["origin_hex"].is_null(),
                static_barrier: kind_name == "static_barrier",
                attack,
                finishing,
                after_cards_played: kind_name == "after_cards_played_trigger",
            }),
            id,
            granted_passive: optional_symbol(value, "granted_passive_effect_id", symbols)?,
            remaining_bounty: match &value["max_value"] {
                Value::Null => None,
                v => Some(v.as_i64().ok_or("invalid bounty limit")?),
            },
            passive_uses: value["passive_uses_this_turn"]
                .as_i64()
                .ok_or("invalid effect passive usage")?,
            scope_origin,
            barrier_origin,
            origin_basic,
            origin_action,
            source,
            card,
            linked_card,
            token,
            duration,
            created_turn: value["created_at_turn"]
                .as_i64()
                .ok_or("effect turn missing")?,
            created_round: value["created_at_round"]
                .as_i64()
                .ok_or("effect round missing")?,
            active: value["is_active"]
                .as_bool()
                .ok_or("effect activation missing")?,
        })
    }
    pub fn is_active(&self, round: i64, turn: i64) -> bool {
        if self.duration == Duration::Passive {
            return true;
        }
        if self.linked_card.is_some() && !self.active {
            return false;
        }
        match self.duration {
            Duration::ThisTurn => round == self.created_round && turn == self.created_turn,
            Duration::NextTurn => {
                round == self.created_round && turn.checked_sub(1) == Some(self.created_turn)
            }
            Duration::ThisRound => round == self.created_round,
            Duration::Passive => true,
        }
    }
    fn expires_this_turn(&self, round: i64, turn: i64) -> bool {
        self.token.is_none()
            && match self.duration {
                Duration::ThisTurn => true,
                Duration::NextTurn => round != self.created_round || turn > self.created_turn,
                _ => false,
            }
    }
    pub fn export(&self, symbols: &Symbols) -> Value {
        let mut fields = self.definition.fields.clone();
        let mut scope = self.definition.scope.clone();
        if let Some((_, origin)) = scope
            .iter_mut()
            .find(|(k, _)| *k == self.definition.origin_key)
        {
            *origin = self.scope_origin.map_or(Datum::Null, Datum::Text);
        }
        let mut set = |name: &str, value: Datum| {
            if let Some(key) = symbols.lookup(name)
                && let Some((_, v)) = fields.iter_mut().find(|(k, _)| *k == key)
            {
                *v = value;
            }
        };
        set(
            "origin_action_type",
            self.origin_action.map_or(Datum::Null, Datum::Text),
        );
        set("id", Datum::Text(self.id));
        set("source_id", Datum::Text(self.source));
        set("source_card_id", self.card.map_or(Datum::Null, Datum::Text));
        set("token_id", self.token.map_or(Datum::Null, Datum::Text));
        set("is_active", Datum::Bool(self.active));
        if self.definition.behavior.kind == crate::effect_rules::Kind::MinionBounty {
            set(
                "max_value",
                self.remaining_bounty.map_or(Datum::Null, Datum::Int),
            );
        }
        set("passive_uses_this_turn", Datum::Int(self.passive_uses));
        set(
            "granted_passive_effect_id",
            self.granted_passive.map_or(Datum::Null, Datum::Text),
        );
        set("created_at_turn", Datum::Int(self.created_turn));
        set("created_at_round", Datum::Int(self.created_round));
        set("scope", Datum::Object(scope));
        set(
            "barrier_origin_id",
            self.barrier_origin.map_or(Datum::Null, Datum::Text),
        );
        set(
            "origin_attack_is_basic",
            self.origin_basic.map_or(Datum::Null, Datum::Bool),
        );
        Datum::Object(fields).to_json(symbols)
    }
}
fn optional_symbol(
    value: &Value,
    key: &str,
    symbols: &mut Symbols,
) -> Result<Option<Symbol>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(symbols.intern(s))),
        _ => Err(format!("effect {key} must be text or null")),
    }
}
#[derive(Clone, Debug, Default)]
pub struct EffectStore {
    pub instances: Vec<EffectInstance>,
}
#[derive(Clone, Debug, Default)]
pub struct Expiry {
    pub ids: Vec<Symbol>,
    pub finishing: Vec<(Symbol, Vec<Datum>)>,
}
impl EffectStore {
    pub fn compile(value: &Value, symbols: &mut Symbols) -> Result<Self, String> {
        Ok(Self {
            instances: value
                .as_array()
                .ok_or("active effects must be an array")?
                .iter()
                .map(|v| EffectInstance::compile(v, symbols))
                .collect::<Result<_, _>>()?,
        })
    }
    pub fn export(&self, symbols: &Symbols) -> Value {
        Value::Array(self.instances.iter().map(|e| e.export(symbols)).collect())
    }
    fn collect(&self, predicate: impl Fn(&EffectInstance) -> bool) -> Expiry {
        let mut result = Expiry::default();
        for e in self.instances.iter().filter(|e| predicate(e)) {
            result.ids.push(e.id);
            if !e.definition.after_cards_played && !e.definition.finishing.is_empty() {
                result
                    .finishing
                    .push((e.source, e.definition.finishing.clone()));
            }
        }
        result
    }
    pub(crate) fn duration_indices(&self, duration: Duration) -> Vec<usize> {
        self.instances
            .iter()
            .enumerate()
            .filter_map(|(index, effect)| {
                (effect.token.is_none() && effect.duration == duration).then_some(index)
            })
            .collect()
    }
    pub(crate) fn turn_expiry_indices(&self, round: i64, turn: i64) -> Vec<usize> {
        self.instances
            .iter()
            .enumerate()
            .filter_map(|(index, effect)| effect.expires_this_turn(round, turn).then_some(index))
            .collect()
    }
    pub fn collect_turn_expiry(&self, round: i64, turn: i64) -> Expiry {
        self.collect(|e| e.expires_this_turn(round, turn))
    }
    pub fn collect_duration_expiry(&self, duration: Duration) -> Expiry {
        self.collect(|e| e.token.is_none() && e.duration == duration)
    }
    pub fn activate_card(&mut self, card: Symbol, active: bool) {
        for e in &mut self.instances {
            if e.card == Some(card) {
                e.active = active;
            }
        }
    }
    pub fn activate_id(&mut self, id: Symbol, active: bool) {
        if let Some(e) = self.instances.iter_mut().find(|e| e.id == id) {
            e.active = active;
        }
    }
    /// Source deliberately leaves card flags unchanged during this sweep.
    pub fn cleanup_stale(&mut self) {
        self.instances
            .retain(|e| e.card.is_none() || e.duration == Duration::Passive || e.active);
    }
}
impl NativeWorld {
    fn update_effect_card_flags(&mut self, cards: Vec<Symbol>) {
        for printed in cards {
            let active = self
                .effects
                .instances
                .iter()
                .any(|e| e.card == Some(printed));
            if let Some(id) = self.card_by_printed(printed) {
                self.card_mut(id).unwrap().active = active;
            }
        }
        self.has_effects = !self.effects.instances.is_empty();
        self.refresh_card_auras();
    }
    pub fn remove_effects(&mut self, ids: &[Symbol]) {
        let cards = self
            .effects
            .instances
            .iter()
            .filter(|e| ids.contains(&e.id))
            .filter_map(|e| e.linked_card)
            .collect();
        self.effects.instances.retain(|e| !ids.contains(&e.id));
        self.update_effect_card_flags(cards);
    }
    /// Premature removal never returns finishing steps, even for delayed triggers.
    pub fn expire_effects_by_card(&mut self, printed: Symbol) {
        self.effects.instances.retain(|e| e.card != Some(printed));
        if let Some(id) = self.card_by_printed(printed) {
            self.card_mut(id).unwrap().active = false;
        }
        self.has_effects = !self.effects.instances.is_empty();
        self.refresh_card_auras();
    }
    /// Removing a placed token ends its own effects without finishing payloads
    /// or card-flag refreshes. Match instances, never all rows sharing an ID.
    pub fn remove_token_effects(&mut self, token: Symbol) -> usize {
        let before = self.effects.instances.len();
        self.effects.instances.retain(|e| {
            e.token != Some(token) && e.source != token && e.scope_origin != Some(token)
        });
        self.has_effects = !self.effects.instances.is_empty();
        before - self.effects.instances.len()
    }
    pub fn expire_effects_by_source(&mut self, source: Symbol) {
        let mut expiring = Vec::new();
        let mut cards = Vec::new();
        for (index, e) in self.effects.instances.iter().enumerate() {
            if e.token.is_some() {
                continue;
            }
            let owner = e.linked_card.and_then(|card| {
                self.heroes
                    .iter()
                    .find(|h| self.card_for_hero_printed(h.id, card).is_some())
                    .map(|h| h.id)
            });
            if owner.unwrap_or(e.source) == source {
                expiring.push(index);
                if let Some(card) = e.linked_card {
                    cards.push(card);
                }
            }
        }
        // This sweep removes matching instances, not all rows sharing their IDs.
        let mut index = 0;
        self.effects.instances.retain(|_| {
            let keep = !expiring.contains(&index);
            index += 1;
            keep
        });
        self.update_effect_card_flags(cards);
    }
    pub fn expire_turn_effects(&mut self, round: i64, turn: i64) -> Expiry {
        let expiry = self.effects.collect_turn_expiry(round, turn);
        self.remove_effects(&expiry.ids);
        expiry
    }
    pub fn expire_duration_effects(&mut self, duration: Duration) -> Expiry {
        let expiry = self.effects.collect_duration_expiry(duration);
        let cards = self
            .effects
            .instances
            .iter()
            .filter(|e| e.token.is_none() && e.duration == duration)
            .filter_map(|e| e.linked_card)
            .collect();
        self.effects
            .instances
            .retain(|e| !expiry.ids.contains(&e.id));
        self.update_effect_card_flags(cards);
        expiry
    }
    /// Revelation removes trigger instances by type, never by their possibly
    /// duplicated IDs. All linked cards are refreshed, including stale triggers.
    pub(crate) fn remove_after_cards_played_effects(&mut self) {
        let cards = self
            .effects
            .instances
            .iter()
            .filter(|effect| effect.definition.after_cards_played)
            .filter_map(|effect| effect.linked_card)
            .collect();
        self.effects
            .instances
            .retain(|effect| !effect.definition.after_cards_played);
        self.update_effect_card_flags(cards);
    }
    pub fn cleanup_stale_effects(&mut self) {
        self.effects.cleanup_stale();
        self.has_effects = !self.effects.instances.is_empty();
    }
}

/// A validated, fully expanded creation template compiled at the boundary.
/// Definitions are shared across instantiations; runtime creation consumes no JSON.
#[derive(Clone, Debug)]
pub struct EffectCreateSpec {
    pub(crate) template: EffectInstance,
}
impl EffectCreateSpec {
    pub fn compile(value: &Value, symbols: &mut Symbols) -> Result<Self, String> {
        Ok(Self {
            template: EffectInstance::compile(value, symbols)?,
        })
    }
}
pub struct CreationContext<'a> {
    pub round: i64,
    pub turn: i64,
    pub actor: Option<Symbol>,
    pub registers: &'a Registers,
}
impl NativeWorld {
    pub fn create_effect(
        &mut self,
        spec: &EffectCreateSpec,
        context: &CreationContext<'_>,
        symbols: &mut Symbols,
    ) -> Result<usize, String> {
        let template = &spec.template;
        if let Some(card) = template.linked_card
            && let Some(index) = self.effects.instances.iter().position(|e| {
                e.card == Some(card)
                    && e.definition.kind == template.definition.kind
                    && e.definition.affects == template.definition.affects
            })
        {
            // Existing dormant rows still own their card instance: no refresh.
            return Ok(index);
        }
        let mut effect = template.clone();
        if effect.definition.behavior.kind == crate::effect_rules::Kind::MinionBounty {
            effect.remaining_bounty = effect.definition.behavior.maximum;
        }
        if !effect.definition.global_scope && !effect.definition.fixed_origin {
            let origin = effect
                .scope_origin
                .filter(|id| !symbols.name(*id).is_empty())
                .unwrap_or(effect.source);
            let board = self.resolved_actor(origin);
            if board != origin {
                effect.scope_origin = Some(board);
            }
        }
        if effect.definition.static_barrier {
            let origin = effect
                .barrier_origin
                .filter(|id| !symbols.name(*id).is_empty())
                .unwrap_or(effect.source);
            let board = self.resolved_actor(origin);
            if board != origin {
                effect.barrier_origin = Some(board);
            }
        }
        let card = effect
            .linked_card
            .and_then(|printed| self.card_by_printed(printed));
        if card
            .and_then(|id| self.card(id))
            .is_some_and(|c| c.lifecycle != crate::world::CardLifecycle::Unresolved)
        {
            effect.active = true;
        }
        if effect.origin_action == Some(effect.definition.attack) {
            let basic = match context.actor.filter(|id| self.hero(*id).is_some()) {
                Some(actor) => self
                    .performing_card(actor, context.registers)?
                    .and_then(|id| self.card(id))
                    .is_some_and(|c| self.definitions.cards[c.definition].basic),
                None => false,
            };
            effect.origin_basic = Some(basic);
        }
        let next = self
            .next_entity_id
            .checked_add(1)
            .ok_or("effect entity counter exhausted")?;
        effect.id = symbols.intern(&format!("eff_e_{}", self.next_entity_id));
        effect.created_round = context.round;
        effect.created_turn = context.turn;
        self.next_entity_id = next;
        let index = self.effects.instances.len();
        self.effects.instances.push(effect);
        self.has_effects = true;
        if let Some(id) = card {
            self.card_mut(id).unwrap().active = true;
        }
        self.refresh_card_auras();
        Ok(index)
    }
}
