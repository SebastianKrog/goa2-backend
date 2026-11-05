//! Player-scoped search facts. Physical card IDs and hidden planning buffers
//! are deliberately excluded from the observation.
use super::{EngineError, NativeSession, Result};
use crate::{
    cards::{CardId, HeroRecord},
    codec::Phase,
    data::{Datum, Symbol},
    geometry::Hex,
    objects::ObjectValue,
    stats::Stat,
};

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
    pub hand_size: usize,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamKnowledgeMode {
    #[default]
    TrueRules,
    Coordinated,
    CoordinatedShow,
    CoordinatedKnowSameTeamHumans,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct VisibleCommitment {
    pub hero: String,
    pub first: String,
    pub second: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BotTeamObservation {
    pub mode: TeamKnowledgeMode,
    pub player: PlayerObservation,
    pub allied_bots: Vec<PrivateCards>,
    pub allied_human_commitments: Vec<VisibleCommitment>,
}

impl NativeSession {
    /// The integration must supply the authorized bot roster, not client claims.
    pub fn bot_team_observation(
        &self,
        player: Symbol,
        bots: &[Symbol],
        mode: TeamKnowledgeMode,
    ) -> Result<BotTeamObservation> {
        let team = self.knowledge_team(player, bots, true)?;
        let mut result = BotTeamObservation {
            mode,
            player: self.player_observation(player)?,
            allied_bots: Vec::new(),
            allied_human_commitments: Vec::new(),
        };
        for ally in self
            .world
            .heroes
            .iter()
            .filter(|hero| hero.id != player && hero.team == team)
        {
            if bots.contains(&ally.id) && mode != TeamKnowledgeMode::TrueRules {
                result.allied_bots.push(self.private_cards(ally)?);
            } else if !bots.contains(&ally.id)
                && mode == TeamKnowledgeMode::CoordinatedKnowSameTeamHumans
                && let Some(commitment) = self.visible_commitment(ally)?
            {
                result.allied_human_commitments.push(commitment);
            }
        }
        Ok(result)
    }

    /// Human-facing projection for show mode. It carries committed cards only.
    pub fn shown_bot_commitments(
        &self,
        viewer: Symbol,
        bots: &[Symbol],
        mode: TeamKnowledgeMode,
    ) -> Result<Vec<VisibleCommitment>> {
        let team = self.knowledge_team(viewer, bots, false)?;
        if mode != TeamKnowledgeMode::CoordinatedShow {
            return Ok(Vec::new());
        }
        let mut commitments = Vec::new();
        for ally in self
            .world
            .heroes
            .iter()
            .filter(|hero| hero.team == team && bots.contains(&hero.id))
        {
            if let Some(commitment) = self.visible_commitment(ally)? {
                commitments.push(commitment);
            }
        }
        Ok(commitments)
    }

    pub(super) fn knowledge_team(
        &self,
        viewer: Symbol,
        bots: &[Symbol],
        bot_viewer: bool,
    ) -> Result<Symbol> {
        if bots.contains(&viewer) != bot_viewer
            || bots
                .iter()
                .any(|id| !self.world.heroes.iter().any(|hero| hero.id == *id))
        {
            return Err(EngineError::InvalidInput(
                "invalid team knowledge roster or viewer".into(),
            ));
        }
        self.world
            .heroes
            .iter()
            .find(|hero| hero.id == viewer)
            .map(|hero| hero.team)
            .ok_or_else(|| EngineError::InvalidInput("knowledge viewer is not a hero".into()))
    }

    fn visible_commitment(
        &self,
        hero: &crate::cards::HeroRecord,
    ) -> Result<Option<VisibleCommitment>> {
        if self.planning.phase != Phase::Planning {
            return Ok(None);
        }
        let (first, second) = self.observed_planning_cards(hero.id)?;
        first
            .map(|first| {
                Ok(VisibleCommitment {
                    hero: self.symbols.name(hero.id).to_owned(),
                    first: self.printed_observation_card(first)?,
                    second: second
                        .map(|card| self.printed_observation_card(card))
                        .transpose()?,
                })
            })
            .transpose()
    }

    pub(super) fn printed_observation_card(&self, id: CardId) -> Result<String> {
        self.world
            .card(id)
            .map(|card| {
                self.symbols
                    .name(self.world.definitions.cards[card.definition].printed_id)
                    .to_owned()
            })
            .ok_or_else(|| EngineError::Unsupported("observed card is not bound".into()))
    }

    fn private_cards(&self, hero: &crate::cards::HeroRecord) -> Result<PrivateCards> {
        let printed = |card| self.printed_observation_card(card);
        let (current, extra) = if self.planning.phase == Phase::Planning {
            self.observed_planning_cards(hero.id)?
        } else {
            (hero.current, hero.extra)
        };
        Ok(PrivateCards {
            hero: self.symbols.name(hero.id).to_owned(),
            hand: hero
                .hand
                .iter()
                .copied()
                .map(printed)
                .collect::<Result<_>>()?,
            current: current.map(printed).transpose()?,
            extra: extra.map(printed).transpose()?,
        })
    }

    fn observation_items(&self, player: Symbol, hero: &HeroRecord) -> Result<[i64; 6]> {
        if hero.id == player {
            return Ok(hero.items);
        }
        let invalid = || EngineError::Unsupported("invalid public upgrade item snapshot".into());
        let Some(objects) = &self.objects else {
            return if self.planning.phase == Phase::LevelUp {
                Err(invalid())
            } else {
                Ok(hero.items)
            };
        };
        let Some(snapshot) = self
            .symbols
            .lookup("upgrade_reveal_snapshot")
            .and_then(|key| objects.arena.root_field(key))
        else {
            return Ok(hero.items);
        };
        let ObjectValue::Object(snapshot) = snapshot else {
            return Err(invalid());
        };
        let Some(items) = objects.arena.field(snapshot, hero.id) else {
            return Ok(hero.items);
        };
        let ObjectValue::Object(items) = items else {
            return Err(invalid());
        };
        let mut visible = [0; 6];
        for &(key, ref value) in objects.arena.fields(items).ok_or_else(invalid)? {
            let stat = Stat::parse(self.symbols.name(key)).map_err(|_| invalid())?;
            let ObjectValue::Atom(Datum::Int(count)) = value else {
                return Err(invalid());
            };
            visible[stat.index()] = *count;
        }
        Ok(visible)
    }

    /// Project the public board and one hero's own cards for search sampling.
    /// Other heroes' hand counts are public. Their card identities are hidden.
    pub fn player_observation(&self, player: Symbol) -> Result<PlayerObservation> {
        let hero = self
            .world
            .hero(player)
            .filter(|hero| hero.id == player)
            .ok_or_else(|| EngineError::InvalidInput("observation player is not a hero".into()))?;
        let name = |symbol| self.symbols.name(symbol).to_owned();
        let printed = |id| self.printed_observation_card(id);
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
                        items: self.observation_items(player, row)?,
                        hand_size: row.hand.len(),
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
            private: self.private_cards(hero)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::SampledHand;
    use super::*;

    #[test]
    fn upgrade_items_are_hidden_from_other_viewers_until_reveal() {
        let fixture = super::super::object_execution::fixture_case(
            "object-execution-upgrade-live-Arien-3-single-0",
        );
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let arien = root.symbols.lookup("hero_arien").unwrap();
        let observer = root
            .world
            .heroes
            .iter()
            .find(|hero| hero.id != arien)
            .unwrap()
            .id;
        let own = root.player_observation(arien).unwrap();
        let other = root.player_observation(observer).unwrap();
        let items = |view: PlayerObservation| {
            view.heroes
                .into_iter()
                .find(|hero| hero.id == "hero_arien")
                .unwrap()
                .items
        };
        assert_eq!(items(own), root.world.hero(arien).unwrap().items);
        assert_eq!(items(other), [-2, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn team_knowledge_modes_share_only_authorized_allied_cards() {
        let fixture =
            super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let viewer = root.world.heroes[0].id;
        let team = root.world.heroes[0].team;
        let ally = root
            .world
            .heroes
            .iter()
            .find(|hero| hero.id != viewer && hero.team == team)
            .unwrap();
        let (ally_id, ally_card) = (ally.id, ally.hand[0]);
        let enemy = root
            .world
            .heroes
            .iter()
            .find(|hero| hero.team != team)
            .unwrap();
        let (enemy_id, enemy_card) = (enemy.id, enemy.hand[0]);
        root.commit_card(ally_id, ally_card).unwrap();
        root.commit_card(enemy_id, enemy_card).unwrap();
        let bots = [viewer, ally_id, enemy_id];
        for mode in [
            TeamKnowledgeMode::Coordinated,
            TeamKnowledgeMode::CoordinatedShow,
            TeamKnowledgeMode::CoordinatedKnowSameTeamHumans,
        ] {
            let view = root.bot_team_observation(viewer, &bots, mode).unwrap();
            assert_eq!(view.allied_bots.len(), 1);
            assert_eq!(view.allied_bots[0].hero, root.symbols.name(ally_id));
            assert_eq!(
                view.allied_bots[0].current,
                Some(root.printed_observation_card(ally_card).unwrap())
            );
            assert!(view.allied_human_commitments.is_empty());
        }
        let strict = root
            .bot_team_observation(viewer, &bots, TeamKnowledgeMode::TrueRules)
            .unwrap();
        assert!(strict.allied_bots.is_empty() && strict.allied_human_commitments.is_empty());
        let coordinated = root
            .sampled_bot_team_world(viewer, &bots, TeamKnowledgeMode::Coordinated, 39, None)
            .unwrap();
        let sampled_view = coordinated
            .bot_team_observation(viewer, &bots, TeamKnowledgeMode::Coordinated)
            .unwrap();
        let actual_view = root
            .bot_team_observation(viewer, &bots, TeamKnowledgeMode::Coordinated)
            .unwrap();
        assert_eq!(sampled_view.allied_bots, actual_view.allied_bots);
        assert_eq!(
            coordinated.observed_planning_cards(enemy_id).unwrap(),
            (None, None)
        );
        let strict_world = root
            .sampled_bot_team_world(viewer, &bots, TeamKnowledgeMode::TrueRules, 39, None)
            .unwrap();
        assert_eq!(
            strict_world.observed_planning_cards(ally_id).unwrap(),
            (None, None)
        );
        let bots = [viewer, enemy_id];
        let show = root
            .bot_team_observation(viewer, &bots, TeamKnowledgeMode::CoordinatedShow)
            .unwrap();
        assert!(show.allied_human_commitments.is_empty());
        let know = root
            .bot_team_observation(
                viewer,
                &bots,
                TeamKnowledgeMode::CoordinatedKnowSameTeamHumans,
            )
            .unwrap();
        let known_world = root
            .sampled_bot_team_world(
                viewer,
                &bots,
                TeamKnowledgeMode::CoordinatedKnowSameTeamHumans,
                39,
                None,
            )
            .unwrap();
        assert_eq!(
            known_world.observed_planning_cards(ally_id).unwrap(),
            (Some(ally_card), None)
        );
        assert!(
            !known_world
                .world
                .hero(ally_id)
                .unwrap()
                .hand
                .contains(&ally_card)
        );
        assert_eq!(
            known_world.observed_planning_cards(enemy_id).unwrap(),
            (None, None)
        );
        assert_eq!(know.allied_human_commitments.len(), 1);
        assert_eq!(
            know.allied_human_commitments[0].first,
            root.printed_observation_card(ally_card).unwrap()
        );
        root.commit_card(viewer, root.world.hero(viewer).unwrap().hand[0])
            .unwrap();
        let shared = root
            .shown_bot_commitments(ally_id, &bots, TeamKnowledgeMode::CoordinatedShow)
            .unwrap();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].hero, root.symbols.name(viewer));
        assert!(
            root.shown_bot_commitments(ally_id, &bots, TeamKnowledgeMode::Coordinated)
                .unwrap()
                .is_empty()
        );
    }

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
            left.player_observation(viewer).unwrap(),
            right.player_observation(viewer).unwrap()
        );
        // Sampling deliberately reopens the epoch, restoring its hand counts.
        assert_eq!(
            sampled.player_observation(viewer).unwrap(),
            root.player_observation(viewer).unwrap()
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
    fn commitment_changes_only_public_count_and_owners_private_cards() {
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
                    let mut observed = branch.player_observation(*id)?;
                    let row = observed
                        .heroes
                        .iter_mut()
                        .find(|hero| &hero.id == owner_id)
                        .unwrap();
                    assert_eq!(row.hand_size + 1, before.private.hand.len());
                    assert_eq!(
                        observed.sample_planning_hand(owner_id, 17)?,
                        Some(SampledHand {
                            hero: sample.hero.clone(),
                            cards: sample.cards.clone(),
                            loadout: sample.loadout.clone(),
                            retired: sample.retired.clone(),
                        })
                    );
                    observed
                        .heroes
                        .iter_mut()
                        .find(|hero| &hero.id == owner_id)
                        .unwrap()
                        .hand_size += 1;
                    assert_eq!(&observed, expected);
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
        row.hand_size -= 1;
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
