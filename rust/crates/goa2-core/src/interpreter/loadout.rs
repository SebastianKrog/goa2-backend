//! Static loadout hypotheses conditioned only on player-visible facts.
use super::{EngineError, PlayerObservation, PublicCard, Result};
use crate::stats::Stat;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
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
    let initial = UpgradeCheckpoint {
        level: 1,
        card_items: [0; 6],
    };
    for batch in ordered.into_values() {
        if batch.hero_id.is_empty()
            || !(1..batch.after.level).contains(&batch.before.level)
            || batch.after.level > 8
            || !ordinary_item_gains_fit(initial, batch.before)
            || !ordinary_item_gains_fit(batch.before, batch.after)
        {
            return Err(invalid("invalid public upgrade batch"));
        }
        let rows = checkpoints.entry(batch.hero_id.clone()).or_default();
        if let Some(&previous) = rows.last()
            && (batch.before.level < previous.level
                || !ordinary_item_gains_fit(previous, batch.before))
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

fn ordinary_item_gains_fit(before: UpgradeCheckpoint, after: UpgradeCheckpoint) -> bool {
    // Level eight grants the ultimate, with no ordinary card-item reward.
    let allowance = after.level.min(7) - before.level.min(7);
    before
        .card_items
        .into_iter()
        .zip(after.card_items)
        .try_fold(0i64, |gained, (old, new)| {
            (new >= old).then_some(())?;
            gained.checked_add(new - old)
        })
        .is_some_and(|gained| gained <= allowance)
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
    #[serde(default)]
    cyclic_golds: Vec<String>,
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

/// Per-match public evidence, sampled at the upgrade prompt and after reveal.
#[derive(Default)]
pub(super) struct PublicUpgradeHistory {
    tracked: std::collections::HashMap<crate::data::Symbol, TrackedUpgrades>,
    checkpoints: BTreeMap<String, Vec<UpgradeCheckpoint>>,
    pub(super) batches: Vec<PublicUpgradeBatch>,
}

struct TrackedUpgrades {
    known: UpgradeCheckpoint,
    pending: Option<UpgradeCheckpoint>,
}

impl PublicUpgradeHistory {
    pub(super) fn new(session: &super::NativeSession) -> Result<Self> {
        let catalog = catalog()?;
        let mut history = Self::default();
        for hero in &session.world.heroes {
            if hero.level == 1
                && session.world.entity(hero.id).is_some_and(|entity| {
                    catalog.heroes.get(session.symbols.name(entity.name))
                        .is_some_and(|definition| definition.standard)
                })
            {
                history.tracked.insert(hero.id, TrackedUpgrades {
                    known: UpgradeCheckpoint { level: 1, card_items: [0; 6] },
                    pending: None,
                });
            }
        }
        Ok(history)
    }

    pub(super) fn checkpoints(&self) -> Option<&BTreeMap<String, Vec<UpgradeCheckpoint>>> {
        (!self.checkpoints.is_empty()).then_some(&self.checkpoints)
    }

    pub(super) fn begin_batch(&mut self, session: &super::NativeSession) -> Result<()> {
        for hero in &session.world.heroes {
            if let Some(tracked) = self.tracked.get_mut(&hero.id)
                && tracked.known.level < hero.level
                && tracked.pending.is_none()
                && let Some(items) = session.public_pre_upgrade_items(hero.id)?
            {
                tracked.pending = Some(UpgradeCheckpoint { level: hero.level, card_items: items });
            }
        }
        Ok(())
    }

    pub(super) fn planning_boundary(
        &mut self,
        session: &super::NativeSession,
        revision: u64,
    ) -> Result<()> {
        for hero in &session.world.heroes {
            let Some(tracked) = self.tracked.get_mut(&hero.id) else {
                continue;
            };
            let before = tracked.known;
            if before.level == hero.level {
                continue;
            }
            let Some(after) = tracked.reveal(hero.level, hero.items) else {
                // A missed or inconsistent reveal leaves later cumulative
                // totals unknown. Earlier checkpoints remain valid evidence.
                self.tracked.remove(&hero.id);
                continue;
            };
            let hero_id = session.symbols.name(hero.id).to_owned();
            let checkpoints = self.checkpoints.entry(hero_id.clone()).or_default();
            if checkpoints.last() != Some(&before) {
                checkpoints.push(before);
            }
            checkpoints.push(after);
            self.batches.push(PublicUpgradeBatch { revision, hero_id, before, after });
        }
        Ok(())
    }
}

impl TrackedUpgrades {
    fn reveal(&mut self, level: i64, public_items: [i64; 6]) -> Option<UpgradeCheckpoint> {
        let pending = self.pending.take();
        if self.known.level == 7 && level == 8 {
            // The ultimate unlock adds no ordinary card item and can
            // advance straight to planning without an upgrade prompt.
            self.known.level = level;
            return Some(self.known);
        }
        let pending = pending?;
        if pending.level != level {
            return None;
        }
        let mut after = self.known;
        after.level = level;
        for (index, public) in public_items.into_iter().enumerate() {
            let gain = public.checked_sub(pending.card_items[index])?;
            after.card_items[index] = after.card_items[index].checked_add(gain)?;
        }
        if !ordinary_item_gains_fit(self.known, after) {
            return None;
        }
        self.known = after;
        Some(after)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HiddenGoldSlot {
    Played(usize),
    Discard(usize),
}

#[derive(Debug, PartialEq, Eq)]
pub struct SampledHand {
    pub hero: String,
    pub cards: Vec<String>,
    pub loadout: LoadoutHypothesis,
    pub(crate) retired: Vec<String>,
    pub(crate) hidden_gold: Option<(HiddenGoldSlot, String)>,
}

fn hidden_gold_slots(hero: &super::HeroObservation) -> impl Iterator<Item = HiddenGoldSlot> + '_ {
    fn slots(
        cards: &[PublicCard],
        slot: fn(usize) -> HiddenGoldSlot,
    ) -> impl Iterator<Item = HiddenGoldSlot> + '_ {
        cards.iter().enumerate().filter_map(move |(index, card)| {
            matches!(card, PublicCard::Hidden).then(|| slot(index))
        })
    }

    slots(&hero.played, HiddenGoldSlot::Played)
        .chain(slots(&hero.discarded, HiddenGoldSlot::Discard))
}

pub(super) fn sample_preserved_gold(
    hero: &super::HeroObservation,
    seed: u64,
) -> Result<Option<(HiddenGoldSlot, String)>> {
    let mut slots = hidden_gold_slots(hero);
    let Some(slot) = slots.next() else {
        return Ok(None);
    };
    let golds = catalog()?.heroes.get(&hero.name).map(|row| &row.cyclic_golds);
    let Some(golds) = golds.filter(|golds| {
        hero.level < 8 && !golds.is_empty() && slots.next().is_none()
    }) else {
        return Err(EngineError::Unsupported("unsupported concealed slots on preserved hero".into()));
    };
    Ok(Some((slot, golds[sample_index(seed ^ 0x81f2b1aa727ea991, golds.len())].clone())))
}

pub(super) fn sample_index(seed: u64, len: usize) -> usize {
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
        if self.phase != crate::codec::Phase::Planning {
            return Ok(None);
        }
        self.sample_ordinary_hand(hero_id, seed, checkpoints)
    }

    /// Sample an ordinary hidden hand after current cards are revealed.
    /// A mismatched count indicates unsupported card provenance.
    pub fn sample_resolution_hand_with_checkpoints(
        &self,
        hero_id: &str,
        seed: u64,
        checkpoints: &[UpgradeCheckpoint],
    ) -> Result<Option<SampledHand>> {
        if !matches!(self.phase, crate::codec::Phase::Resolution | crate::codec::Phase::Cleanup) {
            return Ok(None);
        }
        self.sample_ordinary_hand(hero_id, seed, checkpoints)
    }

    fn sample_ordinary_hand(
        &self,
        hero_id: &str,
        seed: u64,
        checkpoints: &[UpgradeCheckpoint],
    ) -> Result<Option<SampledHand>> {
        Ok(self
            .ordinary_hand_sampler(hero_id, checkpoints)?
            .and_then(|sample| sample(seed)))
    }

    pub(super) fn sample_resolution_hands(
        &self,
        hero_id: &str,
        seeds: impl Iterator<Item = u64>,
        checkpoints: &[UpgradeCheckpoint],
    ) -> Result<Option<Vec<SampledHand>>> {
        if !matches!(self.phase, crate::codec::Phase::Resolution | crate::codec::Phase::Cleanup) {
            return Ok(None);
        }
        Ok(self
            .ordinary_hand_sampler(hero_id, checkpoints)?
            .and_then(|sample| seeds.map(sample).collect()))
    }

    fn ordinary_hand_sampler<'a>(
        &'a self,
        hero_id: &str,
        checkpoints: &[UpgradeCheckpoint],
    ) -> Result<Option<impl Fn(u64) -> Option<SampledHand> + use<'a>>> {
        if hero_id == self.private.hero {
            return Ok(None);
        }
        let Some(hero) = self.heroes.iter().find(|hero| hero.id == hero_id) else {
            return Err(EngineError::InvalidInput("unknown observed hero".into()));
        };
        let zones = hero.played.iter().chain(&hero.discarded);
        let Some(definition) = catalog()?.heroes.get(&hero.name) else {
            return Ok(None);
        };
        let unavailable = zones
            .filter_map(|card| match card {
                PublicCard::Revealed(id) => Some(id),
                _ => None,
            })
            .chain(&hero.revealed_current)
            .collect::<BTreeSet<_>>();
        let public_golds = definition
            .cyclic_golds
            .iter()
            .filter(|id| unavailable.contains(id))
            .count();
        if !definition.cyclic_golds.is_empty() && hero.level < 8 && public_golds > 1 {
            return Ok(None);
        }
        let mut hidden = hidden_gold_slots(hero);
        let hidden_gold_slot = hidden.next();
        if hidden_gold_slot.is_some()
            && (definition.cyclic_golds.is_empty() || hero.level >= 8
                || public_golds != 0 || hidden.next().is_some())
        {
            return Ok(None);
        }
        let loadouts = if !definition.standard && hero.level == 1 {
            // These heroes have nonstandard upgrade provenance. At level one
            // the printed starting pool is sufficient if the evidence agrees.
            if checkpoints
                .iter()
                .any(|point| point.level != 1 || point.card_items != [0; 6])
                || unavailable.iter().any(|id| !definition.starting.contains(id))
                || self.revealed_cards.iter().any(|(owner, id)| {
                    owner == hero_id && !definition.starting.contains(id)
                })
            {
                return Ok(None);
            }
            vec![LoadoutHypothesis::default()]
        } else {
            let Some(loadouts) = self
                .loadout_hypotheses_with_checkpoints(hero_id, checkpoints)?
                .filter(|rows| !rows.is_empty())
            else {
                return Ok(None);
            };
            loadouts
        };
        Ok(Some(move |seed| {
            let selected = sample_index(seed, loadouts.len());
            let loadout = &loadouts[selected];
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
                    !definition.cyclic_golds.contains(id)
                        && !(hero.level >= 8 && !definition.cyclic_golds.is_empty()
                            && id.as_str() == "bushido")
                        && !definition
                        .cards
                        .iter()
                        .any(|card| card.id == **id && upgraded.contains(&card.color.index()))
                })
                .chain(&loadout.active_card_ids)
                .filter(|id| !unavailable.contains(id))
                .cloned()
                .collect::<Vec<_>>();
            let mut hidden_gold = None;
            if hero.level >= 8 {
                cards.extend(
                    definition.cyclic_golds.iter().filter(|id| !unavailable.contains(id)).cloned()
                );
            } else if public_golds == 0 && !definition.cyclic_golds.is_empty() {
                let gold = if hidden_gold_slot.is_none()
                    && self.phase == crate::codec::Phase::Planning
                    && self.round == 1 && self.turn == 1
                {
                    definition.starting.iter().find(|id| definition.cyclic_golds.contains(id))?
                } else {
                    &definition.cyclic_golds[sample_index(
                        seed ^ 0x81f2b1aa727ea991, definition.cyclic_golds.len()
                    )]
                };
                if let Some(slot) = hidden_gold_slot {
                    hidden_gold = Some((slot, gold.clone()));
                } else {
                    cards.push(gold.clone());
                }
            }
            // The reconstructed pool still includes unrevealed commitments.
            // Only the count is public and can rule out an impossible pool.
            if cards.len() < hero.hand_size
                || (self.phase != crate::codec::Phase::Planning && cards.len() != hero.hand_size)
            {
                return None;
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
            Some(SampledHand {
                hero: hero.id.clone(),
                cards,
                loadout: loadout.clone(),
                retired,
                hidden_gold,
            })
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
            .filter(|definition| definition.standard || matches!(hero.name.as_str(), "Dodger" | "Min" | "Snorri"))
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
        let swaps_possible = self.heroes.iter().any(|ally| {
            ally.team == hero.team && ally.name == "Snorri" && (
                self.revealed_cards.iter().any(|(owner, id)| {
                    *owner == ally.id && id == "ancestral_grace"
                }) || ally.revealed_current.iter().any(|id| id == "ancestral_grace")
                    || ally.played.iter().chain(&ally.discarded).any(|card| {
                        matches!(card, PublicCard::Revealed(id) if id == "ancestral_grace")
                    })
            )
        });
        let Some(mut candidates) = enumerate(
            definition, hero.level, &reveals, checkpoints, swaps_possible,
        ) else {
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
        if hero.name == "Min" {
            let compatible = candidates
                .into_iter()
                .flat_map(|row| min_item_conversions(row, definition))
                .filter(|row| items_match(row, definition, &hero.items, false, None))
                .collect::<BTreeSet<_>>();
            return Ok(Some(compatible.into_iter().collect()));
        }
        for extra in [false, true] {
            let filtered = candidates
                .iter()
                .filter(|row| {
                    items_match(
                        row,
                        definition,
                        &hero.items,
                        extra && hero.name != "Snorri",
                        (hero.name == "Dodger").then_some(hero.level),
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            if !filtered.is_empty() {
                return Ok(Some(filtered));
            }
        }
        if matches!(hero.name.as_str(), "Dodger" | "Snorri") {
            return Ok(Some(Vec::new()));
        }
        Ok(Some(candidates))
    }
}

fn enumerate(
    hero: &StaticHero,
    level: i64,
    reveals: &[&str],
    checkpoints: &[UpgradeCheckpoint],
    swaps_possible: bool,
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
                if swaps_possible {
                    // A swap preserves the number of items, but can change
                    // which stat was recorded at an earlier upgrade batch.
                    point.card_items.into_iter().try_fold(0i64, i64::checked_add)
                        == Some(counts.iter().sum())
                } else {
                    counts == point.card_items
                }
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
            if swaps_possible && card.tier > 1 {
                return hero.cards[active[card.color.index()]].tier >= card.tier;
            }
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

// Perfect Self can turn a retired Tier II card into an item.
// Each color has at most one such card from its ordinary upgrade path.
fn min_item_conversions(row: LoadoutHypothesis, definition: &StaticHero) -> Vec<LoadoutHypothesis> {
    if !row.active_card_ids.iter().any(|id| id == "perfect_self") {
        return vec![row];
    }
    let eligible = definition.cards.iter().filter(|card| {
        card.tier == 2
            && card.item.is_some()
            && !row.item_card_ids.contains(&card.id)
            && definition.cards.iter().any(|active| {
                active.tier == 3
                    && active.color.index() == card.color.index()
                    && row.active_card_ids.contains(&active.id)
            })
    });
    let mut variants = vec![row.clone()];
    for card in eligible {
        let count = variants.len();
        for index in 0..count {
            let mut converted = variants[index].clone();
            let slot = converted.item_card_ids.binary_search(&card.id).unwrap_err();
            converted.item_card_ids.insert(slot, card.id.clone());
            variants.push(converted);
        }
    }
    variants
}

fn items_match(
    row: &LoadoutHypothesis,
    hero: &StaticHero,
    public: &[i64; 6],
    extra: bool,
    dodger_level: Option<i64>,
) -> bool {
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
    if let Some(level) = dodger_level {
        return counts.iter().zip(public).enumerate().all(|(index, (count, wanted))| {
            if extra && level >= 8 && index == Stat::Attack.index() {
                count <= wanted
            } else {
                count == wanted
            }
        });
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
    fn resolution_sample_excludes_revealed_card_and_requires_exact_hand_size() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let session = super::super::NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        )
        .unwrap();
        let mut view = session
            .player_observation(session.world.heroes[0].id)
            .unwrap();
        let hero_id = view.heroes[1].id.clone();
        let planning = view.sample_planning_hand(&hero_id, 39).unwrap().unwrap();
        let revealed = planning.cards[0].clone();
        view.phase = crate::codec::Phase::Resolution;
        view.heroes[1].revealed_current.push(revealed.clone());
        view.heroes[1].hand_size = planning.cards.len() - 1;
        let sample = view
            .sample_resolution_hand_with_checkpoints(&hero_id, 39, &[])
            .unwrap()
            .unwrap();
        assert_eq!(sample.cards.len(), view.heroes[1].hand_size);
        assert!(!sample.cards.contains(&revealed));
        view.phase = crate::codec::Phase::Cleanup;
        assert_eq!(
            view.sample_resolution_hand_with_checkpoints(&hero_id, 39, &[])
                .unwrap()
                .unwrap(),
            sample
        );
        view.heroes[1].hand_size -= 1;
        assert!(
            view.sample_resolution_hand_with_checkpoints(&hero_id, 39, &[])
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn takahide_samples_exactly_one_gold_before_the_ultimate() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let session = super::super::NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        )
        .unwrap();
        let mut view = session.player_observation(session.world.heroes[0].id).unwrap();
        view.phase = crate::codec::Phase::Resolution;
        view.revealed_cards.clear();
        let hero = &mut view.heroes[1];
        hero.name = "Takahide".into();
        hero.level = 1;
        hero.items = [0; 6];
        hero.hand_size = 5;
        hero.played.clear();
        hero.discarded.clear();
        hero.revealed_current.clear();
        let id = hero.id.clone();
        let golds = &catalog().unwrap().heroes["Takahide"].cyclic_golds;
        let mut seen = BTreeSet::new();
        for seed in 0..32 {
            let sample = view
                .sample_resolution_hand_with_checkpoints(&id, seed, &[])
                .unwrap()
                .unwrap();
            let sampled: Vec<_> = sample.cards.into_iter().filter(|id| golds.contains(id)).collect();
            assert_eq!(sampled.len(), 1);
            seen.extend(sampled);
        }
        assert_eq!(seen.len(), 3);

        view.heroes[1].revealed_current.push("sting_like_a_bee".into());
        view.heroes[1].hand_size = 4;
        let sample = view
            .sample_resolution_hand_with_checkpoints(&id, 0, &[])
            .unwrap()
            .unwrap();
        assert!(sample.cards.iter().all(|id| !golds.contains(id)));

        view.heroes[1].played.push(PublicCard::Hidden);
        assert!(view.sample_resolution_hand_with_checkpoints(&id, 0, &[]).unwrap().is_none());

        view.heroes[1].revealed_current.clear();
        let sample = view
            .sample_resolution_hand_with_checkpoints(&id, 0, &[])
            .unwrap()
            .unwrap();
        assert_eq!(sample.cards.len(), 4);
        assert!(sample.cards.iter().all(|id| !golds.contains(id)));
        let (slot, gold) = sample.hidden_gold.unwrap();
        assert_eq!(slot, HiddenGoldSlot::Played(0));
        assert!(golds.contains(&gold));

        view.heroes[1].played.clear();
        view.heroes[1].discarded.push(PublicCard::Hidden);
        let discarded = view
            .sample_resolution_hand_with_checkpoints(&id, 0, &[])
            .unwrap()
            .unwrap();
        assert_eq!(discarded.cards, sample.cards);
        assert_eq!(discarded.hidden_gold, Some((HiddenGoldSlot::Discard(0), gold)));
        let preserved: BTreeSet<_> = (0..32)
            .map(|seed| {
                let (slot, id) = sample_preserved_gold(&view.heroes[1], seed).unwrap().unwrap();
                assert_eq!(slot, HiddenGoldSlot::Discard(0));
                id
            })
            .collect();
        assert_eq!(preserved, seen);
    }

    #[test]
    fn item_swap_evidence_allows_both_cards_from_an_upgraded_pair_to_have_been_seen() {
        let hero = &catalog().unwrap().heroes["Snorri"];
        let reveals = ["runic_hammer", "runecaster"];
        assert!(enumerate(hero, 2, &reveals, &[], false).unwrap().is_empty());
        let swapped = enumerate(hero, 2, &reveals, &[], true).unwrap();
        assert_eq!(swapped.len(), 2);
        for row in swapped {
            assert_eq!(row.active_card_ids.len(), 1);
            assert_eq!(row.item_card_ids.len(), 1);
            assert_ne!(row.active_card_ids, row.item_card_ids);
        }
    }

    #[test]
    fn min_item_conversion_requires_perfect_self_and_a_retired_tier_two() {
        let min = &catalog().unwrap().heroes["Min"];
        let mut row = LoadoutHypothesis {
            active_card_ids: vec!["perfect_self".into(), "viper_stance".into()],
            item_card_ids: vec!["ruse".into(), "tiger_stance".into(), "vanish".into()],
        };
        let variants = min_item_conversions(row.clone(), min);
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0], row);
        assert_eq!(
            variants[1].item_card_ids,
            ["inner_strength", "ruse", "tiger_stance", "vanish"]
        );

        row.active_card_ids[0] = "ruse".into();
        row.item_card_ids[0] = "perfect_self".into();
        assert_eq!(min_item_conversions(row.clone(), min), [row]);
    }

    #[test]
    fn dodger_sampling_respects_reveals_and_card_items() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let session = super::super::NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        )
        .unwrap();
        let mut view = session.player_observation(session.world.heroes[0].id).unwrap();
        view.phase = crate::codec::Phase::Resolution;
        view.revealed_cards.clear();
        let hero = &mut view.heroes[1];
        hero.name = "Dodger".into();
        hero.hand_size = 3;
        hero.played = vec![PublicCard::Revealed("dark_ritual".into())];
        hero.discarded.clear();
        hero.revealed_current = vec!["dread_razor".into()];
        let id = hero.id.clone();
        let sample = view.sample_resolution_hand_with_checkpoints(&id, 39, &[]).unwrap().unwrap();
        assert_eq!(sample.cards, ["death_trap", "littlefinger_of_death", "shield_of_decay"]);
        assert_eq!(sample.loadout, LoadoutHypothesis::default());

        view.revealed_cards.push((id.clone(), "unknown_card".into()));
        assert!(view.sample_resolution_hand_with_checkpoints(&id, 39, &[]).unwrap().is_none());
        view.revealed_cards.clear();
        view.heroes[1].level = 2;
        assert!(view.sample_resolution_hand_with_checkpoints(&id, 39, &[]).unwrap().is_none());

        view.heroes[1].items[Stat::Defense.index()] = 1;
        view.heroes[1].revealed_current = vec!["finger_of_death".into()];
        view.heroes[1].hand_size = 3;
        let upgraded = view
            .sample_resolution_hand_with_checkpoints(&id, 39, &[])
            .unwrap()
            .unwrap();
        assert_eq!(upgraded.loadout.item_card_ids, ["burning_skull"]);
        assert_eq!(upgraded.cards, ["death_trap", "dread_razor", "shield_of_decay"]);
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
    fn revealed_batch_records_only_ordinary_item_deltas() {
        let mut tracked = TrackedUpgrades {
            known: UpgradeCheckpoint { level: 1, card_items: [0; 6] },
            pending: Some(UpgradeCheckpoint { level: 4, card_items: [2, 0, 0, 0, 0, 0] }),
        };
        let point = tracked.reveal(4, [3, 1, 0, 1, 0, 0]).unwrap();
        assert_eq!(point, UpgradeCheckpoint { level: 4, card_items: [1, 1, 0, 1, 0, 0] });
        assert!(tracked.pending.is_none());
        assert!(tracked.reveal(5, [3, 1, 0, 2, 0, 0]).is_none());
        assert_eq!(tracked.known, point);
    }

    #[test]
    fn automatic_ultimate_unlock_preserves_ordinary_items_without_a_prompt() {
        let before = UpgradeCheckpoint { level: 7, card_items: [1, 2, 1, 1, 1, 0] };
        let mut tracked = TrackedUpgrades { known: before, pending: None };
        let after = tracked.reveal(8, [2, 2, 1, 1, 1, 0]).unwrap();
        assert_eq!(after, UpgradeCheckpoint { level: 8, ..before });
        assert_eq!(tracked.known, after);
        assert!(tracked.pending.is_none());
    }

    #[test]
    fn public_batches_bound_ordinary_item_gains_by_levels_crossed() {
        let checkpoint = |level, attack| UpgradeCheckpoint {
            level,
            card_items: [attack, 0, 0, 0, 0, 0],
        };
        let batch = |revision, before, after| PublicUpgradeBatch {
            revision,
            hero_id: "hero_arien".into(),
            before,
            after,
        };
        let initial = checkpoint(1, 0);
        let seventh = checkpoint(7, 6);
        let eighth = checkpoint(8, 6);
        assert!(replay_public_upgrade_batches(&[batch(1, initial, eighth)]).is_ok());
        assert!(replay_public_upgrade_batches(&[batch(1, seventh, eighth)]).is_ok());
        assert!(
            replay_public_upgrade_batches(&[batch(1, seventh, checkpoint(8, 7))]).is_err()
        );
        assert!(
            replay_public_upgrade_batches(&[batch(1, checkpoint(2, 2), checkpoint(3, 2))])
                .is_err()
        );
        let first = batch(1, initial, checkpoint(2, 0));
        let later = batch(2, checkpoint(3, 2), checkpoint(4, 2));
        assert!(replay_public_upgrade_batches(&[first, later.clone()]).is_err());
        assert!(replay_public_upgrade_batches(&[later]).is_ok());
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
        assert_eq!(enumerate(hero, 4, &[], &[level_four], false).unwrap().len(), 2);
        let aggregate = enumerate(hero, 5, &[], &[level_five], false).unwrap();
        assert_eq!(aggregate.len(), 4);
        let observed = enumerate(hero, 5, &[], &[level_four, level_five], false).unwrap();
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
                }],
                false,
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
            let mut result = enumerate(hero, case.level, &reveals, &[], false).unwrap();
            if let Some(items) = case.items {
                let mut public = [0; 6];
                for (name, value) in items {
                    public[Stat::parse(&name).unwrap().index()] = value;
                }
                result.retain(|row| items_match(row, hero, &public, case.extra, None));
            }
            assert_eq!(result, case.expected, "{} level {}", case.hero, case.level);
        }
    }
}
