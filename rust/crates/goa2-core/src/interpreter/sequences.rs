//! Shared-prefix legality checks on a worker-local live session.
use super::{
    EngineError, ExecutionBudget, NativeRequest, NativeSession, RequestChoices, RequestToken,
    Result,
};
use crate::{
    cards::CardId,
    codec::Phase,
    data::{Datum, Symbol},
};

use std::{collections::HashMap, hash::BuildHasher};

#[derive(Clone, Debug, Hash, PartialEq)]
pub enum DecisionMove {
    Card {
        hero: Symbol,
        card: CardId,
    },
    Pass {
        hero: Symbol,
    },
    FinishPlanning {
        hero: Symbol,
    },
    Input {
        player: Symbol,
        request: Option<RequestToken>,
        selection: Datum,
    },
}

/// A complete choice for one hero before a simultaneous planning reveal.
/// For an open second-card slot, first is the next card (or None to finish)
/// and second is always None.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlanningCommitment {
    pub hero: Symbol,
    pub first: Option<CardId>,
    pub second: Option<CardId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SequenceVerdict {
    Legal { checked: usize },
    Illegal { failed_at: usize, reason: String },
    Unknown { failed_at: usize, reason: String },
}

#[derive(Debug)]
pub struct SequenceBatch {
    pub verdicts: Vec<SequenceVerdict>,
    pub applied_decisions: usize,
}

#[derive(Default)]
struct Prefix<'a> {
    children: Vec<(&'a DecisionMove, usize)>,
    outputs: Vec<usize>,
}

impl NativeSession {
    /// Evaluate a fixed joint planning choice under rollback. All choices are
    /// checked against the same pre-commit state before any card is applied.
    pub fn with_joint_planning<T>(
        &mut self,
        commitments: &[PlanningCommitment],
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let invalid = |message: &str| EngineError::InvalidInput(message.into());
        if self.planning.phase != Phase::Planning {
            return Err(invalid("joint commitments require planning"));
        }
        let mut ordered = Vec::new();
        for hero in &self.world.heroes {
            let choices = self.planning_choices(hero.id)?;
            let mut matches = commitments
                .iter()
                .filter(|commitment| commitment.hero == hero.id);
            if choices.is_empty() {
                if matches.next().is_some() {
                    return Err(invalid("joint choice includes a closed planning slot"));
                }
                continue;
            }
            let second_slot = choices.contains(&DecisionMove::FinishPlanning { hero: hero.id });
            let commitment = matches
                .next()
                .ok_or_else(|| invalid("joint choice is missing an open hero"))?;
            if matches.next().is_some() {
                return Err(invalid("joint choice repeats a hero"));
            }
            let first = match commitment.first {
                Some(card) => DecisionMove::Card {
                    hero: hero.id,
                    card,
                },
                None if second_slot => DecisionMove::FinishPlanning { hero: hero.id },
                None => DecisionMove::Pass { hero: hero.id },
            };
            if !choices.contains(&first) {
                return Err(invalid("first commitment is not a legal planning choice"));
            }
            if let Some(second) = commitment.second
                && (second_slot
                    || commitment.first.is_none()
                    || commitment.first == Some(second)
                    || !self.planning.two_cards(&self.world, hero.id)
                    || !choices.contains(&DecisionMove::Card {
                        hero: hero.id,
                        card: second,
                    }))
            {
                return Err(invalid("second commitment is not a legal planning choice"));
            }
            ordered.push(*commitment);
        }
        if ordered.is_empty() || ordered.len() != commitments.len() {
            return Err(invalid("joint choice is empty or includes an unknown hero"));
        }
        self.with_rollback(|worker| {
            worker.apply_joint_planning_ordered(&ordered)?;
            operation(worker)
        })
    }

    // The caller validates or generates the complete plan before mutation and
    // owns the rollback scope.
    pub(super) fn apply_joint_planning_ordered(
        &mut self,
        commitments: &[PlanningCommitment],
    ) -> Result<()> {
        for commitment in commitments {
            if let Some(card) = commitment.first {
                self.commit_card(commitment.hero, card)?;
                if let Some(second) = commitment.second {
                    self.commit_card(commitment.hero, second)?;
                } else if self.planning_choices(commitment.hero)?.contains(
                    &DecisionMove::FinishPlanning {
                        hero: commitment.hero,
                    },
                ) {
                    self.finish_planning(commitment.hero)?;
                }
            } else if self.observed_planning_cards(commitment.hero)?.0.is_some() {
                self.finish_planning(commitment.hero)?;
            } else {
                self.pass_planning(commitment.hero)?;
            }
        }
        if self.planning.phase == Phase::Planning {
            return Err(EngineError::InvalidInput(
                "joint commitments did not close planning".into(),
            ));
        }
        Ok(())
    }

    /// Enumerate meaningful live planning decisions in hand order.
    pub fn planning_choices(&self, hero: Symbol) -> Result<Vec<DecisionMove>> {
        self.planning_choices_up_to(hero, usize::MAX)
    }

    fn planning_choices_up_to(&self, hero: Symbol, limit: usize) -> Result<Vec<DecisionMove>> {
        let objects = self.objects.as_ref().ok_or_else(|| {
            EngineError::Unsupported(
                "planning choice generation requires live object bindings".into(),
            )
        })?;
        self.planning_choices_bound(objects, hero, limit)
    }

    pub(super) fn first_planning_commitment(
        &self,
        hero: Symbol,
    ) -> Result<Option<PlanningCommitment>> {
        let choices = self.planning_choices_up_to(hero, 1)?;
        let first = match choices.first() {
            Some(DecisionMove::Card { card, .. }) => Some(*card),
            Some(DecisionMove::Pass { .. } | DecisionMove::FinishPlanning { .. }) => None,
            None => return Ok(None),
            _ => {
                return Err(EngineError::Unsupported(
                    "unexpected planning choice".into(),
                ));
            }
        };
        Ok(Some(PlanningCommitment {
            hero,
            first,
            second: None,
        }))
    }

    /// Enumerate a player's current resolution inputs in native option order.
    /// Structured choices use one rune or upgrade at a time, so search can
    /// reach every arrangement through successive decisions.
    pub fn resolution_choices(&mut self, player: Symbol) -> Result<Vec<DecisionMove>> {
        Ok(self.resolution_choices_up_to(player, usize::MAX)?.0)
    }

    pub(super) fn first_resolution_choice(
        &mut self,
        player: Symbol,
    ) -> Result<Option<DecisionMove>> {
        Ok(self.resolution_choices_up_to(player, 1)?.0.pop())
    }

    /// Distinguish a forced input from a branch without materializing every
    /// destination. A duplicated prefix does not prove the input is forced.
    pub(super) fn resolution_branch_probe(&mut self, player: Symbol) -> Result<Vec<DecisionMove>> {
        let (choices, exhausted) = self.resolution_choices_up_to(player, 2)?;
        if choices.len() == 2 || exhausted {
            Ok(choices)
        } else {
            self.resolution_choices(player)
        }
    }

    pub(super) fn resolution_request_addresses(
        &self,
        request: &NativeRequest,
        player: Symbol,
    ) -> bool {
        let Some(hero) = self.world.hero(player).filter(|hero| hero.id == player) else {
            return false;
        };
        match &request.choices {
            RequestChoices::Actors { team, .. } => hero.team == *team,
            RequestChoices::Upgrades(players) => players.iter().any(|row| row.hero == player),
            _ => self
                .symbols
                .name(request.owner)
                .strip_prefix("team:")
                .map_or(request.owner == player, |team| {
                    team == self.symbols.name(hero.team)
                }),
        }
    }

    fn resolution_choices_up_to(
        &mut self,
        player: Symbol,
        limit: usize,
    ) -> Result<(Vec<DecisionMove>, bool)> {
        enum Choice {
            Typed(Datum),
            Named(&'static str),
        }
        use Choice::{Named, Typed};

        let Some(request) = &self.last_request else {
            return Ok((Vec::new(), true));
        };
        if !self.resolution_request_addresses(request, player) {
            return Ok((Vec::new(), true));
        }

        let token = request.id;
        let text = |id| Typed(Datum::Text(id));
        let printed = |id| {
            let card = self
                .world
                .card(id)
                .ok_or_else(|| EngineError::Unsupported("request card is not bound".into()))?;
            Ok(self.world.definitions.cards[card.definition].printed_id)
        };
        let mut choices: Vec<Choice> =
            match &request.choices {
                RequestChoices::Actors { values, .. }
                | RequestChoices::Runes(values)
                | RequestChoices::CardColors(values)
                | RequestChoices::CardIds { values, .. }
                | RequestChoices::Multiple { values, .. }
                | RequestChoices::RuneInscription {
                    remaining: values, ..
                } => values.iter().take(limit).copied().map(text).collect(),
                RequestChoices::Cards(values) => values
                    .iter()
                    .take(limit)
                    .map(|(id, _, _)| text(*id))
                    .collect(),
                RequestChoices::SpellCards { values, .. } | RequestChoices::Protection(values) => {
                    values
                        .iter()
                        .take(limit)
                        .copied()
                        .map(|card| printed(card).map(text))
                        .collect::<Result<_>>()?
                }
                RequestChoices::Units { values, .. } => {
                    values.iter().take(limit).map(|(id, _)| text(*id)).collect()
                }
                RequestChoices::Minions(values) => {
                    values.iter().take(limit).map(|(id, _)| text(*id)).collect()
                }
                RequestChoices::Hexes(values)
                | RequestChoices::ZoneHexes { values, .. }
                | RequestChoices::Respawn {
                    hexes: values,
                    location: true,
                } => values
                    .iter()
                    .take(limit)
                    .map(|hex| Typed(super::native_hex(*hex, self.coordinates)))
                    .collect(),
                RequestChoices::Respawn {
                    location: false, ..
                } => vec![Named("RESPAWN"), Named("PASS")],
                RequestChoices::MirrorDistances(values) => values
                    .iter()
                    .take(limit)
                    .map(|(distance, _)| Typed(Datum::Int(*distance)))
                    .collect(),
                RequestChoices::Numbers { values, .. } => values
                    .iter()
                    .take(limit)
                    .map(|n| Typed(Datum::Int(*n)))
                    .collect(),
                RequestChoices::Mines(paths) => (0..paths.len().min(limit))
                    .map(|index| Typed(Datum::Int(index as i64)))
                    .collect(),
                RequestChoices::Laugh
                | RequestChoices::YesNo
                | RequestChoices::Repeat { .. }
                | RequestChoices::Passive { .. } => {
                    vec![Named("YES"), Named("NO")]
                }
                RequestChoices::ConfirmAction => vec![Named("CONFIRM")],
                RequestChoices::PrimaryReplacement {
                    primary,
                    replacement,
                    ..
                } => {
                    vec![Named(primary.name()), Named(replacement.name())]
                }
                RequestChoices::Action(menu) => menu
                    .options
                    .iter()
                    .take(limit)
                    .map(|option| Named(option.kind.name()))
                    .collect(),
                RequestChoices::Defense(menu) => menu
                    .cards
                    .iter()
                    .take(limit)
                    .map(|card| text(card.id))
                    .chain(std::iter::once(Named("PASS")))
                    .collect(),
                RequestChoices::Upgrades(players) => {
                    let hero_key = self.symbols.lookup("hero_id").ok_or_else(|| {
                        EngineError::Unsupported("upgrade hero key missing".into())
                    })?;
                    let card_key = self.symbols.lookup("card_id").ok_or_else(|| {
                        EngineError::Unsupported("upgrade card key missing".into())
                    })?;
                    let mut choices = Vec::new();
                    for card in players
                        .iter()
                        .filter(|row| row.hero == player)
                        .flat_map(|row| &row.options)
                        .flat_map(|option| &option.pair)
                        .take(limit)
                    {
                        let card = Datum::Text(printed(*card)?);
                        choices.push(Typed(Datum::Object(vec![
                            (hero_key, Datum::Text(player)),
                            (card_key, card),
                        ])));
                    }
                    choices
                }
            };
        if request.can_skip {
            choices.push(Named("SKIP"));
            if matches!(request.choices, RequestChoices::Multiple { .. }) {
                choices.push(Named("DONE"));
            }
        }

        // Fewer raw candidates than the limit proves the list was exhausted.
        // Counting after deduplication cannot distinguish a truncated prefix.
        let exhausted = choices.len() < limit;
        let mut moves = Vec::with_capacity(choices.len());
        for choice in choices.into_iter().take(limit) {
            let selection = match choice {
                Typed(value) => value,
                Named(name) => Datum::Text(self.choice_symbol(name)),
            };
            let decision = DecisionMove::Input {
                player,
                request: Some(token),
                selection,
            };
            if !moves.contains(&decision) {
                moves.push(decision);
            }
        }
        Ok((moves, exhausted))
    }

    /// Choose an input using the ported deterministic Python heuristic ranks.
    pub fn heuristic_input_choice(&mut self, player: Symbol) -> Result<Option<DecisionMove>> {
        let moves = self.resolution_choices(player)?;
        if moves.is_empty() {
            return Ok(None);
        }
        let request = self
            .last_request
            .as_ref()
            .expect("choices require a live request");
        let selection = |decision: &DecisionMove| match decision {
            DecisionMove::Input { selection, .. } => selection.clone(),
            _ => unreachable!("resolution choices are inputs"),
        };
        let best = match &request.choices {
            RequestChoices::Numbers { .. } | RequestChoices::MirrorDistances(_) => moves
                .iter()
                .filter_map(|decision| match selection(decision) {
                    Datum::Int(number) => Some((number, decision)),
                    _ => None,
                })
                .fold(None, |best: Option<(i64, &DecisionMove)>, candidate| {
                    if best.is_none_or(|(score, _)| candidate.0 > score) {
                        Some(candidate)
                    } else {
                        best
                    }
                })
                .map_or(&moves[0], |(_, decision)| decision),
            RequestChoices::Defense(menu) => {
                let rank = |decision| match selection(decision) {
                    Datum::Text(id) => menu
                        .cards
                        .iter()
                        .find(|card| card.id == id)
                        .map_or(0, |card| card.total),
                    _ => 0,
                };
                moves.iter().fold(&moves[0], |best, candidate| {
                    if rank(candidate) > rank(best) {
                        candidate
                    } else {
                        best
                    }
                })
            }
            RequestChoices::Upgrades(players) => {
                let row = players
                    .iter()
                    .find(|row| row.hero == player)
                    .expect("upgrade choices require an addressed hero");
                let first_attack = row
                    .options
                    .iter()
                    .position(|option| {
                        option.pair.iter().any(|id| {
                            self.world.card(*id).is_some_and(|card| {
                                !card.facedown
                                    && self.world.definitions.cards[card.definition].primary
                                        == Some(crate::world::ActionKind::Attack)
                            })
                        })
                    })
                    .unwrap_or(0);
                let offset = row.options[..first_attack]
                    .iter()
                    .map(|option| option.pair.len())
                    .sum::<usize>();
                moves.get(offset).unwrap_or(&moves[0])
            }
            RequestChoices::Units { .. }
            | RequestChoices::Minions(_)
            | RequestChoices::Hexes(_)
            | RequestChoices::ZoneHexes { .. }
            | RequestChoices::Respawn { location: true, .. }
            | RequestChoices::Action(_) => {
                return Err(EngineError::Unsupported(
                    "heuristic input rank not ported for this prompt".into(),
                ));
            }
            _ => &moves[0],
        };
        Ok(Some(best.clone()))
    }

    pub(super) fn choice_symbol(&mut self, name: &str) -> Symbol {
        if let Some(id) = self.symbols.lookup(name) {
            return id;
        }
        let id = std::sync::Arc::make_mut(&mut self.symbols).intern(name);
        if let Some(objects) = &mut self.objects {
            objects.arena.symbols = self.symbols.clone();
        }
        id
    }

    fn validate_planning_player(&self, hero: Symbol) -> Result<()> {
        if self.planning.phase != Phase::Planning {
            return Err(EngineError::InvalidInput(
                "planning decision outside planning phase".into(),
            ));
        }
        if !self
            .world
            .hero(hero)
            .is_some_and(|record| record.id == hero)
        {
            return Err(EngineError::InvalidInput("unknown planning player".into()));
        }
        Ok(())
    }

    /// Check typed decisions with automatic advancement between them.
    /// Unported input validators return Unknown. Engine and budget errors
    /// abort the batch and restore the caller's session.
    pub fn check_sequences(
        &mut self,
        sequences: &[Vec<DecisionMove>],
        budget: &ExecutionBudget<'_>,
    ) -> Result<SequenceBatch> {
        if sequences.iter().any(|sequence| sequence.len() > 128) {
            return Err(EngineError::InvalidInput(
                "sequence exceeds 128 decisions".into(),
            ));
        }
        let mut trie = vec![Prefix::default()];
        // Preserve insertion order in the trie. Large batches use a separate
        // index so distinct destinations don't scan every prior sibling.
        let mut edges = (sequences.len() >= 64).then(HashMap::<(usize, u64), usize>::new);
        for (output, sequence) in sequences.iter().enumerate() {
            let mut node = 0;
            for decision in sequence {
                let siblings = &trie[node].children;
                let find = || siblings.iter().position(|(key, _)| *key == decision);
                let indexed = edges.as_mut().map(|index| {
                    let key = (node, index.hasher().hash_one(decision));
                    index.entry(key)
                });
                let existing = match &indexed {
                    Some(std::collections::hash_map::Entry::Occupied(edge)) => {
                        let position = *edge.get();
                        if siblings[position].0 == decision {
                            Some(position)
                        } else {
                            find()
                        }
                    }
                    Some(std::collections::hash_map::Entry::Vacant(_)) => None,
                    None => find(),
                };
                if let Some(position) = existing {
                    node = siblings[position].1;
                } else {
                    if let Some(edge) = indexed {
                        edge.insert_entry(siblings.len());
                    }
                    let child = trie.len();
                    trie.push(Prefix::default());
                    trie[node].children.push((decision, child));
                    node = child;
                }
            }
            trie[node].outputs.push(output);
        }
        let mut batch = SequenceBatch {
            verdicts: vec![SequenceVerdict::Legal { checked: 0 }; sequences.len()],
            applied_decisions: 0,
        };
        if !trie[0].children.is_empty() {
            self.with_rollback(|worker| {
                worker.advance_to_decision(budget)?;
                visit(worker, &trie, 0, 0, budget, &mut batch)
            })?;
        }
        Ok(batch)
    }

    /// Run automatic rules until a prompt, planning, or an idle boundary.
    pub fn advance_to_decision(&mut self, budget: &ExecutionBudget<'_>) -> Result<()> {
        let mut remaining = budget.max_steps.min(1000);
        while self.last_request.is_none()
            && !matches!(self.planning.phase, Phase::Planning | Phase::GameOver)
        {
            let bounded = ExecutionBudget {
                max_steps: remaining,
                ..*budget
            };
            let advance = if self.objects.is_some() {
                self.advance_objects(&bounded, None, false)?
            } else {
                self.advance(&bounded)?
            };
            if advance.steps == 0 {
                break;
            }
            remaining -= advance.steps;
        }
        Ok(())
    }

    fn validate_sequence_input(
        &mut self,
        player: Symbol,
        token: Option<RequestToken>,
        selection: &Datum,
    ) -> Result<RequestToken> {
        let invalid = |message: &str| EngineError::InvalidInput(message.into());
        let request = self
            .last_request
            .as_ref()
            .ok_or_else(|| invalid("no input decision is pending"))?;
        if token.is_some_and(|token| token != request.id) {
            return Err(invalid("stale input request"));
        }
        let upgrade_hero_matches = !matches!(request.choices, RequestChoices::Upgrades(_))
            || matches!(selection, Datum::Object(fields) if fields.iter().any(|(key, value)| {
                self.symbols.lookup("hero_id") == Some(*key) && *value == Datum::Text(player)
            }));
        if !upgrade_hero_matches || !self.resolution_request_addresses(request, player) {
            return Err(invalid("input is addressed to another player"));
        }
        if matches!(request.choices, RequestChoices::Upgrades(_)) {
            let id = request.id;
            self.validate_upgrade_submission_bound(selection)?;
            return Ok(id);
        }
        let text = if let Datum::Text(id) = selection {
            Some(*id)
        } else {
            None
        };
        let named = |name| text.is_some() && text == self.symbols.lookup(name);
        let printed_card = |id| {
            self.world.card(id).is_some_and(|card| {
                text == Some(self.world.definitions.cards[card.definition].printed_id)
            })
        };
        let offered = match &request.choices {
            RequestChoices::Actors { values, .. }
            | RequestChoices::Runes(values)
            | RequestChoices::CardColors(values)
            | RequestChoices::CardIds { values, .. }
            | RequestChoices::Multiple { values, .. } => {
                text.is_some_and(|id| values.contains(&id))
            }
            RequestChoices::Cards(values) => values.iter().any(|(id, _, _)| text == Some(*id)),
            RequestChoices::SpellCards { values, .. } | RequestChoices::Protection(values) => {
                values.iter().copied().any(printed_card)
            }
            RequestChoices::Units { values, .. } => values.iter().any(|(id, _)| text == Some(*id)),
            RequestChoices::Minions(values) => values.iter().any(|(id, _)| text == Some(*id)),
            RequestChoices::Hexes(values)
            | RequestChoices::ZoneHexes { values, .. }
            | RequestChoices::Respawn {
                hexes: values,
                location: true,
            } => selection
                .hex(self.coordinates, &self.symbols)
                .is_ok_and(|hex| values.contains(&hex)),
            RequestChoices::Respawn {
                location: false, ..
            } => named("RESPAWN") || named("PASS"),
            RequestChoices::MirrorDistances(values) => values.iter().any(|(distance, _)| {
                selection.to_integer(&self.symbols).ok().flatten() == Some(*distance)
            }),
            RequestChoices::Numbers { values, .. } => {
                matches!(selection, Datum::Int(value) if values.contains(value))
            }
            RequestChoices::Mines(paths) => matches!(selection, Datum::Int(value)
                if usize::try_from(*value).is_ok_and(|index| index < paths.len())),
            RequestChoices::Laugh
            | RequestChoices::YesNo
            | RequestChoices::Repeat { .. }
            | RequestChoices::Passive { .. } => named("YES") || named("NO"),
            RequestChoices::RuneInscription { remaining, .. } => {
                super::valid_rune_batch(selection, remaining).is_some()
            }
            RequestChoices::ConfirmAction => named("CONFIRM"),
            RequestChoices::PrimaryReplacement {
                primary,
                replacement,
                ..
            } => named(primary.name()) || named(replacement.name()),
            RequestChoices::Action(menu) => {
                menu.options.iter().any(|option| named(option.kind.name()))
            }
            RequestChoices::Defense(menu) => {
                named("PASS") || menu.cards.iter().any(|card| text == Some(card.id))
            }
            RequestChoices::Upgrades(_) => unreachable!("validated structured upgrade above"),
        };
        let skip = request.can_skip && named("SKIP");
        let done = request.can_skip
            && matches!(request.choices, RequestChoices::Multiple { .. })
            && named("DONE");
        if !offered && !skip && !done {
            return Err(invalid("selection is not offered by the current decision"));
        }
        Ok(request.id)
    }
}

fn visit(
    worker: &mut NativeSession,
    trie: &[Prefix<'_>],
    node: usize,
    depth: usize,
    budget: &ExecutionBudget<'_>,
    batch: &mut SequenceBatch,
) -> Result<()> {
    assign(
        trie,
        node,
        &SequenceVerdict::Legal { checked: depth },
        false,
        batch,
    );
    let children = &trie[node].children;
    for (index, (decision, child)) in children.iter().enumerate() {
        budget.check(0)?;
        let terminal = trie[*child].children.is_empty();
        if terminal
            && let DecisionMove::Input { player, request, selection } = decision
            && !worker.last_request.as_ref().is_some_and(|request| {
                matches!(request.choices, RequestChoices::Upgrades(_))
            })
        {
            // Plain prompt membership is read-only and the leaf has no
            // transition to execute. The root rollback still owns the batch.
            let verdict = sequence_failure(
                worker.validate_sequence_input(*player, *request, selection).map(|_| ()),
                depth,
            )?.unwrap_or(SequenceVerdict::Legal { checked: depth + 1 });
            assign(trie, *child, &verdict, false, batch);
            continue;
        }
        let mut evaluate = |branch: &mut NativeSession| {
            let applied = match decision {
                DecisionMove::Card { hero, card } => {
                    branch.validate_planning_player(*hero).and_then(|()| {
                        if !branch.planning_choices(*hero)?.contains(decision) {
                            return Err(EngineError::InvalidInput(
                                "card is not a playable planning choice".into(),
                            ));
                        }
                        branch.commit_card(*hero, *card)
                    })
                }
                DecisionMove::Pass { hero } => branch
                    .validate_planning_player(*hero)
                    .and_then(|()| branch.pass_planning(*hero)),
                DecisionMove::FinishPlanning { hero } => branch
                    .validate_planning_player(*hero)
                    .and_then(|()| branch.finish_planning(*hero)),
                DecisionMove::Input {
                    player,
                    request,
                    selection,
                } => branch
                    .validate_sequence_input(*player, *request, selection)
                    .and_then(|token| {
                        if terminal {
                            Ok(())
                        } else {
                            branch.submit(token, selection.clone())?;
                            branch.last_request = None;
                            Ok(())
                        }
                    }),
            };
            if let Some(failure) = sequence_failure(applied, depth)? {
                assign(trie, *child, &failure, true, batch);
            } else {
                if !terminal || !matches!(decision, DecisionMove::Input { .. }) {
                    batch.applied_decisions += 1;
                }
                if !terminal {
                    branch.advance_to_decision(budget)?;
                }
                visit(branch, trie, *child, depth + 1, budget, batch)?;
            }
            Ok(())
        };
        // Only a later sibling needs this node restored. The last child
        // can use the rollback already owned by the enclosing branch.
        if index + 1 == children.len() {
            evaluate(worker)?;
        } else {
            worker.with_rollback(evaluate)?;
        }
    }
    Ok(())
}

fn sequence_failure(applied: Result<()>, depth: usize) -> Result<Option<SequenceVerdict>> {
    Ok(match applied {
        Ok(()) => None,
        Err(EngineError::InvalidInput(reason)) => Some(SequenceVerdict::Illegal {
            failed_at: depth,
            reason,
        }),
        Err(EngineError::Unsupported(reason)) => Some(SequenceVerdict::Unknown {
            failed_at: depth,
            reason,
        }),
        Err(error) => return Err(error),
    })
}

fn assign(
    trie: &[Prefix<'_>],
    node: usize,
    verdict: &SequenceVerdict,
    descendants: bool,
    batch: &mut SequenceBatch,
) {
    for output in &trie[node].outputs {
        batch.verdicts[*output] = verdict.clone();
    }
    if descendants {
        for (_, child) in &trie[node].children {
            assign(trie, *child, verdict, true, batch);
        }
    }
}
