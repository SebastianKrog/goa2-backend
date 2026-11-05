//! Passive offers retain provider identities; execution re-resolves live cards.
use crate::{
    cards::CardId,
    data::{Datum, Registers, Symbol, Symbols},
    world::{CardLifecycle, NativeWorld},
};

#[derive(Clone, Copy, Debug, Hash)]
pub struct Offer {
    pub card: Symbol,
    pub trigger: Symbol,
    pub optional: bool,
    pub prompt: Symbol,
    pub hero: Option<Symbol>,
    pub provider: Option<Symbol>,
}
struct PassiveSource {
    physical: CardId,
    owner: Symbol,
    definition: usize,
    card: Symbol,
    effect: Symbol,
    uses: i64,
    provider: Option<Symbol>,
}
#[derive(Clone, Debug)]
struct Config {
    trigger: Symbol,
    limit: i64,
    optional: bool,
    prompt: Option<Symbol>,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Predicate {
    Default,
    Jinx,
    Legend,
    BattleFury,
    Archwizard,
    RuneMastery,
    BulletTime,
    Winter,
    DragonKnight,
}
#[derive(Clone, Debug)]
pub struct PassiveRules {
    configs: Vec<(Symbol, Vec<Config>)>,
    prompts: Vec<Symbol>,
    triggers: Vec<Symbol>,
    gun: Symbol,
    wild: Symbol,
    before_action: Symbol,
    after_push: Symbol,
    victim: Symbol,
    predicates: Vec<(Symbol, Predicate)>,
}
impl PassiveRules {
    pub fn compile(world: &NativeWorld, symbols: &mut Symbols) -> Result<Self, String> {
        let source = crate::templates::instructions()?;
        let mut configs = Vec::new();
        for (name, rows) in source["passive_configs"]
            .as_object()
            .ok_or("missing passive configs")?
        {
            let rows = rows.as_array().ok_or("invalid passive configs")?;
            if rows.is_empty() {
                continue;
            }
            let rows = rows
                .iter()
                .map(|row| {
                    Ok(Config {
                        trigger: symbols
                            .intern(row["trigger"].as_str().ok_or("invalid passive trigger")?),
                        limit: row["uses"].as_i64().ok_or("invalid passive usage limit")?,
                        optional: row["optional"]
                            .as_bool()
                            .ok_or("invalid passive optional flag")?,
                        prompt: match &row["prompt"] {
                            serde_json::Value::Null => None,
                            serde_json::Value::String(s) => {
                                (!s.is_empty()).then(|| symbols.intern(s))
                            }
                            _ => return Err("invalid passive prompt".into()),
                        },
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            configs.push((symbols.intern(name), rows));
        }
        let prompts = world
            .definitions
            .cards
            .iter()
            .map(|c| symbols.intern(&format!("Use {} passive ability?", symbols.name(c.name))))
            .collect();
        let triggers = source["passive_triggers"]
            .as_array()
            .ok_or("missing passive triggers")?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(|v| symbols.intern(v))
                    .ok_or("invalid passive trigger".into())
            })
            .collect::<Result<_, String>>()?;
        let predicates = source["passive_offer_predicates"]
            .as_object()
            .ok_or("missing passive predicate metadata")?
            .iter()
            .map(|(effect, method)| {
                let predicate = match method
                    .as_str()
                    .ok_or("invalid passive predicate metadata")?
                {
                    "JinxEffect.should_offer_passive" => Predicate::Jinx,
                    "LegendOfTheSkiesEffect.should_offer_passive" => Predicate::Legend,
                    "BattleFuryEffect.should_offer_passive" => Predicate::BattleFury,
                    "TheArchwizardEffect.should_offer_passive" => Predicate::Archwizard,
                    "RuneMasteryEffect.should_offer_passive" => Predicate::RuneMastery,
                    "BulletTimeEffect.should_offer_passive" => Predicate::BulletTime,
                    "ReignOfWinterEffect.should_offer_passive" => Predicate::Winter,
                    "DragonKnightEffect.should_offer_passive" => Predicate::DragonKnight,
                    _ => return Err("uncompiled passive eligibility predicate".to_owned()),
                };
                Ok((symbols.intern(effect), predicate))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            predicates,
            configs,
            prompts,
            triggers,
            gun: symbols.intern("big_sodding_gun"),
            wild: symbols.intern("wild_hunt"),
            before_action: symbols.intern("before_action"),
            after_push: symbols.intern("after_push"),
            victim: symbols.intern("push_victim_id"),
        })
    }
    pub fn scan(
        &self,
        world: &NativeWorld,
        actor: Option<Symbol>,
        hero: Option<Symbol>,
        trigger: Symbol,
    ) -> Result<Vec<Offer>, String> {
        self.scan_with(world, actor, hero, trigger, |_, _, effect| {
            if effect == self.gun || effect == self.wild {
                Ok(true)
            } else {
                Err("eligible passive execution not compiled".into())
            }
        })
    }
    pub(crate) fn predicate(&self, effect: Symbol) -> Predicate {
        self.predicates
            .iter()
            .find_map(|(id, p)| (*id == effect).then_some(*p))
            .unwrap_or(Predicate::Default)
    }
    pub(crate) fn validate_trigger(&self, trigger: Symbol) -> Result<(), String> {
        if self.triggers.contains(&trigger) {
            Ok(())
        } else {
            Err("unknown passive trigger".into())
        }
    }
    pub(crate) fn scan_with(
        &self,
        world: &NativeWorld,
        actor: Option<Symbol>,
        hero: Option<Symbol>,
        trigger: Symbol,
        mut eligible: impl FnMut(Symbol, CardId, Symbol) -> Result<bool, String>,
    ) -> Result<Vec<Offer>, String> {
        let Some(h) = hero.or(actor).and_then(|id| world.hero(id)) else {
            return Ok(Vec::new());
        };
        if !self.triggers.contains(&trigger) {
            return Err("unknown passive trigger".into());
        }
        let mut offers = Vec::new();
        let mut seen = Vec::new();
        // Ordinary aliases are intentionally not deduplicated. The seen set only
        // suppresses the later active-effect source scan, as in Python.
        for id in h
            .played
            .iter()
            .flatten()
            .filter(|id| {
                world
                    .card(**id)
                    .is_some_and(|c| c.lifecycle == CardLifecycle::Resolved && !c.facedown)
            })
            .chain(h.ultimate.iter().filter(|_| h.level >= 8))
        {
            let c = world.card(*id).ok_or("passive card missing")?;
            let d = &world.definitions.cards[c.definition];
            seen.push(d.printed_id);
            if c.facedown {
                continue;
            }
            self.add_offers(
                &mut offers,
                PassiveSource {
                    physical: *id,
                    owner: h.id,
                    definition: c.definition,
                    card: d.printed_id,
                    effect: d.effect,
                    uses: c.passive_uses,
                    provider: None,
                },
                trigger,
                hero.map(|_| h.id),
                &mut eligible,
            )?;
        }
        for e in &world.effects.instances {
            if e.source != h.id || !e.active {
                continue;
            }
            let Some(card) = e.linked_card.filter(|id| !seen.contains(id)) else {
                continue;
            };
            let Some(granted) = e
                .granted_passive
                .filter(|id| *id != world.definitions.effect_empty)
            else {
                continue;
            };
            let Some(id) = world.card_by_printed(card) else {
                continue;
            };
            seen.push(card);
            self.add_offers(
                &mut offers,
                PassiveSource {
                    physical: id,
                    owner: h.id,
                    definition: world.card(id).unwrap().definition,
                    card,
                    effect: granted,
                    uses: e.passive_uses,
                    provider: Some(e.id),
                },
                trigger,
                hero.map(|_| h.id),
                &mut eligible,
            )?;
        }
        Ok(offers)
    }
    fn add_offers(
        &self,
        offers: &mut Vec<Offer>,
        source: PassiveSource,
        trigger: Symbol,
        hero: Option<Symbol>,
        eligible: &mut impl FnMut(Symbol, CardId, Symbol) -> Result<bool, String>,
    ) -> Result<(), String> {
        let PassiveSource {
            physical,
            owner,
            definition,
            card,
            effect,
            uses,
            provider,
        } = source;
        if let Some((_, configs)) = self.configs.iter().find(|(e, _)| *e == effect) {
            for config in configs {
                if config.trigger != trigger || config.limit > 0 && uses >= config.limit {
                    continue;
                }
                if !eligible(owner, physical, effect)? {
                    continue;
                }
                offers.push(Offer {
                    card,
                    trigger,
                    hero,
                    provider,
                    optional: config.optional,
                    prompt: config.prompt.unwrap_or(self.prompts[definition]),
                });
            }
        }
        Ok(())
    }
    pub fn card(world: &NativeWorld, owner: Symbol, printed: Symbol) -> Option<CardId> {
        let hero = world.hero(owner)?;
        hero.played
            .iter()
            .flatten()
            .chain(hero.ultimate.iter())
            .copied()
            .find(|id| {
                world
                    .card(*id)
                    .is_some_and(|c| world.definitions.cards[c.definition].printed_id == printed)
            })
    }
    /// Re-resolve the provider at execution; scans never retain physical pointers.
    pub fn resolve(
        &self,
        world: &NativeWorld,
        owner: Symbol,
        offer: Offer,
    ) -> Option<(CardId, Symbol)> {
        if let Some(provider) = offer.provider {
            let e = world.effects.instances.iter().find(|e| {
                e.id == provider
                    && e.active
                    && e.source == owner
                    && e.card == Some(offer.card)
                    && e.granted_passive
                        .is_some_and(|id| id != world.definitions.effect_empty)
            })?;
            Some((world.card_by_printed(offer.card)?, e.granted_passive?))
        } else {
            let id = Self::card(world, owner, offer.card)?;
            let c = world.card(id)?;
            let d = &world.definitions.cards[c.definition];
            (!c.facedown && d.has_effect).then_some((id, d.effect))
        }
    }
    pub fn body(
        &self,
        world: &NativeWorld,
        ctx: &Registers,
        symbols: &Symbols,
        owner: Symbol,
        effect: Symbol,
        trigger: Symbol,
    ) -> Result<bool, String> {
        if effect == self.wild {
            return Ok(trigger == self.before_action);
        }
        if effect != self.gun {
            return Err("eligible passive execution not compiled".into());
        }
        if trigger != self.after_push {
            return Ok(false);
        }
        let Some(value) = ctx.get(self.victim).filter(|v| v.truthy(symbols)) else {
            return Ok(false);
        };
        let Datum::Text(victim) = value else {
            return Err("non-string passive victim coercion not compiled".into());
        };
        let Some(victim) = world.hero(*victim) else {
            return Ok(false);
        };
        let owner = world.hero(owner).ok_or("passive owner missing")?;
        Ok(world.entity(victim.id).and_then(|e| e.team)
            != world.entity(owner.id).and_then(|e| e.team))
    }
    pub fn mark(
        world: &mut NativeWorld,
        actor: Option<Symbol>,
        printed: Symbol,
        provider: Option<Symbol>,
    ) -> Result<(), String> {
        if let Some(provider) = provider {
            if let Some(e) = world
                .effects
                .instances
                .iter_mut()
                .find(|e| e.id == provider && e.card == Some(printed))
            {
                e.passive_uses = e
                    .passive_uses
                    .checked_add(1)
                    .ok_or("effect passive usage overflow")?;
            }
        } else if let Some(card) = actor.and_then(|a| Self::card(world, a, printed)) {
            let c = world.card_mut(card).ok_or("passive card missing")?;
            c.passive_uses = c
                .passive_uses
                .checked_add(1)
                .ok_or("passive usage overflow")?;
        }
        Ok(())
    }
}
