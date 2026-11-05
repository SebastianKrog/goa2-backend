//! Static loadout hypotheses conditioned only on player-visible facts.
use super::{EngineError, PlayerObservation, PublicCard, Result};
use crate::stats::Stat;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct LoadoutHypothesis {
    pub active_card_ids: Vec<String>,
    pub item_card_ids: Vec<String>,
}

/// Cumulative ordinary card-item totals after a publicly observed upgrade
/// batch. Callers remove independently attributable item gains before recording.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UpgradeCheckpoint {
    pub level: i64,
    pub card_items: [i64; 6],
}

/// One hero's public evidence from a completed upgrade batch.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublicUpgradeBatch {
    pub revision: u64,
    pub hero_id: String,
    pub before: UpgradeCheckpoint,
    pub after: UpgradeCheckpoint,
}

/// Replay public batches into the checkpoints used by the loadout sampler.
/// A revision can contain several heroes, and gaps between batches are allowed.
/// Reconnected clients can merge overlapping histories in any arrival order.
pub fn replay_public_upgrade_batches(
    batches: &[PublicUpgradeBatch],
) -> Result<BTreeMap<String, Vec<UpgradeCheckpoint>>> {
    let invalid = |message: &str| EngineError::InvalidInput(message.into());
    let mut checkpoints = BTreeMap::<String, Vec<UpgradeCheckpoint>>::new();
    let mut ordered = BTreeMap::<(u64, &str), &PublicUpgradeBatch>::new();
    for batch in batches {
        if let Some(previous) = ordered.insert((batch.revision, &batch.hero_id), batch)
            && previous != batch
        {
            return Err(invalid("conflicting public upgrade batch"));
        }
    }
    for batch in ordered.into_values() {
        if batch.hero_id.is_empty()
            || !(1..batch.after.level).contains(&batch.before.level)
            || batch.after.level > 8
            || batch
                .before
                .card_items
                .iter()
                .zip(batch.after.card_items)
                .any(|(before, after)| *before < 0 || after < *before)
        {
            return Err(invalid("invalid public upgrade batch"));
        }
        let rows = checkpoints.entry(batch.hero_id.clone()).or_default();
        if let Some(previous) = rows.last()
            && (batch.before.level < previous.level
                || (batch.before.level == previous.level && batch.before != *previous)
                || previous
                    .card_items
                    .iter()
                    .zip(batch.before.card_items)
                    .any(|(old, new)| new < *old))
        {
            return Err(invalid("inconsistent public upgrade history"));
        }
        if rows.last() != Some(&batch.before) {
            rows.push(batch.before);
        }
        rows.push(batch.after);
    }
    Ok(checkpoints)
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
        self.sample_planning_hand_with_checkpoints(hero_id, seed, &[])
    }

    pub fn sample_planning_hand_with_checkpoints(
        &self,
        hero_id: &str,
        seed: u64,
        checkpoints: &[UpgradeCheckpoint],
    ) -> Result<Option<SampledHand>> {
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
            .loadout_hypotheses_with_checkpoints(hero_id, checkpoints)?
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
        // The reconstructed pool still includes unrevealed commitments.
        // Only the count is public and can rule out an impossible pool.
        if cards.len() < hero.hand_size {
            return Ok(None);
        }
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
    /// nonstandard provenance returns None. Use only publicly visible card zones.
    pub fn loadout_hypotheses(&self, hero_id: &str) -> Result<Option<Vec<LoadoutHypothesis>>> {
        self.loadout_hypotheses_with_checkpoints(hero_id, &[])
    }

    pub fn loadout_hypotheses_with_checkpoints(
        &self,
        hero_id: &str,
        checkpoints: &[UpgradeCheckpoint],
    ) -> Result<Option<Vec<LoadoutHypothesis>>> {
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
        if checkpoints.iter().any(|point| {
            !(1..=hero.level).contains(&point.level)
                || point.card_items.iter().any(|count| *count < 0)
        }) || checkpoints
            .windows(2)
            .any(|pair| pair[0].level > pair[1].level)
        {
            return Err(EngineError::InvalidInput(
                "invalid public upgrade checkpoints".into(),
            ));
        }
        let Some(mut candidates) = enumerate(definition, hero.level, &reveals, checkpoints) else {
            return Ok(None);
        };
        let visible = hero
            .played
            .iter()
            .chain(&hero.discarded)
            .filter_map(|card| match card {
                PublicCard::Revealed(id) => Some(id),
                _ => None,
            })
            .chain(&hero.revealed_current)
            .filter_map(|id| definition.cards.iter().find(|card| card.id == *id))
            .collect::<Vec<_>>();
        candidates.retain(|row| {
            visible.iter().all(|card| {
                if card.tier == 1 {
                    !definition.cards.iter().any(|upgrade| {
                        upgrade.color.index() == card.color.index()
                            && row.active_card_ids.contains(&upgrade.id)
                    })
                } else {
                    row.active_card_ids.contains(&card.id)
                }
            })
        });
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

fn enumerate(
    hero: &StaticHero,
    level: i64,
    reveals: &[&str],
    checkpoints: &[UpgradeCheckpoint],
) -> Option<Vec<LoadoutHypothesis>> {
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
    let matches_checkpoints = |upgrades, items: &[usize]| {
        checkpoints
            .iter()
            .filter(|point| (point.level - 1).min(6) == upgrades)
            .all(|point| {
                let mut counts = [0; 6];
                for &index in items {
                    if let Some(stat) = hero.cards[index].item {
                        counts[stat.index()] += 1;
                    }
                }
                counts == point.card_items
            })
    };
    states.retain(|(_, items)| matches_checkpoints(0, items));
    if states.is_empty() {
        return Some(Vec::new());
    }
    for upgrades in 1..=(level - 1).min(6) {
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
        next.retain(|(_, items)| matches_checkpoints(upgrades, items));
        if next.is_empty() {
            return Some(Vec::new());
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
    fn visible_discard_constrains_loadout_without_reveal_history() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let session = super::super::NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        )
        .unwrap();
        let mut observation = session
            .player_observation(session.world.heroes[0].id)
            .unwrap();
        observation.revealed_cards.clear();
        let hero = &mut observation.heroes[1];
        hero.name = "Arien".into();
        hero.level = 2;
        hero.items = [0; 6];
        hero.items[Stat::Attack.index()] = 1;
        hero.played.clear();
        hero.discarded.clear();
        hero.revealed_current.clear();
        let id = hero.id.clone();
        assert_eq!(
            observation.loadout_hypotheses(&id).unwrap().unwrap().len(),
            2
        );
        observation.heroes[1].hand_size -= 1;
        observation.heroes[1]
            .discarded
            .push(PublicCard::Revealed("expert_duelist".into()));
        let rows = observation.loadout_hypotheses(&id).unwrap().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].active_card_ids, ["expert_duelist"]);
        let sample = observation.sample_planning_hand(&id, 39).unwrap().unwrap();
        assert!(!sample.cards.iter().any(|card| card == "expert_duelist"));
        observation.heroes[1].hand_size = sample.cards.len() + 1;
        assert!(observation.sample_planning_hand(&id, 39).unwrap().is_none());
    }

    #[test]
    fn public_batches_roundtrip_and_replay_without_intermediate_levels() {
        let initial = UpgradeCheckpoint {
            level: 1,
            card_items: [0; 6],
        };
        let fourth = UpgradeCheckpoint {
            level: 4,
            card_items: [1, 1, 0, 1, 0, 0],
        };
        let sixth = UpgradeCheckpoint {
            level: 6,
            card_items: [2, 1, 0, 2, 0, 0],
        };
        let first = PublicUpgradeBatch {
            revision: 5,
            hero_id: "hero_arien".into(),
            before: initial,
            after: fourth,
        };
        let later = PublicUpgradeBatch {
            revision: 9,
            hero_id: first.hero_id.clone(),
            before: fourth,
            after: sixth,
        };
        let encoded =
            serde_json::to_string(&[later.clone(), first.clone(), later.clone()]).unwrap();
        let replayed: Vec<PublicUpgradeBatch> = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            replay_public_upgrade_batches(&replayed).unwrap()["hero_arien"],
            [initial, fourth, sixth]
        );
        let mut conflict = first.clone();
        conflict.after = sixth;
        assert!(replay_public_upgrade_batches(&[first.clone(), later.clone(), conflict]).is_err());

        let mut wrong_revision = first;
        wrong_revision.revision = 10;
        assert!(replay_public_upgrade_batches(&[wrong_revision, later]).is_err());
    }

    #[test]
    fn upgrade_checkpoints_preserve_batch_ambiguity_and_filter_later_gains() {
        let hero = &catalog().unwrap().heroes["Arien"];
        let items = |attack| {
            let mut counts = [0; 6];
            counts[Stat::Attack.index()] = attack;
            counts[Stat::Defense.index()] = 1;
            counts[Stat::Initiative.index()] = 1;
            counts
        };
        let level_four = UpgradeCheckpoint {
            level: 4,
            card_items: items(1),
        };
        let level_five = UpgradeCheckpoint {
            level: 5,
            card_items: items(2),
        };
        // Level one to four supplies only a combined three-upgrade total.
        assert_eq!(enumerate(hero, 4, &[], &[level_four]).unwrap().len(), 2);
        let aggregate = enumerate(hero, 5, &[], &[level_five]).unwrap();
        assert_eq!(aggregate.len(), 4);
        let observed = enumerate(hero, 5, &[], &[level_four, level_five]).unwrap();
        assert!(!observed.is_empty() && observed.len() < aggregate.len());
        assert!(
            observed
                .iter()
                .all(|row| row.active_card_ids.iter().any(|id| id == "master_duelist"))
        );
        assert!(
            enumerate(
                hero,
                5,
                &[],
                &[UpgradeCheckpoint {
                    level: 4,
                    card_items: [0; 6]
                }]
            )
            .unwrap()
            .is_empty()
        );
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
            let mut result = enumerate(hero, case.level, &reveals, &[]).unwrap();
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
