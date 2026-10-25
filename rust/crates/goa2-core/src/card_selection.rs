//! Ordered physical-card candidate selection. Definitions remain shared;
//! current containers, lifecycle, visibility and flags are read on every scan.
use crate::{
    cards::CardId,
    data::{Datum, Registers, Symbol, Symbols},
    world::{ActionKind, CardLifecycle, NativeWorld},
};
use serde::Deserialize;
use serde_json::Value;
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Container {
    Hand,
    Played,
    Discard,
    Deck,
}
#[derive(Clone, Debug)]
pub struct ForceBinding {
    pub victim: Symbol,
    pub basic: Option<bool>,
    pub shields: bool,
    pub allowed: Option<std::sync::Arc<[Symbol]>>,
}
#[derive(Clone, Debug)]
pub struct CardSelectSpec {
    pub output: Symbol,
    pub prompt: Symbol,
    pub auto: bool,
    pub discard: bool,
    pub hero: Option<Symbol>,
    pub hero_key: Option<Symbol>,
    pub owner: Option<Symbol>,
    pub owner_key: Option<Symbol>,
    containers: Vec<Container>,
    shields: bool,
    facedown: bool,
    actions: Option<Vec<ActionKind>>,
    colors: Vec<Symbol>,
    color_key: Option<Symbol>,
    tier_key: Option<Symbol>,
    basic: Option<bool>,
    active: Option<bool>,
    item: Option<bool>,
    allowed: Option<Vec<Symbol>>,
    exclude: Vec<Symbol>,
    states: Option<Vec<CardLifecycle>>,
    exclude_states: Vec<CardLifecycle>,
    color_output: Option<Symbol>,
    tier_output: Option<Symbol>,
}
impl CardSelectSpec {
    pub fn compile(raw: &Value, s: &mut Symbols) -> Result<Self, String> {
        if raw["filters"].as_array().is_none_or(|v| !v.is_empty()) {
            return Err("generic CARD predicate closure not compiled".into());
        }
        fn field<T: serde::de::DeserializeOwned>(v: &Value, k: &str) -> Result<Option<T>, String> {
            v.get(k)
                .filter(|v| !v.is_null())
                .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
                .transpose()
        }
        fn symbol(v: &Value, k: &str, s: &mut Symbols) -> Result<Option<Symbol>, String> {
            match v.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(v)) => Ok((!v.is_empty()).then(|| s.intern(v))),
                _ => Err(format!("invalid card selection symbol {k}")),
            }
        }
        fn ids(v: &Value, k: &str, s: &mut Symbols) -> Result<Option<Vec<Symbol>>, String> {
            v.get(k)
                .filter(|v| !v.is_null())
                .map(|v| {
                    v.as_array()
                        .ok_or("invalid card selection identity list".to_string())?
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(|v| s.intern(v))
                                .ok_or("non-string card selection identity".into())
                        })
                        .collect()
                })
                .transpose()
        }
        let mut containers: Vec<Container> = field(raw, "card_containers")?.unwrap_or_default();
        if containers.is_empty() {
            containers.push(field(raw, "card_container")?.unwrap_or(Container::Hand));
        }
        let colors = ids(raw, "card_colors", s)?.unwrap_or_default();
        for c in &colors {
            validate_color(s.name(*c))?;
        }
        Ok(Self {
            output: s.intern(raw["output_key"].as_str().unwrap_or("selection")),
            prompt: s.intern(raw["prompt"].as_str().ok_or("missing card prompt")?),
            auto: field(raw, "auto_select_if_one")?.unwrap_or(false),
            discard: raw["decision_purpose"] == "discard",
            hero: symbol(raw, "context_hero_id", s)?,
            hero_key: symbol(raw, "context_hero_id_key", s)?,
            owner: symbol(raw, "override_player_id", s)?,
            owner_key: symbol(raw, "override_player_id_key", s)?,
            containers,
            shields: field(raw, "restrict_played_to_shields")?.unwrap_or(false),
            facedown: field(raw, "include_facedown")?.unwrap_or(false),
            actions: field(raw, "card_action_types")?,
            colors,
            color_key: symbol(raw, "card_color_key", s)?,
            tier_key: symbol(raw, "card_tier_key", s)?,
            basic: field(raw, "card_is_basic")?,
            active: field(raw, "card_is_active")?,
            item: field(raw, "card_has_item")?,
            allowed: ids(raw, "allowed_card_ids", s)?,
            exclude: ids(raw, "exclude_card_id_keys", s)?.unwrap_or_default(),
            states: field(raw, "card_states")?,
            exclude_states: field(raw, "exclude_card_states")?.unwrap_or_default(),
            color_output: symbol(raw, "selected_card_color_key", s)?,
            tier_output: symbol(raw, "selected_card_tier_key", s)?,
        })
    }
    pub fn owner(
        &self,
        actor: Option<Symbol>,
        ctx: &Registers,
        symbols: &Symbols,
        forced: Option<&ForceBinding>,
    ) -> Result<Option<Symbol>, String> {
        if let Some(f) = forced {
            return Ok(Some(f.victim));
        }
        if self.owner.is_some() {
            Ok(self.owner)
        } else {
            Ok(text_key(ctx, symbols, self.owner_key)?.or(actor))
        }
    }
    pub fn choices(
        &self,
        world: &NativeWorld,
        ctx: &Registers,
        symbols: &Symbols,
        owner: Option<Symbol>,
        forced: Option<&ForceBinding>,
    ) -> Result<Vec<CardId>, String> {
        let hero = if let Some(f) = forced {
            Some(f.victim)
        } else {
            if self.hero.is_some() {
                self.hero
            } else {
                text_key(ctx, symbols, self.hero_key)?.or(owner)
            }
        };
        let Some(hero) = hero.and_then(|h| world.hero(h)) else {
            return Ok(Vec::new());
        };
        let mut cards = Vec::new();
        let forced_containers = [Container::Hand, Container::Played];
        let containers = if forced.is_some_and(|f| f.shields) {
            &forced_containers[..]
        } else {
            &self.containers
        };
        for c in containers {
            match c {
                Container::Hand => cards.extend(hero.hand.iter().copied()),
                Container::Played => {
                    if self.shields || forced.is_some_and(|f| f.shields) {
                        cards.extend(world.active_shield_cards(hero.id));
                    } else {
                        cards.extend(hero.played.iter().flatten().copied());
                    }
                }
                Container::Discard => cards.extend(hero.discard.iter().copied()),
                Container::Deck => cards.extend(hero.deck.iter().copied()),
            }
        }
        let mut colors = self.colors.clone();
        if let Some(c) = text_key(ctx, symbols, self.color_key)? {
            validate_color(symbols.name(c))?;
            colors.push(c);
        }
        let tier = text_key(ctx, symbols, self.tier_key)?;
        if let Some(t) = tier
            && !matches!(symbols.name(t), "I" | "II" | "III" | "IV" | "UNTIERED")
        {
            return Err("invalid card tier context".into());
        }
        let basic = forced.map_or(self.basic, |f| if f.shields { None } else { f.basic });
        let allowed = forced
            .filter(|f| f.shields)
            .map_or(self.allowed.as_deref(), |f| f.allowed.as_deref());
        let identity =
            self.actions.is_some() || !colors.is_empty() || tier.is_some() || basic.is_some();
        let hide = containers
            .iter()
            .any(|c| matches!(c, Container::Played | Container::Discard));
        let excluded = self
            .exclude
            .iter()
            .map(|key| text_key(ctx, symbols, Some(*key)))
            .collect::<Result<Vec<_>, _>>()?;
        let mut selected = Vec::new();
        for id in cards {
            let c = world.card(id).ok_or("card candidate missing")?;
            let d = &world.definitions.cards[c.definition];
            let masked = c.facedown
                && matches!(
                    c.lifecycle,
                    CardLifecycle::Discard | CardLifecycle::Resolved
                );
            if masked && ((!self.facedown && hide) || identity) {
                continue;
            }
            if self
                .actions
                .as_ref()
                .is_some_and(|a| d.primary.is_none_or(|p| !a.contains(&p)))
            {
                continue;
            }
            if !colors.is_empty() && d.color.is_none_or(|c| !colors.contains(&c)) {
                continue;
            }
            if tier.is_some_and(|t| t != d.tier) || basic.is_some_and(|b| b != d.basic) {
                continue;
            }
            if self.active.is_some_and(|a| {
                a != (c.active || c.enraged && c.lifecycle == CardLifecycle::Resolved)
            }) {
                continue;
            }
            if self.item.is_some_and(|i| i != d.has_item) {
                continue;
            }
            if allowed.is_some_and(|a| !a.contains(&d.printed_id))
                || excluded.contains(&Some(d.printed_id))
            {
                continue;
            }
            if self
                .states
                .as_ref()
                .is_some_and(|s| !s.contains(&c.lifecycle))
                || self.exclude_states.contains(&c.lifecycle)
            {
                continue;
            }
            selected.push(id);
        }
        Ok(selected)
    }
    pub fn store(
        &self,
        world: &NativeWorld,
        ctx: &mut Registers,
        choices: &[CardId],
        printed: Symbol,
    ) -> Result<(), String> {
        let id = choices
            .iter()
            .find(|id| {
                world
                    .card(**id)
                    .is_some_and(|c| world.definitions.cards[c.definition].printed_id == printed)
            })
            .ok_or("selected card no longer present")?;
        let c = world.card(*id).ok_or("selected card missing")?;
        let d = &world.definitions.cards[c.definition];
        ctx.set(self.output, Datum::Text(printed));
        if let Some(k) = self.color_output {
            ctx.set(k, d.color.map_or(Datum::Null, Datum::Text));
        }
        if let Some(k) = self.tier_output {
            ctx.set(k, Datum::Text(d.tier));
        }
        Ok(())
    }
}
fn text_key(
    ctx: &Registers,
    symbols: &Symbols,
    key: Option<Symbol>,
) -> Result<Option<Symbol>, String> {
    match key.and_then(|k| ctx.get(k)).filter(|v| v.truthy(symbols)) {
        None => Ok(None),
        Some(Datum::Text(s)) => Ok(Some(*s)),
        _ => Err("non-string CARD context coercion not compiled".into()),
    }
}
pub(crate) fn validate_color(s: &str) -> Result<(), String> {
    if matches!(s, "GOLD" | "SILVER" | "RED" | "BLUE" | "GREEN" | "PURPLE") {
        Ok(())
    } else {
        Err("invalid card color".into())
    }
}

impl NativeWorld {
    /// Direct active-flag/creator lookup; shield cards are not temporal scopes.
    pub(crate) fn active_shield_cards(&self, owner: Symbol) -> Vec<CardId> {
        let Some(hero) = self.hero(owner) else {
            return Vec::new();
        };
        let shields = self
            .effects
            .instances
            .iter()
            .filter(|e| {
                e.active
                    && e.source == hero.id
                    && e.definition.behavior.kind == crate::effect_rules::Kind::DiscardShield
            })
            .filter_map(|e| e.linked_card)
            .collect::<Vec<_>>();
        let mut seen = Vec::new();
        hero.played
            .iter()
            .flatten()
            .chain(hero.current.iter())
            .copied()
            .filter(|id| {
                let printed = self.definitions.cards[self.card(*id).unwrap().definition].printed_id;
                if shields.contains(&printed) && !seen.contains(&printed) {
                    seen.push(printed);
                    true
                } else {
                    false
                }
            })
            .collect()
    }
}
