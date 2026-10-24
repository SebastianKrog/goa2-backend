//! Native card aura hooks. Unknown hooks remain explicit unsupported closures.
use crate::{
    cards::{CardId, HeroRecord},
    data::Symbol,
    stats::Stat,
    world::{CardLifecycle, NativeWorld},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardAura {
    None,
    Metamorphosis,
    UnlimitedFirepower,
    BigSoddingGun,
    Uncompiled,
}
impl CardAura {
    pub fn compile(id: &str, has_hook: bool) -> Self {
        if !has_hook {
            return Self::None;
        }
        match id {
            "metamorphosis" => Self::Metamorphosis,
            "unlimited_firepower" => Self::UnlimitedFirepower,
            "big_sodding_gun" => Self::BigSoddingGun,
            _ => Self::Uncompiled,
        }
    }
}
impl NativeWorld {
    pub(crate) fn active_aura_cards<'a>(
        &'a self,
        hero: &'a HeroRecord,
    ) -> impl Iterator<Item = CardId> + 'a {
        hero.ultimate
            .iter()
            .copied()
            .filter(|_| hero.level >= 8)
            .chain(hero.played.iter().flatten().copied().filter(|id| {
                self.card(*id)
                    .is_some_and(|c| c.lifecycle == CardLifecycle::Resolved)
            }))
            .filter(|id| self.card(*id).is_some_and(|c| !c.facedown))
    }
    pub(crate) fn card_aura_bonus(
        &self,
        unit: Symbol,
        stat: Stat,
        performing: Option<CardId>,
    ) -> Result<i64, String> {
        let Some(hero) = self.hero(unit) else {
            return Ok(0);
        };
        // Initiative belongs to the shared turn card even when queried via a piece.
        let unit = if stat == Stat::Initiative {
            hero.id
        } else {
            unit
        };
        let aggregate = unit == hero.id
            && self.entity(unit).is_some_and(|e| e.multi_piece)
            && (stat == Stat::Initiative || self.resolved_actor(unit) == unit);
        let actor = self.resolved_actor(unit);
        let adjacent_enemies = || {
            self.locations
                .iter()
                .filter(|(target, hex)| {
                    let Some(team) = self.entity(*target).and_then(|e| e.team) else {
                        return false;
                    };
                    if team == hero.team {
                        return false;
                    }
                    if aggregate {
                        self.piece_ids(hero.id).any(|p| {
                            self.position(p)
                                .is_some_and(|from| from.distance(*hex) <= 1)
                        })
                    } else {
                        self.position(actor)
                            .is_some_and(|from| from.distance(*hex) <= 1)
                    }
                })
                .count()
        };
        let mut total = 0i64;
        for id in self.active_aura_cards(hero) {
            let aura = self.definitions.cards[self.card(id).unwrap().definition].aura;
            let value = match aura {
                CardAura::UnlimitedFirepower if matches!(stat, Stat::Range | Stat::Radius) => 1,
                CardAura::Metamorphosis if matches!(stat, Stat::Movement | Stat::Initiative) => {
                    i64::try_from(adjacent_enemies()).map_err(|_| "aura count exceeds i64")?
                }
                CardAura::BigSoddingGun if matches!(stat, Stat::Attack | Stat::Range) => {
                    if let Some(c) = performing.and_then(|id| self.card(id)) {
                        let d = &self.definitions.cards[c.definition];
                        if d.spell {
                            return Err("conditional spell-face aura stats not compiled".into());
                        }
                        if d.basic && d.primary == Some(crate::world::ActionKind::Attack) {
                            2
                        } else {
                            0
                        }
                    } else {
                        0
                    }
                }
                CardAura::Uncompiled => return Err("card stat aura closure not compiled".into()),
                _ => 0,
            };
            total = total.checked_add(value).ok_or("aura bonus exceeds i64")?;
        }
        Ok(total)
    }
    pub(crate) fn movement_aura_passes(&self, unit: Symbol) -> bool {
        self.hero(unit).is_some_and(|hero| {
            self.active_aura_cards(hero).any(|id| {
                self.definitions.cards[self.card(id).unwrap().definition].aura
                    == CardAura::Metamorphosis
            })
        })
    }
}
