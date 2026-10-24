//! Passive providers are scanned now; offers re-resolve their card when executed.
//! Immutable configs/prompts are shared, while usage counters live on physical cards.
use crate::{
    cards::CardId,
    data::{Datum, Registers, Symbol, Symbols},
    world::{CardLifecycle, NativeWorld},
};

#[derive(Clone, Copy, Debug)]
pub struct Offer {
    pub card: Symbol,
    pub trigger: Symbol,
    pub optional: bool,
    pub prompt: Symbol,
    pub hero: Option<Symbol>,
}
#[derive(Clone, Debug)]
struct Config {
    trigger: Symbol,
    limit: i64,
    optional: bool,
    prompt: Option<String>,
}
#[derive(Clone, Debug)]
pub struct PassiveRules {
    configs: Vec<(Symbol, Vec<Config>)>,
    prompts: Vec<Vec<Symbol>>,
    triggers: Vec<Symbol>,
    gun: Symbol,
    after_push: Symbol,
    victim: Symbol,
}
impl PassiveRules {
    pub fn compile(world: &NativeWorld, symbols: &mut Symbols) -> Result<Self, String> {
        let source = crate::templates::instructions()?;
        let mut configs = Vec::new();
        for (name, rows) in source["passive_configs"]
            .as_object()
            .ok_or("missing passive configs")?
        {
            let rows = rows
                .as_array()
                .ok_or("invalid passive configs")?
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
                            serde_json::Value::String(s) => Some(s.clone()),
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
            .map(|card| {
                configs
                    .iter()
                    .find(|(effect, _)| *effect == card.effect)
                    .map_or_else(Vec::new, |(_, rows)| {
                        rows.iter()
                            .map(|c| {
                                let prompt = c
                                    .prompt
                                    .clone()
                                    .filter(|s| !s.is_empty())
                                    .unwrap_or_else(|| {
                                        format!("Use {} passive ability?", symbols.name(card.name))
                                    });
                                symbols.intern(&prompt)
                            })
                            .collect()
                    })
            })
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
        Ok(Self {
            configs,
            prompts,
            triggers,
            gun: symbols.intern("big_sodding_gun"),
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
        let Some(h) = hero.or(actor).and_then(|id| world.hero(id)) else {
            return Ok(Vec::new());
        };
        if !self.triggers.contains(&trigger) {
            return Err("unknown passive trigger".into());
        }
        if world.has_effects {
            return Err("effect-backed passive providers not compiled".into());
        }
        let mut offers = Vec::new();
        // Do not deduplicate played/ultimate aliases: the reference only deduplicates
        // the later active-effect provider scan against these two ordinary sources.
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
            if c.facedown {
                continue;
            }
            let d = &world.definitions.cards[c.definition];
            if let Some((_, configs)) = self.configs.iter().find(|(e, _)| *e == d.effect) {
                for (index, config) in configs.iter().enumerate() {
                    if config.trigger != trigger
                        || (config.limit > 0 && c.passive_uses >= config.limit)
                    {
                        continue;
                    }
                    if d.effect != self.gun {
                        return Err("eligible passive execution not compiled".into());
                    }
                    offers.push(Offer {
                        card: d.printed_id,
                        trigger,
                        optional: config.optional,
                        prompt: self.prompts[c.definition][index],
                        hero: hero.map(|_| h.id),
                    });
                }
            }
        }
        Ok(offers)
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
    pub fn body(
        &self,
        world: &NativeWorld,
        ctx: &Registers,
        symbols: &Symbols,
        owner: Symbol,
        card: CardId,
        trigger: Symbol,
    ) -> Result<bool, String> {
        let c = world.card(card).ok_or("passive card missing")?;
        if c.facedown {
            return Ok(false);
        }
        let d = &world.definitions.cards[c.definition];
        if !d.has_effect {
            return Ok(false);
        }
        if d.effect != self.gun {
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
    ) -> Result<(), String> {
        if let Some(card) = actor.and_then(|a| Self::card(world, a, printed)) {
            let c = world.card_mut(card).ok_or("passive card missing")?;
            c.passive_uses = c
                .passive_uses
                .checked_add(1)
                .ok_or("passive usage overflow")?;
        }
        Ok(())
    }
}
