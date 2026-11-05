//! Typed active-effect storage and lifecycle. Behavioral consumers are separate.
use crate::{
    data::{Datum, RegisterRead, Symbol, Symbols},
    world::NativeWorld,
};
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
#[derive(Clone, Debug, Hash)]
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
    pub reversed_initiative: bool,
}
#[derive(Clone, Debug, Hash)]
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
    /// Search-local binding only; equal effect IDs do not imply model identity.
    pub(crate) arena_model: Option<crate::objects::ObjectId>,
    pub scope_origin: Option<Symbol>,
    pub barrier_origin: Option<Symbol>,
    pub origin_basic: Option<bool>,
    pub(crate) origin_action: Option<Symbol>,
}
impl EffectInstance {
    fn compile(value: &Value, symbols: &mut Symbols) -> Result<Self, String> {
        let value = Datum::from_json(value, symbols)?;
        symbols.intern("ATTACK");
        symbols.intern("origin_id");
        Self::from_datum(&value, symbols)
    }
    pub(crate) fn from_datum(value: &Datum, symbols: &Symbols) -> Result<Self, String> {
        let record = crate::effect_record::Record::new(value, symbols)?;
        let scope = record.record("scope")?;
        let id = record.required_symbol("id")?;
        let source = record.required_symbol("source_id")?;
        let kind = record.required_symbol("effect_type")?;
        let card = record.symbol("source_card_id")?;
        let attack = symbols
            .lookup("ATTACK")
            .ok_or("effect ATTACK symbol missing")?;
        let origin_key = symbols
            .lookup("origin_id")
            .ok_or("effect origin field missing")?;
        let behavior = crate::effect_rules::Behavior::from_record(record)?;
        let finishing = record.list("finishing_steps")?.to_vec();
        let origin_basic = match record.get("origin_attack_is_basic") {
            Datum::Null => None,
            Datum::Bool(value) => Some(*value),
            _ => return Err("invalid originating attack flag".into()),
        };
        Ok(Self {
            definition: Arc::new(EffectDefinition {
                global_scope: behavior.shape == crate::effect_rules::Shape::Global,
                fixed_origin: behavior.origin_hex.is_some(),
                static_barrier: symbols.name(kind) == "static_barrier",
                behavior,
                fields: record.fields.to_vec(),
                kind,
                affects: scope.required_symbol("affects")?,
                scope: scope.fields.to_vec(),
                origin_key,
                attack,
                finishing,
                after_cards_played: symbols.name(kind) == "after_cards_played_trigger",
                reversed_initiative: symbols.name(kind) == "reversed_initiative",
            }),
            id,
            source,
            card,
            linked_card: card.filter(|id| !symbols.name(*id).is_empty()),
            token: record.symbol("token_id")?,
            duration: Duration::parse(record.text("duration").ok_or("effect duration missing")?)?,
            created_turn: record.integer("created_at_turn")?,
            created_round: record.integer("created_at_round")?,
            active: record.flag("is_active")?,
            granted_passive: record.symbol("granted_passive_effect_id")?,
            passive_uses: record.integer("passive_uses_this_turn")?,
            remaining_bounty: record.optional_integer("max_value")?,
            arena_model: None,
            scope_origin: scope.symbol("origin_id")?,
            barrier_origin: record.symbol("barrier_origin_id")?,
            origin_basic,
            origin_action: record.symbol("origin_action_type")?,
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
        self.payload(symbols).to_json(symbols)
    }
    pub(crate) fn payload(&self, symbols: &Symbols) -> Datum {
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
        Datum::Object(fields)
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
    // Effects can be changed in place, reordered or removed. Read the current
    // ordered rows here until their mutations share a journal.
    pub(crate) fn fingerprint(&self) -> u128 {
        use std::hash::{DefaultHasher, Hash, Hasher};

        let lane = |salt| {
            let mut hash = DefaultHasher::new();
            ("active effects", salt, &self.instances).hash(&mut hash);
            hash.finish()
        };
        u128::from(lane(0u64)) | (u128::from(lane(1u64)) << 64)
    }

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
    /// The global initiative rule ignores immunity, duration and token binding.
    pub(crate) fn initiative_reversed(&self, round: i64, turn: i64) -> bool {
        self.instances.iter().any(|effect| {
            effect.definition.reversed_initiative
                && effect.active
                && effect.created_round == round
                && turn.checked_sub(1) == Some(effect.created_turn)
        })
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
    pub(crate) fn expiring_source_rows(&self, source: Symbol) -> Vec<usize> {
        self.effects
            .instances
            .iter()
            .enumerate()
            .filter_map(|(index, e)| {
                if e.token.is_some() {
                    return None;
                }
                let owner = e.linked_card.and_then(|card| {
                    self.heroes
                        .iter()
                        .find(|h| self.card_for_hero_printed(h.id, card).is_some())
                        .map(|h| h.id)
                });
                (owner.unwrap_or(e.source) == source).then_some(index)
            })
            .collect()
    }
    pub fn expire_effects_by_source(&mut self, source: Symbol) {
        let expiring = self.expiring_source_rows(source);
        let cards = expiring
            .iter()
            .filter_map(|index| self.effects.instances[*index].linked_card)
            .collect();
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
    pub registers: &'a dyn RegisterRead,
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
