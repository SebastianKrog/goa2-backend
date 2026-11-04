//! Static loadout hypotheses conditioned only on player-visible facts.
use super::{EngineError, PlayerObservation, PublicCard, Result};
use crate::stats::Stat;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct LoadoutHypothesis {
    pub active_card_ids: Vec<String>,
    pub item_card_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "UPPERCASE")]
enum Color {
    Red,
    Blue,
    Green,
}
impl Color {
    fn index(&self) -> usize {
        match self {
            Self::Red => 0,
            Self::Blue => 1,
            Self::Green => 2,
        }
    }
}

#[derive(Deserialize)]
struct StaticCard {
    id: String,
    color: Color,
    tier: u8,
    item: Option<Stat>,
    starts_in_deck: bool,
}

#[derive(Deserialize)]
struct StaticHero {
    standard: bool,
    starting: Vec<String>,
    cards: Vec<StaticCard>,
}

#[derive(Deserialize)]
struct Catalog {
    source_sha256: String,
    heroes: BTreeMap<String, StaticHero>,
}

fn catalog() -> Result<&'static Catalog> {
    static CATALOG: std::sync::OnceLock<std::result::Result<Catalog, String>> =
        std::sync::OnceLock::new();
    CATALOG
        .get_or_init(|| {
            let catalog: Catalog = serde_json::from_str(include_str!(
                "../../../../tests/fixtures/loadout-provenance.json"
            ))
            .map_err(|error| error.to_string())?;
            if crate::codec::oracle_manifest()?["source_sha256"].as_str()
                != Some(&catalog.source_sha256)
            {
                return Err("loadout provenance oracle mismatch".into());
            }
            Ok(catalog)
        })
        .as_ref()
        .map_err(|reason| EngineError::Unsupported(reason.clone()))
}

#[derive(Debug, PartialEq, Eq)]
pub struct SampledHand {
    pub hero: String,
    pub cards: Vec<String>,
    pub loadout: LoadoutHypothesis,
    pub(crate) retired: Vec<String>,
}

fn sample_index(seed: u64, len: usize) -> usize {
    // SplitMix64 mixes sequential sample seeds before indexing the sorted hypotheses.
    let mut value = seed.wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((value ^ (value >> 31)) % len as u64) as usize
}

impl PlayerObservation {
    /// Sample one other hero's ordinary planning hand before hidden commitments.
    /// Returns None when public card slots or a special lifecycle leave the
    /// ordinary hand reconstruction unsupported.
    pub fn sample_planning_hand(&self, hero_id: &str, seed: u64) -> Result<Option<SampledHand>> {
        if self.phase != crate::codec::Phase::Planning || hero_id == self.private.hero {
            return Ok(None);
        }
        let Some(hero) = self.heroes.iter().find(|hero| hero.id == hero_id) else {
            return Err(EngineError::InvalidInput("unknown observed hero".into()));
        };
        let zones = hero.played.iter().chain(&hero.discarded);
        if zones.clone().any(|card| matches!(card, PublicCard::Hidden)) {
            return Ok(None);
        }
        let Some(definition) = catalog()?.heroes.get(&hero.name) else {
            return Ok(None);
        };
        let Some(loadouts) = self
            .loadout_hypotheses(hero_id)?
            .filter(|rows| !rows.is_empty())
        else {
            return Ok(None);
        };
        let unavailable = zones
            .filter_map(|card| match card {
                PublicCard::Revealed(id) => Some(id),
                _ => None,
            })
            .chain(&hero.revealed_current)
            .collect::<BTreeSet<_>>();
        let selected = sample_index(seed, loadouts.len());
        let loadout = loadouts.into_iter().nth(selected).unwrap();
        let upgraded = definition
            .cards
            .iter()
            .filter(|card| loadout.active_card_ids.contains(&card.id))
            .map(|card| card.color.index())
            .collect::<BTreeSet<_>>();
        let mut cards = definition
            .starting
            .iter()
            .filter(|id| {
                !definition
                    .cards
                    .iter()
                    .any(|card| card.id == **id && upgraded.contains(&card.color.index()))
            })
            .chain(&loadout.active_card_ids)
            .filter(|id| !unavailable.contains(id))
            .cloned()
            .collect::<Vec<_>>();
        cards.sort();
        let retired = definition
            .cards
            .iter()
            .filter(|card| {
                !loadout.item_card_ids.contains(&card.id)
                    && definition.cards.iter().any(|active| {
                        loadout.active_card_ids.contains(&active.id)
                            && active.color.index() == card.color.index()
                            && active.tier > card.tier
                    })
            })
            .map(|card| card.id.clone())
            .collect();
        Ok(Some(SampledHand {
            hero: hero_id.into(),
            cards,
            loadout,
            retired,
        }))
    }

    /// Candidate upgrade paths for a publicly identified hero. Unsupported or
    /// nonstandard provenance returns None, without consulting live card zones.
    pub fn loadout_hypotheses(&self, hero_id: &str) -> Result<Option<Vec<LoadoutHypothesis>>> {
        let Some(hero) = self.heroes.iter().find(|hero| hero.id == hero_id) else {
            return Err(EngineError::InvalidInput("unknown observed hero".into()));
        };
        let Some(definition) = catalog()?
            .heroes
            .get(&hero.name)
            .filter(|hero| hero.standard)
        else {
            return Ok(None);
        };
        let reveals = self
            .revealed_cards
            .iter()
            .filter(|(owner, _)| owner == hero_id)
            .map(|(_, card)| card.as_str())
            .collect::<Vec<_>>();
        let Some(candidates) = enumerate(definition, hero.level, &reveals) else {
            return Ok(None);
        };
        for extra in [false, true] {
            let filtered = candidates
                .iter()
                .filter(|row| items_match(row, definition, &hero.items, extra))
                .cloned()
                .collect::<Vec<_>>();
            if !filtered.is_empty() {
                return Ok(Some(filtered));
            }
        }
        Ok(Some(candidates))
    }
}

fn enumerate(hero: &StaticHero, level: i64, reveals: &[&str]) -> Option<Vec<LoadoutHypothesis>> {
    if !(1..=8).contains(&level) {
        return None;
    }
    let mut by_tier: [[Vec<usize>; 3]; 3] =
        std::array::from_fn(|_| std::array::from_fn(|_| Vec::new()));
    for (index, card) in hero.cards.iter().enumerate() {
        if card.tier != 1 || !card.starts_in_deck {
            by_tier[card.color.index()][usize::from(card.tier.checked_sub(1)?)].push(index);
        }
    }
    if by_tier.iter().any(|tiers| tiers[0].len() != 1) {
        return None;
    }
    let mut states = BTreeSet::from([(
        std::array::from_fn::<_, 3, _>(|color| by_tier[color][0][0]),
        Vec::new(),
    )]);
    for _ in 0..(level - 1).min(6) {
        let mut next = BTreeSet::new();
        for (active, items) in states {
            let minimum = active
                .iter()
                .map(|index| hero.cards[*index].tier)
                .min()
                .unwrap();
            for color in 0..3 {
                if hero.cards[active[color]].tier != minimum {
                    continue;
                }
                let Some([first, second]) =
                    by_tier[color].get(usize::from(minimum)).map(Vec::as_slice)
                else {
                    continue;
                };
                for (chosen, item) in [(*first, *second), (*second, *first)] {
                    let mut upgraded = active;
                    upgraded[color] = chosen;
                    let mut history = items.clone();
                    let index = history.binary_search(&item).unwrap_or_else(|index| index);
                    history.insert(index, item);
                    next.insert((upgraded, history));
                }
            }
        }
        if next.is_empty() {
            return None;
        }
        states = next;
    }

    let mut results = BTreeSet::new();
    for (active, items) in states {
        let consistent = reveals.iter().all(|id| {
            let Some((index, card)) = hero
                .cards
                .iter()
                .enumerate()
                .find(|(_, card)| card.id == *id)
            else {
                return true;
            };
            match card.tier {
                2 => by_tier[card.color.index()][1]
                    .iter()
                    .all(|paired| *paired == index || items.contains(paired)),
                3 => active.contains(&index),
                _ => true,
            }
        });
        if consistent {
            let mut active_card_ids = active
                .iter()
                .filter(|index| hero.cards[**index].tier > 1)
                .map(|index| hero.cards[*index].id.clone())
                .collect::<Vec<_>>();
            let mut item_card_ids = items
                .iter()
                .map(|index| hero.cards[*index].id.clone())
                .collect::<Vec<_>>();
            active_card_ids.sort();
            item_card_ids.sort();
            results.insert(LoadoutHypothesis {
                active_card_ids,
                item_card_ids,
            });
        }
    }
    Some(results.into_iter().collect())
}

fn items_match(row: &LoadoutHypothesis, hero: &StaticHero, public: &[i64; 6], extra: bool) -> bool {
    let mut counts = [0; 6];
    for card in hero
        .cards
        .iter()
        .filter(|card| row.item_card_ids.contains(&card.id))
    {
        if let Some(item) = card.item {
            counts[item.index()] += 1;
        }
    }
    if extra {
        counts
            .iter()
            .zip(public)
            .all(|(count, wanted)| *count == 0 || count <= wanted)
    } else {
        counts == *public
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Case {
        hero: String,
        level: i64,
        items: Option<BTreeMap<String, i64>>,
        reveals: Vec<String>,
        extra: bool,
        expected: Vec<LoadoutHypothesis>,
    }

    #[test]
    fn native_provenance_matches_frozen_python_hypotheses() {
        let raw: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/loadout-provenance.json"
        ))
        .unwrap();
        let cases: Vec<Case> = serde_json::from_value(raw["cases"].clone()).unwrap();
        assert_eq!(cases.len(), 20);
        for case in cases {
            let hero = &catalog().unwrap().heroes[&case.hero];
            let reveals = case.reveals.iter().map(String::as_str).collect::<Vec<_>>();
            let mut result = enumerate(hero, case.level, &reveals).unwrap();
            if let Some(items) = case.items {
                let mut public = [0; 6];
                for (name, value) in items {
                    public[Stat::parse(&name).unwrap().index()] = value;
                }
                result.retain(|row| items_match(row, hero, &public, case.extra));
            }
            assert_eq!(result, case.expected, "{} level {}", case.hero, case.level);
        }
    }
}
