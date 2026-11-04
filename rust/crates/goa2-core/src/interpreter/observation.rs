//! Player-scoped search facts. Physical card IDs and hidden planning buffers
//! are deliberately excluded from the observation.
use super::{EngineError, NativeSession, Result};
use crate::{cards::CardId, codec::Phase, data::Symbol, geometry::Hex};

#[derive(Debug, PartialEq, Eq)]
pub struct PlayerObservation {
    pub phase: Phase,
    pub round: i64,
    pub turn: i64,
    pub heroes: Vec<HeroObservation>,
    pub positions: Vec<(String, Hex)>,
    pub team_lives: Vec<(String, i64)>,
    pub revealed_cards: Vec<(String, String)>,
    pub private: PrivateCards,
}

#[derive(Debug, PartialEq, Eq)]
pub struct HeroObservation {
    pub id: String,
    pub team: String,
    pub name: String,
    pub level: i64,
    pub gold: i64,
    pub items: [i64; 6],
    pub played: Vec<PublicCard>,
    pub discarded: Vec<PublicCard>,
    pub revealed_current: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PublicCard {
    Empty,
    Hidden,
    Revealed(String),
}

#[derive(Debug, PartialEq, Eq)]
pub struct PrivateCards {
    pub hero: String,
    pub hand: Vec<String>,
    pub current: Option<String>,
    pub extra: Option<String>,
}

impl NativeSession {
    /// Project the public board and one hero's own cards for search sampling.
    /// Other heroes' hands, hand sizes, and pending commits are omitted.
    pub fn player_observation(&self, player: Symbol) -> Result<PlayerObservation> {
        let hero = self
            .world
            .hero(player)
            .filter(|hero| hero.id == player)
            .ok_or_else(|| EngineError::InvalidInput("observation player is not a hero".into()))?;
        let name = |symbol| self.symbols.name(symbol).to_owned();
        let printed = |id: CardId| {
            self.world
                .card(id)
                .map(|card| name(self.world.definitions.cards[card.definition].printed_id))
                .ok_or_else(|| EngineError::Unsupported("observed card is not bound".into()))
        };
        let public_card = |id: Option<CardId>| {
            let Some(id) = id else {
                return Ok(PublicCard::Empty);
            };
            let card = self
                .world
                .card(id)
                .ok_or_else(|| EngineError::Unsupported("public card is not bound".into()))?;
            if card.facedown {
                Ok(PublicCard::Hidden)
            } else {
                printed(id).map(PublicCard::Revealed)
            }
        };
        Ok(PlayerObservation {
            phase: self.planning.phase,
            round: self.world.combat.round,
            turn: self.world.combat.turn,
            heroes: self
                .world
                .heroes
                .iter()
                .map(|row| {
                    Ok(HeroObservation {
                        id: name(row.id),
                        team: name(row.team),
                        name: self
                            .world
                            .entity(row.id)
                            .map(|entity| name(entity.name))
                            .unwrap_or_default(),
                        level: row.level,
                        gold: row.gold,
                        items: row.items,
                        played: row
                            .played
                            .iter()
                            .copied()
                            .map(public_card)
                            .collect::<Result<_>>()?,
                        discarded: row
                            .discard
                            .iter()
                            .map(|id| public_card(Some(*id)))
                            .collect::<Result<_>>()?,
                        revealed_current: [row.current, row.extra]
                            .into_iter()
                            .flatten()
                            .filter(|id| self.world.card(*id).is_some_and(|card| !card.facedown))
                            .map(printed)
                            .collect::<Result<_>>()?,
                    })
                })
                .collect::<Result<_>>()?,
            positions: self
                .world
                .locations
                .iter()
                .map(|(id, hex)| (name(*id), *hex))
                .collect(),
            team_lives: self
                .world
                .combat
                .team_lives()
                .map(|(team, life)| (name(team), life))
                .collect(),
            revealed_cards: self
                .world
                .history
                .revealed_cards()
                .iter()
                .map(|(hero, card)| (name(*hero), name(*card)))
                .collect(),
            private: PrivateCards {
                hero: name(hero.id),
                hand: hero
                    .hand
                    .iter()
                    .copied()
                    .map(printed)
                    .collect::<Result<_>>()?,
                current: hero.current.map(printed).transpose()?,
                extra: hero.extra.map(printed).transpose()?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::SampledHand;
    use super::*;

    #[test]
    fn sampled_worker_erases_other_commitments_and_accepts_new_choices() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let opponent = root.world.heroes[0].id;
        let viewer = root.world.heroes[1].id;
        let hand = &root.world.heroes[0].hand;
        let mut left = root.clone();
        let mut right = root.clone();
        left.commit_card(opponent, hand[0]).unwrap();
        right.commit_card(opponent, hand[1]).unwrap();
        let before = serde_json::to_value(left.export_envelope().unwrap()).unwrap();
        let mut sampled = left.sampled_planning_world(viewer, 23).unwrap();
        let alternative = right.sampled_planning_world(viewer, 23).unwrap();
        assert_eq!(
            serde_json::to_value(sampled.export_envelope().unwrap()).unwrap(),
            serde_json::to_value(alternative.export_envelope().unwrap()).unwrap(),
        );
        assert_eq!(
            sampled.player_observation(viewer).unwrap(),
            left.player_observation(viewer).unwrap()
        );
        let choices = sampled
            .world
            .heroes
            .iter()
            .map(|row| {
                let moves = sampled.planning_choices(row.id).unwrap();
                let first = match moves[0] {
                    super::super::DecisionMove::Card { card, .. } => Some(card),
                    super::super::DecisionMove::Pass { .. } => None,
                    _ => panic!("expected an open sampled planning slot"),
                };
                super::super::PlanningCommitment {
                    hero: row.id,
                    first,
                    second: None,
                }
            })
            .collect::<Vec<_>>();
        let sampled_before = serde_json::to_value(sampled.export_envelope().unwrap()).unwrap();
        assert!(
            sampled
                .with_joint_planning(&choices[..1], |_| Ok(()))
                .is_err()
        );
        let mut reversed = choices.clone();
        reversed.reverse();
        let first = sampled
            .with_joint_planning(&choices, |worker| {
                assert_ne!(worker.planning.phase, Phase::Planning);
                worker.export_envelope()
            })
            .unwrap();
        let second = sampled
            .with_joint_planning(&reversed, |worker| worker.export_envelope())
            .unwrap();
        assert_eq!(
            serde_json::to_value(first).unwrap(),
            serde_json::to_value(second).unwrap()
        );
        assert_eq!(
            serde_json::to_value(sampled.export_envelope().unwrap()).unwrap(),
            sampled_before
        );
        let choices = sampled.planning_choices(opponent).unwrap();
        assert!(choices.len() > 1);
        let super::super::DecisionMove::Card { hero, card } = choices[0] else {
            panic!("expected sampled card");
        };
        sampled.commit_card(hero, card).unwrap();
        assert_eq!(sampled.world.hero(hero).unwrap().current, Some(card));
        assert_eq!(
            serde_json::to_value(left.export_envelope().unwrap()).unwrap(),
            before
        );
    }

    #[test]
    fn commitment_changes_only_the_owners_observation() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut session =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let owner = session.world.heroes[0].id;
        let card = session.world.heroes[0].hand[0];
        let others = session
            .world
            .heroes
            .iter()
            .filter(|hero| hero.id != owner)
            .map(|hero| (hero.id, session.player_observation(hero.id).unwrap()))
            .collect::<Vec<_>>();
        let before = session.player_observation(owner).unwrap();
        let owner_id = &before.private.hero;
        let (_, viewer) = &others[0];
        let sample = viewer.sample_planning_hand(owner_id, 17).unwrap().unwrap();
        let committed = before.private.hand.first().unwrap();
        assert!(sample.cards.contains(committed));
        session
            .with_rollback(|branch| {
                branch.commit_card(owner, card)?;
                let after = branch.player_observation(owner)?;
                assert_eq!(after.private.hand.len() + 1, before.private.hand.len());
                assert_eq!(after.private.current.as_ref(), before.private.hand.first());
                for (id, expected) in &others {
                    let observed = branch.player_observation(*id)?;
                    assert_eq!(&observed, expected);
                    assert_eq!(
                        observed.sample_planning_hand(owner_id, 17)?,
                        Some(SampledHand {
                            hero: sample.hero.clone(),
                            cards: sample.cards.clone(),
                            loadout: sample.loadout.clone(),
                            retired: sample.retired.clone(),
                        })
                    );
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(session.player_observation(owner).unwrap(), before);
        let mut observed = session.player_observation(others[0].0).unwrap();
        let row = observed
            .heroes
            .iter_mut()
            .find(|hero| &hero.id == owner_id)
            .unwrap();
        row.discarded.push(PublicCard::Revealed(committed.clone()));
        let filtered = observed
            .sample_planning_hand(owner_id, 17)
            .unwrap()
            .unwrap();
        assert!(!filtered.cards.contains(committed));
        let row = observed
            .heroes
            .iter_mut()
            .find(|hero| &hero.id == owner_id)
            .unwrap();
        row.discarded.push(PublicCard::Hidden);
        assert!(
            observed
                .sample_planning_hand(owner_id, 17)
                .unwrap()
                .is_none()
        );
    }
}
