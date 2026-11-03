//! Round cleanup, mandatory leveling and atomic ordered upgrade decisions.
use crate::{
    cards::{CardError, CardId, reference},
    codec::StateEnvelope,
    data::{Datum, Symbol, Symbols},
    geometry::Hex,
    stats::Stat,
    world::{CardLifecycle, NativeWorld},
};
use serde_json::{Value, json};
type Result<T> = std::result::Result<T, CardError>;
#[derive(Clone, Debug)]
pub struct RoundState {
    pub round: i64,
    pub pending: Vec<(Symbol, i64)>,
    pub reveal: Vec<(Symbol, Vec<(Stat, i64)>)>,
    pool: Vec<(Symbol, bool)>,
}
#[derive(Clone, Debug)]
pub struct UpgradeOption {
    pub color: Symbol,
    pub tier: Symbol,
    pub pair: Vec<CardId>,
}
#[derive(Clone, Debug)]
pub struct UpgradePlayer {
    pub hero: Symbol,
    pub remaining: i64,
    pub options: Vec<UpgradeOption>,
}
#[derive(Clone, Debug)]
pub struct UpgradeChoice {
    pub hero: Symbol,
    pub cards: Vec<Symbol>,
}
#[derive(Clone, Debug)]
pub enum RoundEvent {
    TokenRemoved {
        token: Symbol,
        from: Hex,
        effects_removed: usize,
    },
    DeckSwapped {
        hero: Symbol,
        card: Symbol,
    },
    Retrieved {
        hero: Symbol,
        card: Symbol,
    },
}
impl RoundEvent {
    pub fn to_json(&self, s: &Symbols) -> Value {
        match self {
            Self::TokenRemoved {
                token,
                from,
                effects_removed,
            } => {
                json!({"event_type":"TOKEN_REMOVED","actor_id":null,"target_id":s.name(*token),"from_hex":from,"to_hex":null,"metadata":{"effects_removed":effects_removed}})
            }
            Self::DeckSwapped { hero, card } => {
                json!({"event_type":"DECK_CARD_SWAPPED","actor_id":s.name(*hero),"target_id":null,"from_hex":null,"to_hex":null,"metadata":{"outgoing_card_id":s.name(*card),"incoming_card_id":null,"source":"ready_for_war"}})
            }
            Self::Retrieved { hero, card } => {
                json!({"event_type":"CARD_RETRIEVED","actor_id":s.name(*hero),"target_id":null,"from_hex":null,"to_hex":null,"metadata":{"card_id":s.name(*card),"source":"ready_for_war"}})
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct RoundRules {
    colors: [Symbol; 3],
    tiers: [Symbol; 3],
    silver: Symbol,
    gold: Symbol,
    unbound: Symbol,
    war: Symbol,
    hooks: Vec<Symbol>,
    keys: [Symbol; 3],
    pub simultaneous: Symbol,
    pub prompt: Symbol,
}
impl RoundRules {
    pub fn compile(s: &mut Symbols) -> std::result::Result<Self, String> {
        let hooks = crate::templates::instructions()?["ultimate_unlock_effects"]
            .as_array()
            .ok_or("unlock catalog missing")?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(|id| s.intern(id))
                    .ok_or("invalid unlock effect".to_string())
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Self {
            colors: ["RED", "BLUE", "GREEN"].map(|v| s.intern(v)),
            tiers: ["I", "II", "III"].map(|v| s.intern(v)),
            silver: s.intern("SILVER"),
            gold: s.intern("GOLD"),
            unbound: s.intern("unbound_fury"),
            war: s.intern("ready_for_war"),
            hooks,
            keys: ["hero_id", "card_id", "card_ids"].map(|v| s.intern(v)),
            simultaneous: s.intern("simultaneous"),
            prompt: s.intern("Mandatory Upgrade Phase"),
        })
    }
    pub(crate) fn has_unlock_hook(&self, effect: Symbol) -> bool {
        self.hooks.contains(&effect)
    }
    pub fn options(&self, world: &NativeWorld, hero: Symbol) -> Vec<UpgradeOption> {
        let Some(h) = world.hero(hero) else {
            return vec![];
        };
        let cards = h
            .hand
            .iter()
            .filter_map(|id| world.card(*id))
            .map(|c| &world.definitions.cards[c.definition])
            .filter(|c| c.color.is_some_and(|v| self.colors.contains(&v)))
            .collect::<Vec<_>>();
        let tier = cards
            .iter()
            .map(|c| {
                self.tiers
                    .iter()
                    .position(|t| *t == c.tier)
                    .map_or(99, |i| i + 1)
            })
            .min()
            .unwrap_or(99);
        if !(1..=2).contains(&tier) {
            return vec![];
        }
        cards
            .into_iter()
            .filter(|c| c.tier == self.tiers[tier - 1])
            .filter_map(|c| {
                let color = c.color.unwrap();
                let target = self.tiers[tier];
                let pair = h
                    .deck
                    .iter()
                    .copied()
                    .filter(|id| {
                        world.card(*id).is_some_and(|c| {
                            let d = &world.definitions.cards[c.definition];
                            d.color == Some(color)
                                && d.tier == target
                                && c.lifecycle == CardLifecycle::Deck
                        })
                    })
                    .collect::<Vec<_>>();
                (pair.len() == 2).then_some(UpgradeOption {
                    color,
                    tier: target,
                    pair,
                })
            })
            .collect()
    }
    fn legal(
        &self,
        world: &NativeWorld,
        state: &RoundState,
        hero: Symbol,
        printed: Symbol,
    ) -> bool {
        state.pending.iter().any(|(h, _)| *h == hero)
            && self.options(world, hero).iter().any(|option| {
                option.pair.iter().any(|id| {
                    world.card(*id).is_some_and(|c| {
                        world.definitions.cards[c.definition].printed_id == printed
                    })
                })
            })
    }
    pub fn validate(
        &self,
        world: &NativeWorld,
        state: &RoundState,
        value: &Datum,
    ) -> Result<Option<UpgradeChoice>> {
        let Datum::Object(fields) = value else {
            return Ok(None);
        };
        let get = |key| fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v);
        let Some(Datum::Text(hero)) = get(self.keys[0]) else {
            return Ok(None);
        };
        let Some((_, remaining)) = state
            .pending
            .iter()
            .find(|(h, _)| h == hero)
            .filter(|(_, n)| *n > 0)
        else {
            return Ok(None);
        };
        let cards = if let Some(value) = get(self.keys[2]) {
            if get(self.keys[1]).is_some() {
                return Ok(None);
            };
            let Datum::List(ids) = value else {
                return Ok(None);
            };
            let Some(ids) = ids
                .iter()
                .map(|id| {
                    if let Datum::Text(id) = id {
                        Some(*id)
                    } else {
                        None
                    }
                })
                .collect::<Option<Vec<_>>>()
            else {
                return Ok(None);
            };
            ids
        } else if let Some(Datum::Text(id)) = get(self.keys[1]) {
            vec![*id]
        } else {
            return Ok(None);
        };
        if (cards.len() != 1 && cards.len() as i64 != *remaining)
            || cards
                .iter()
                .enumerate()
                .any(|(i, id)| cards[..i].contains(id))
        {
            return Ok(None);
        };
        if cards.len() == 1 {
            return Ok(self
                .legal(world, state, *hero, cards[0])
                .then_some(UpgradeChoice { hero: *hero, cards }));
        }
        let mut preview = world.clone();
        let mut pending = state.clone();
        for id in &cards {
            if !self.legal(&preview, &pending, *hero, *id) {
                return Ok(None);
            };
            self.apply(&mut preview, &mut pending, *hero, *id)?;
        }
        Ok(Some(UpgradeChoice { hero: *hero, cards }))
    }
    pub fn players(&self, world: &NativeWorld, state: &RoundState) -> Vec<UpgradePlayer> {
        state
            .pending
            .iter()
            .map(|(hero, n)| UpgradePlayer {
                hero: *hero,
                remaining: *n,
                options: self.options(world, *hero),
            })
            .collect()
    }
    pub fn apply(
        &self,
        world: &mut NativeWorld,
        state: &mut RoundState,
        hero: Symbol,
        printed: Symbol,
    ) -> Result<()> {
        let Some(h) = world.hero_index(hero) else {
            return Ok(());
        };
        world.lifecycle_hero(hero)?;
        let Some(chosen) = world.heroes[h].deck.iter().copied().find(|id| {
            world
                .card(*id)
                .is_some_and(|c| world.definitions.cards[c.definition].printed_id == printed)
        }) else {
            return Ok(());
        };
        let d = &world.definitions.cards[world.card(chosen).unwrap().definition];
        let color = d.color;
        let tier = d.tier;
        let prev = world.heroes[h].hand.iter().copied().find(|id| {
            world
                .card(*id)
                .is_some_and(|c| world.definitions.cards[c.definition].color == color)
        });
        let pair = world.heroes[h].deck.iter().copied().find(|id| {
            world.card(*id).is_some_and(|c| {
                let d = &world.definitions.cards[c.definition];
                d.color == color && d.tier == tier && d.printed_id != printed
            })
        });
        if let Some(prev) = prev {
            let index = world.heroes[h]
                .hand
                .iter()
                .position(|id| *id == prev)
                .unwrap();
            world.heroes[h].hand.remove(index);
            world.change_lifecycle(prev, CardLifecycle::Retired);
        }
        world.change_lifecycle(chosen, CardLifecycle::Hand);
        world.card_mut(chosen).unwrap().facedown = false;
        world.heroes[h].hand.push(chosen);
        if let Some(pair) = pair {
            let stat = world.definitions.cards[world.card(pair).unwrap().definition].item;
            if let Some(stat) = stat {
                world.heroes[h].items[stat.index()] = world.heroes[h].items[stat.index()]
                    .checked_add(1)
                    .ok_or(CardError::Unsupported("upgrade item overflow"))?;
                if !world.heroes[h].items_order.contains(&stat) {
                    world.heroes[h].items_order.push(stat);
                }
            }
            world.change_lifecycle(pair, CardLifecycle::Item);
        }
        if let Some(index) = state.pending.iter().position(|(h, _)| *h == hero) {
            state.pending[index].1 -= 1;
            if state.pending[index].1 <= 0 {
                state.pending.remove(index);
            }
        }
        world.refresh_card_auras();
        Ok(())
    }
    pub fn cleanup(
        &self,
        world: &mut NativeWorld,
        state: &mut RoundState,
    ) -> Result<(bool, Vec<RoundEvent>)> {
        // The cleanup source intentionally discards returned finishing payloads.
        // Their scheduled execution belongs to EndPhase, not this second sweep.
        world.expire_duration_effects(crate::effects::Duration::ThisRound);
        for marker in &mut world.markers {
            marker.target = None;
            marker.source = None;
            marker.value = 0;
        }
        world.cleanup_stale_effects();
        for h in 0..world.heroes.len() {
            let expiring = world.heroes[h]
                .played
                .iter()
                .flatten()
                .copied()
                .chain(world.heroes[h].current)
                .collect::<Vec<_>>();
            for id in expiring {
                let printed =
                    world.definitions.cards[world.card(id).unwrap().definition].printed_id;
                world.expire_effects_by_card(printed);
            }
            world.retrieve_cards(world.heroes[h].id)?;
        }
        let mut events = Vec::new();
        for (token, persist) in &state.pool {
            if !persist
                && let Some(from) = world
                    .remove_entity(*token)
                    .map_err(|_| CardError::Illegal("token removal failed"))?
            {
                let effects_removed = world.remove_token_effects(*token);
                events.push(RoundEvent::TokenRemoved {
                    token: *token,
                    from,
                    effects_removed,
                });
            }
        }
        let mut leveled = false;
        for h in 0..world.heroes.len() {
            let mut pending = 0;
            let mut ultimate = false;
            while world.heroes[h].level < 8 {
                if world.heroes[h].level < 1 {
                    return Err(CardError::Unsupported(
                        "invalid hero level for round cleanup",
                    ));
                };
                let cost = world.heroes[h].level;
                if world.heroes[h].gold < cost {
                    break;
                }
                world.heroes[h].gold -= cost;
                world.heroes[h].level += 1;
                leveled = true;
                if world.heroes[h].level == 8 {
                    ultimate = true;
                    self.unlock(world, h, &mut events)?;
                } else {
                    pending += 1;
                }
            }
            let hero = world.heroes[h].id;
            if pending > 0 {
                upsert(&mut state.pending, hero, pending);
                let items = world.heroes[h]
                    .items_order
                    .iter()
                    .map(|s| (*s, world.heroes[h].items[s.index()]))
                    .collect();
                upsert(&mut state.reveal, hero, items);
            } else if !ultimate {
                world.heroes[h].gold = world.heroes[h]
                    .gold
                    .checked_add(1)
                    .ok_or(CardError::Unsupported("pity gold overflow"))?;
            }
        }
        world.refresh_card_auras();
        Ok((leveled, events))
    }
    fn unlock(
        &self,
        world: &mut NativeWorld,
        h: usize,
        events: &mut Vec<RoundEvent>,
    ) -> Result<()> {
        let Some(ult) = world.heroes[h].ultimate.and_then(|id| world.card(id)) else {
            return Ok(());
        };
        let effect = world.definitions.cards[ult.definition].effect;
        if !self.hooks.contains(&effect) {
            return Ok(());
        };
        let hero = world.heroes[h].id;
        if effect == self.unbound {
            let cards = world.heroes[h]
                .deck
                .iter()
                .chain(&world.heroes[h].hand)
                .chain(world.heroes[h].played.iter().flatten())
                .chain(&world.heroes[h].discard)
                .chain(world.heroes[h].ultimate.iter())
                .copied()
                .collect::<Vec<_>>();
            for id in cards {
                world.card_mut(id).unwrap().enraged = true;
            }
        } else if effect == self.war {
            let Some(silver) = world.heroes[h].deck.iter().copied().find(|id| {
                world.card(*id).is_some_and(|c| {
                    world.definitions.cards[c.definition].color == Some(self.silver)
                })
            }) else {
                return Ok(());
            };
            if world.card(silver).unwrap().lifecycle == CardLifecycle::Deck {
                return Ok(());
            };
            if let Some(i) = world.heroes[h].hand.iter().position(|id| *id == silver) {
                world.heroes[h].hand.remove(i);
            }
            if let Some(i) = world.heroes[h].discard.iter().position(|id| *id == silver) {
                world.heroes[h].discard.remove(i);
            }
            let printed =
                world.definitions.cards[world.card(silver).unwrap().definition].printed_id;
            for slot in &mut world.heroes[h].played {
                if *slot == Some(silver) {
                    *slot = None;
                }
            }
            if world.heroes[h].current == Some(silver) {
                world.heroes[h].current = None;
            }
            world.change_lifecycle(silver, CardLifecycle::Deck);
            let card = world.card_mut(silver).unwrap();
            card.facedown = false;
            card.played_this_round = false;
            card.active = false;
            events.push(RoundEvent::DeckSwapped {
                hero,
                card: printed,
            });
            let golds = world.heroes[h]
                .deck
                .iter()
                .copied()
                .filter(|id| {
                    world.card(*id).is_some_and(|c| {
                        c.lifecycle == CardLifecycle::Deck
                            && world.definitions.cards[c.definition].color == Some(self.gold)
                    })
                })
                .collect::<Vec<_>>();
            for id in golds {
                world.return_card_to_hand(hero, id)?;
                events.push(RoundEvent::Retrieved {
                    hero,
                    card: world.definitions.cards[world.card(id).unwrap().definition].printed_id,
                });
            }
        } else {
            return Err(CardError::Unsupported("ultimate unlock hook not compiled"));
        };
        Ok(())
    }
}
fn upsert<T>(values: &mut Vec<(Symbol, T)>, key: Symbol, value: T) {
    if let Some((_, old)) = values.iter_mut().find(|(k, _)| *k == key) {
        *old = value
    } else {
        values.push((key, value));
    }
}
impl RoundState {
    pub fn compile(source: &StateEnvelope, s: &mut Symbols) -> std::result::Result<Self, String> {
        let state = &source.state;
        let pending = state["pending_upgrades"]
            .as_object()
            .ok_or("pending upgrades missing")?
            .iter()
            .map(|(h, n)| Ok((s.intern(h), n.as_i64().ok_or("invalid upgrade count")?)))
            .collect::<std::result::Result<Vec<_>, String>>()?;
        let reveal = state["upgrade_reveal_snapshot"]
            .as_object()
            .ok_or("upgrade snapshot missing")?
            .iter()
            .map(|(h, items)| {
                let items = items
                    .as_object()
                    .ok_or("invalid item snapshot")?
                    .iter()
                    .map(|(key, n)| {
                        Ok((
                            Stat::parse(key)?,
                            n.as_i64().ok_or("invalid snapshot value")?,
                        ))
                    })
                    .collect::<std::result::Result<Vec<_>, String>>()?;
                Ok((s.intern(h), items))
            })
            .collect::<std::result::Result<Vec<_>, String>>()?;
        let mut pool = Vec::new();
        for tokens in state["token_pool"]
            .as_object()
            .ok_or("token pool missing")?
            .values()
        {
            for token in tokens.as_array().ok_or("invalid token pool")? {
                let id = reference(token)?.ok_or("null pool token")?;
                let instance = source
                    .instances
                    .get(id.0 as usize)
                    .ok_or("pool instance missing")?;
                let definition = &source.definitions[instance.definition as usize];
                let name = s.intern(
                    definition.fields["id"]
                        .as_str()
                        .ok_or("pool token identity missing")?,
                );
                let persist = instance.fields["persists_end_of_round"]
                    .as_bool()
                    .ok_or("token persistence missing")?;
                pool.push((name, persist));
            }
        }
        Ok(Self {
            round: state["round"].as_i64().ok_or("round missing")?,
            pending,
            reveal,
            pool,
        })
    }
    pub fn reset(&mut self, world: &mut NativeWorld, clear_defeated: bool) -> Result<()> {
        self.round = self
            .round
            .checked_add(1)
            .filter(|n| *n <= i64::from(u32::MAX))
            .ok_or(CardError::Unsupported("round counter outside u32"))?;
        world.combat.round = self.round;
        world.combat.turn = 1;
        if clear_defeated {
            world.combat.defeated.clear();
        }
        Ok(())
    }
    pub fn export(&self, state: &mut Value, s: &Symbols) {
        state["round"] = json!(self.round);
        state["pending_upgrades"] = Value::Object(
            self.pending
                .iter()
                .map(|(h, n)| (s.name(*h).into(), json!(n)))
                .collect(),
        );
        state["upgrade_reveal_snapshot"] = Value::Object(
            self.reveal
                .iter()
                .map(|(h, items)| {
                    (
                        s.name(*h).into(),
                        Value::Object(
                            items
                                .iter()
                                .map(|(stat, n)| (stat.name().into(), json!(n)))
                                .collect(),
                        ),
                    )
                })
                .collect(),
        );
    }
}
