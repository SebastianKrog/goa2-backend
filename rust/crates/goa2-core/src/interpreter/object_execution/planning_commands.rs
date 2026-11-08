//! Live planning commands preserve source collection replacement and aliasing.
use super::super::{TeamKnowledgeMode, UpgradeCheckpoint, loadout::HiddenGoldSlot};
use super::phases::Field;
use super::*;
use crate::cards::CardId;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Default, Debug, serde::Serialize)]
pub(crate) struct CardSyncProfile {
    calls: usize,
    lists: usize,
    changed_lists: usize,
    skipped_heroes: usize,
    changed_heroes: usize,
}

thread_local! {
    static CARD_SYNC_PROFILE: std::cell::Cell<Option<CardSyncProfile>> = const {
        std::cell::Cell::new(None)
    };
}

pub(super) fn with_card_sync_profile<T>(run: impl FnOnce() -> T) -> (T, CardSyncProfile) {
    struct Restore(Option<CardSyncProfile>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CARD_SYNC_PROFILE.set(self.0);
        }
    }
    let _restore = Restore(CARD_SYNC_PROFILE.replace(Some(CardSyncProfile::default())));
    let result = run();
    (result, CARD_SYNC_PROFILE.get().unwrap())
}

struct PlanningHandSample {
    hero: Symbol,
    update: SampledCards,
    fixed: [Option<CardId>; 2],
}

enum SampledCards {
    Hand(super::super::SampledHand),
    ConcealedGold(HiddenGoldSlot, String),
}

impl NativeSession {
    pub(in crate::interpreter) fn observed_planning_cards(
        &self,
        hero: Symbol,
    ) -> Result<(Option<CardId>, Option<CardId>)> {
        let Some(objects) = &self.objects else {
            return Ok(self.planning.committed_cards(hero));
        };
        self.observed_planning_cards_bound(objects, hero)
    }

    pub(super) fn observed_planning_cards_bound(
        &self,
        objects: &BoundObjects,
        hero: Symbol,
    ) -> Result<(Option<CardId>, Option<CardId>)> {
        let card = |field| {
            let collection = objects.phases.root(&objects.arena, field)?;
            match objects.arena.field(collection, hero) {
                Some(ObjectValue::Atom(Datum::Instance(id))) => Ok(Some(CardId(id))),
                None | Some(ObjectValue::Atom(Datum::Null)) => Ok(None),
                _ => Err(unsupported("invalid planning commitment binding")),
            }
        };
        Ok((card(Field::Pending)?, card(Field::Second)?))
    }

    /// Fork a clean planning boundary with all other ordinary hands sampled.
    /// The returned worker leaves other heroes uncommitted so a joint planning
    /// policy can select their moves from the same pre-reveal state.
    pub fn sampled_planning_world(&self, player: Symbol, seed: u64) -> Result<Self> {
        self.sampled_planning_world_with_checkpoints(player, seed, None)
    }

    pub(in crate::interpreter) fn sampled_planning_world_with_checkpoints(
        &self,
        player: Symbol,
        seed: u64,
        checkpoints: Option<&BTreeMap<String, Vec<UpgradeCheckpoint>>>,
    ) -> Result<Self> {
        let samples = self.planning_samples(player, seed, checkpoints, &[], &[])?;
        let mut worker = self.clone();
        worker.install_planning_samples(samples)?;
        Ok(worker)
    }

    /// Sample an ordinary planning world while preserving exact bot cards
    /// authorized by the team mode. The roster comes from trusted integration.
    pub fn sampled_bot_team_world(
        &self,
        player: Symbol,
        bots: &[Symbol],
        mode: TeamKnowledgeMode,
        seed: u64,
        checkpoints: Option<&BTreeMap<String, Vec<UpgradeCheckpoint>>>,
    ) -> Result<Self> {
        let view = self.bot_team_observation(player, bots, mode)?;
        let preserved = view
            .allied_bots
            .iter()
            .filter_map(|hero| self.symbols.lookup(&hero.hero))
            .collect::<Vec<_>>();
        let known = view
            .allied_human_commitments
            .iter()
            .filter_map(|row| self.symbols.lookup(&row.hero))
            .collect::<Vec<_>>();
        let samples = self.planning_samples(player, seed, checkpoints, &preserved, &known)?;
        let mut worker = self.clone();
        worker.install_planning_samples(samples)?;
        Ok(worker)
    }

    /// Install one observation-derived sample for a speculative operation.
    pub fn with_sampled_planning<T>(
        &mut self,
        player: Symbol,
        seed: u64,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.with_sampled_planning_checkpoints(player, seed, None, operation)
    }

    pub(in crate::interpreter) fn with_sampled_planning_checkpoints<T>(
        &mut self,
        player: Symbol,
        seed: u64,
        checkpoints: Option<&BTreeMap<String, Vec<UpgradeCheckpoint>>>,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let samples = self.planning_samples(player, seed, checkpoints, &[], &[])?;
        self.with_rollback(|worker| {
            worker.install_planning_samples(samples)?;
            operation(worker)
        })
    }

    pub(in crate::interpreter) fn with_sampled_resolution<T>(
        &mut self,
        player: Symbol,
        sample: super::super::PlanningRolloutSample<'_>,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let samples = self
            .resolution_samples(player, sample, &[sample.seed], &[])?
            .pop()
            .unwrap();
        self.with_installed_resolution_samples(player, samples, Some(sample.seed), operation)
    }

    pub(in crate::interpreter) fn sampled_resolution_workers(
        &mut self,
        player: Symbol,
        sample: super::super::PlanningRolloutSample<'_>,
        worlds: usize,
        bots: &[Symbol],
        budget: &ExecutionBudget<'_>,
    ) -> Result<Vec<Self>> {
        budget.check(0)?;
        let seeds = (0..worlds)
            .map(|index| {
                sample
                    .seed
                    .wrapping_add((index as u64).wrapping_mul(0x9e3779b97f4a7c15))
            })
            .collect::<Vec<_>>();
        let samples = self.resolution_samples(player, sample, &seeds, bots)?;
        samples
            .into_iter()
            .zip(seeds)
            .map(|(hands, seed)| {
                budget.check(0)?;
                self.with_installed_resolution_samples(player, hands, Some(seed), |worker| Ok(worker.clone()))
            })
            .collect()
    }

    pub(in crate::interpreter) fn pending_hidden_hand_reader(
        &self,
        viewer: Symbol,
        known: &[Symbol],
    ) -> Result<Option<Symbol>> {
        if self.last_request.is_some() {
            return Ok(None);
        }
        match (&self.objects, self.stack.last()) {
            (Some(objects), Some(frame)) => {
                self.hidden_hand_reader(objects, frame.instruction, frame.object, viewer, known)
            }
            _ => Ok(None),
        }
    }

    pub(super) fn hidden_hand_reader(
        &self,
        objects: &BoundObjects,
        instruction: usize,
        step: Option<ObjectId>,
        viewer: Symbol,
        known: &[Symbol],
    ) -> Result<Option<Symbol>> {
        let Some(step) = step else { return Ok(None) };
        let instruction = &self.steps[instruction].instruction;
        let key_field = match instruction {
            Instruction::LiveReaction => Field::TargetPlayerKey,
            Instruction::LiveForceDiscard | Instruction::LiveColorDiscard
            | Instruction::LiveDiscardOrDefeat => Field::VictimKey,
            _ => return Ok(None),
        };
        if objects.arena.field(step, objects.pending)
            .is_some_and(|input| objects.arena.truthy(&input))
        {
            return Ok(None);
        }
        let direct = if matches!(instruction, Instruction::LiveForceDiscard) {
            self.truthy_symbol_bound(objects, objects.arena.field(step, objects.phases.k(Field::Victim)))?
        } else {
            None
        };
        let target = if direct.is_some() {
            direct
        } else if let Some(key) = self.truthy_symbol_bound(
            objects, objects.arena.field(step, objects.phases.k(key_field)),
        )? {
            self.truthy_symbol_bound(objects, objects.arena.context_value(key))?
        } else {
            None
        };
        let viewer_team = self.world.hero(viewer).map(|row| row.team);
        Ok(target.and_then(|id| self.world.hero(id))
            .filter(|hero| hero.id != viewer
                && !(Some(hero.team) == viewer_team && known.contains(&hero.id)))
            .map(|hero| hero.id))
    }

    pub(in crate::interpreter) fn with_hidden_hand_hypothesis<T>(
        &mut self,
        hero: Symbol,
        hand: &super::super::SampledHand,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.with_rollback(|worker| {
            let row = worker.world.hero(hero).ok_or_else(|| unsupported("hidden hand owner missing"))?;
            let protected = [row.current, row.extra];
            worker.with_live_objects(|session, objects| {
                session.replace_sampled_hand(objects, hero, hand, protected)?;
                session.sync_card_collections_bound(objects)?;
                session.world.refresh_card_auras();
                objects.readonly.take();
                Ok(())
            })?;
            operation(worker)
        })
    }

    pub(super) fn search_step_reads_concealed_state(
        &self,
        objects: &BoundObjects,
        instruction: usize,
        step: Option<ObjectId>,
        viewer: Symbol,
    ) -> Result<bool> {
        use crate::turn::FlowStep;
        Ok(match &self.steps[instruction].instruction {
            // Upgrade choices are simultaneous and the ordinary hand sampler
            // cannot reconstruct other players' pending selections.
            Instruction::Flow(FlowStep::Upgrades) => true,
            Instruction::LiveTriggerMines => {
                let step = step.ok_or_else(|| unsupported("live mine trigger missing"))?;
                self.trigger_reads_concealed_mine_bound(objects, step, viewer)?
            }
            Instruction::LiveCardGuess { .. } => !step.is_some_and(|step| {
                self.sampled_guess_steps.is_some_and(|steps| steps.contains(&step))
            }),
            Instruction::LiveCardSelect if self.has_concealed_card_in_public_zone() => {
                use crate::card_selection::Container;
                let step = step.ok_or_else(|| unsupported("live card selector missing"))?;
                let spec = self.card_spec_bound(objects, step)?;
                // Ordinary selectors exclude face-down played and discarded
                // cards before applying filters. A deck selector may still
                // read their printed identity through the deck collection.
                spec.facedown || spec.containers.iter().any(|c| matches!(c, Container::Deck))
            }
            Instruction::LiveDiscard if self.has_concealed_card_in_public_zone() => {
                let step = step.ok_or_else(|| unsupported("live discard missing"))?;
                !self.discard_targets_hand_bound(objects, step)?
            }
            Instruction::LiveCardSwap if self.has_concealed_card_in_public_zone() => {
                let step = step.ok_or_else(|| unsupported("live card swap missing"))?;
                !self.card_swap_is_current_to_hand_bound(objects, step)?
            }
            Instruction::LiveResolvedCardSwap
            | Instruction::LiveDeckCardSwap | Instruction::LiveRetrieve
            | Instruction::LiveColorDiscard => {
                self.has_concealed_card_in_public_zone()
            }
            _ => false,
        })
    }

    fn has_concealed_card_in_public_zone(&self) -> bool {
        self.world.heroes.iter().any(|hero| {
            hero.played.iter().flatten().chain(&hero.discard)
                .any(|id| self.world.card(*id).is_some_and(|card| card.facedown))
        })
    }

    fn resolution_samples(
        &self,
        player: Symbol,
        sample: super::super::PlanningRolloutSample<'_>,
        seeds: &[u64],
        bots: &[Symbol],
    ) -> Result<Vec<Vec<PlanningHandSample>>> {
        let cleanup_battle = self.planning.phase == Phase::Cleanup
            && self.stack.last().is_some_and(|frame| {
                matches!(
                    self.steps[frame.instruction].instruction,
                    Instruction::Battle(_)
                )
            });
        if !(self.planning.phase == Phase::Resolution || cleanup_battle)
            || self.last_request.as_ref().is_none_or(|request| {
                !self.resolution_request_addresses(request, player)
            })
        {
            return Err(unsupported("resolution sampling requires a live resolution prompt for the player"));
        }
        if let (Some(objects), Some(frame)) = (&self.objects, self.stack.last())
            && !matches!(self.steps[frame.instruction].instruction, Instruction::LiveCardGuess { reveal: false })
            && self.search_step_reads_concealed_state(
                objects, frame.instruction, frame.object, player,
            )?
        {
            return Err(unsupported("resolution sampling cannot reconstruct this concealed choice"));
        }
        let observation = self.player_observation(player)?;
        self.validate_sampled_resolution_effects()?;
        self.validate_sampled_effect_cards(&observation)?;
        observation.validate_upgrade_evidence(sample.upgrade_checkpoints)?;
        let mut samples = (0..seeds.len()).map(|_| Vec::new()).collect::<Vec<_>>();
        let team = self.world.hero(player).map(|hero| hero.team);
        for (index, hero) in self.world.heroes.iter().enumerate() {
            if hero.id != player && Some(hero.team) == team && bots.contains(&hero.id) {
                continue;
            }
            if hero.id == player {
                for (world, seed) in samples.iter_mut().zip(seeds) {
                    if let Some(sample) = self.preserved_gold_sample(&observation, index, *seed)? {
                        world.push(sample);
                    }
                }
                continue;
            }
            let protected = [hero.current, hero.extra];
            // Prepared spells are tracked separately from ordinary cards and
            // are reconstructible from the roster and public casting history.
            if protected.iter().flatten()
                .any(|id| self.world.card(*id).is_none_or(|card| card.facedown))
            {
                return Err(unsupported(
                    "resolution sampling requires revealed ordinary current cards",
                ));
            }
            let hands = observation
                .sample_resolution_hands(
                    &observation.heroes[index].id,
                    seeds.iter().map(|seed| seed.wrapping_add(index as u64)),
                    sample
                        .upgrade_checkpoints
                        .and_then(|rows| rows.get(&observation.heroes[index].id))
                        .map_or(&[], Vec::as_slice),
                )?
                .ok_or_else(|| unsupported(format!(
                    "no consistent ordinary resolution hand for {}", observation.heroes[index].name
                )))?;
            for (world, hand) in samples.iter_mut().zip(hands) {
                world.push(PlanningHandSample {
                    hero: hero.id,
                    update: SampledCards::Hand(hand),
                    fixed: protected,
                });
            }
        }
        Ok(samples)
    }

    pub(in crate::interpreter) fn movement_root_actor(&self) -> Option<Symbol> {
        let request = self.last_request.as_ref()?;
        if !matches!(request.choices, RequestChoices::Hexes(_)) || request.discard {
            return None;
        }
        let objects = self.objects.as_ref()?;
        let mut stack = self.stack.iter().rev();
        let selector = stack.next()?;
        if !matches!(self.steps[selector.instruction].instruction, Instruction::LiveHexSelect) {
            return None;
        }
        let next = stack.next()?;
        let movement = match self.steps[next.instruction].instruction {
            Instruction::LiveMove => true,
            Instruction::LiveTravel { sequence: false } => false,
            _ => return None,
        };
        let step = next.object?;
        if self.passive_text_bound(objects, selector.object?, Field::Output).ok()?
            != self.passive_text_bound(objects, step, Field::DestinationKey).ok()?
        {
            return None;
        }
        let text = |field| self.passive_optional_text_bound(objects, step, field).ok().flatten();
        let unit = text(Field::UnitId).filter(|id| !self.symbols.name(*id).is_empty())
            .or_else(|| {
                if !movement {
                    return None;
                }
                let key = text(Field::UnitKey)?;
                objects.symbol_from_context(key, &self.symbols).ok().flatten()
            })
            .or(self.planning.actor)?;
        Some(self.world.resolved_actor(unit))
    }

    fn with_installed_resolution_samples<T>(
        &mut self,
        player: Symbol,
        samples: Vec<PlanningHandSample>,
        seed: Option<u64>,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.with_rollback(|worker| {
            let root_choices = worker.resolution_choices(player)?;
            let request = worker.last_request.clone()
                .ok_or_else(|| unsupported("sampled resolution root request missing"))?;
            worker.with_live_objects(|session, objects| {
                for sample in samples {
                    match sample.update {
                        SampledCards::Hand(hand) => session.replace_sampled_hand(
                            objects, sample.hero, &hand, sample.fixed,
                        )?,
                        SampledCards::ConcealedGold(slot, gold) => {
                            session.bind_sampled_gold(objects, sample.hero, slot, &gold)?;
                        }
                    }
                }
                session.sync_card_collections_bound(objects)?;
                session.world.refresh_card_auras();
                if let Some(seed) = seed {
                    session.bind_sampled_guess(objects, seed)?;
                }
                objects.readonly.take();
                Ok(())
            })?;
            // A cached request survives hand replacement. Regenerate it to
            // validate the actual rules, including filters on public prompts.
            worker.with_rollback(|probe| {
                probe.advance_objects(&ExecutionBudget::default(), Some(1), false)?;
                let generated = probe.last_request.as_ref()
                    .ok_or_else(|| unsupported("sampled resolution lost the root request"))?;
                if generated.id != request.id
                    || generated.owner != request.owner
                    || generated.controlled != request.controlled
                    || generated.prompt != request.prompt
                    || generated.can_skip != request.can_skip
                    || generated.discard != request.discard
                {
                    return Err(unsupported("sampled resolution changed the root request metadata"));
                }
                if probe.resolution_choices(player)? != root_choices {
                    return Err(unsupported("sampled resolution changed the root choices"));
                }
                Ok(())
            })?;
            operation(worker)
        })
    }

    fn planning_samples(
        &self,
        player: Symbol,
        seed: u64,
        checkpoints: Option<&BTreeMap<String, Vec<UpgradeCheckpoint>>>,
        preserved: &[Symbol],
        known: &[Symbol],
    ) -> Result<Vec<PlanningHandSample>> {
        let mut observation = self.player_observation(player)?;
        observation.validate_upgrade_evidence(checkpoints)?;
        if self.planning.phase != Phase::Planning {
            return Err(unsupported("sampled worlds require planning"));
        }
        self.validate_sampled_effect_cards(&observation)?;
        let mut samples = Vec::new();
        for (index, hero) in self.world.heroes.iter().enumerate() {
            if hero.id == player || preserved.contains(&hero.id) {
                samples.extend(self.preserved_gold_sample(&observation, index, seed)?);
                continue;
            }
            let public = &mut observation.heroes[index];
            if !public.revealed_current.is_empty() {
                return Err(unsupported(
                    "sampled planning requires no publicly revealed current cards",
                ));
            }
            // Prepared spells are public with perfect memory. Keep the
            // spellbook and sample only the ordinary hand.
            let fixed = if known.contains(&hero.id) {
                let (first, second) = self.observed_planning_cards(hero.id)?;
                [first, second]
            } else {
                [None, None]
            };
            for card in fixed.iter().flatten() {
                public.hidden_commitments -= 1;
                let row = self
                    .world
                    .card(*card)
                    .ok_or_else(|| unsupported("known commitment card is missing"))?;
                let printed = self.world.definitions.cards[row.definition].printed_id;
                public
                    .revealed_current
                    .push(self.symbols.name(printed).to_owned());
            }
            let id = public.id.clone();
            let hand = observation
                .sample_planning_hand_with_checkpoints(
                    &id,
                    seed.wrapping_add(index as u64),
                    checkpoints
                        .and_then(|rows| rows.get(&id))
                        .map_or(&[], Vec::as_slice),
                )?
                .ok_or_else(|| unsupported(format!(
                    "no consistent ordinary planning hand for {}", self.symbols.name(hero.id)
                )))?;
            samples.push(PlanningHandSample {
                hero: hero.id,
                update: SampledCards::Hand(hand),
                fixed,
            });
        }
        Ok(samples)
    }

    fn preserved_gold_sample(
        &self,
        observation: &super::super::PlayerObservation,
        index: usize,
        seed: u64,
    ) -> Result<Option<PlanningHandSample>> {
        Ok(super::super::loadout::sample_preserved_gold(
            &observation.heroes[index],
            seed.wrapping_add(index as u64),
        )?
        .map(|(slot, gold)| PlanningHandSample {
            hero: self.world.heroes[index].id,
            update: SampledCards::ConcealedGold(slot, gold),
            fixed: [None, None],
        }))
    }

    fn validate_sampled_resolution_effects(&self) -> Result<()> {
        use crate::effect_rules::Kind;

        let effects = &self.world.effects.instances;
        if effects.is_empty() {
            return Ok(());
        }
        let objects = self.objects.as_ref().ok_or_else(|| {
            unsupported("sampled resolution requires live effect bindings")
        })?;
        let nodes = objects.effect_nodes()?;
        if nodes.len() != effects.len() {
            return Err(unsupported("effect collection binding drift"));
        }
        for (effect, node) in effects.iter().zip(nodes) {
            let behavior = &effect.definition.behavior;
            let kind = self.symbols.name(effect.definition.kind);
            let action_restriction = kind == "target_prevention"
                && !behavior.restrictions.is_empty();
            // These round markers record performed cards. Source-card
            // visibility and live finishing steps are checked separately.
            let double_items = behavior.kind == Kind::DoubleItems
                && effect.card.is_some_and(|card| {
                    matches!(self.symbols.name(card), "inner_strength" | "perfect_self")
                });
            let basic_bonus = behavior.kind == Kind::BasicBonus
                && effect.card.is_some_and(|card| {
                    matches!(self.symbols.name(card),
                        "broom_for_improvement" | "broomstick_beatdown" | "this_is_my_broomstick")
                });
            let round_marker = (matches!(kind, "enraged" | "equilibrium") || double_items || basic_bonus)
                && effect.duration == crate::effects::Duration::ThisRound
                && effect.token.is_none()
                && effect.granted_passive.is_none();
            // Wuk's Dominance uses public minion positions at battle time.
            let dominance = behavior.kind == Kind::BattleExclusion
                && effect.duration == crate::effects::Duration::Passive
                && effect.token.is_none()
                && effect.granted_passive.is_none()
                && effect.scope_origin == Some(effect.source)
                && effect.card.is_some_and(|card| {
                    matches!(self.symbols.name(card), "claim_dominance" | "assert_dominance")
                });
            // Reverse Time changes public initiative ordering next turn.
            let reverse_time = kind == "reversed_initiative"
                && effect.duration == crate::effects::Duration::NextTurn
                && effect.token.is_none()
                && effect.granted_passive.is_none()
                && effect.card.is_some_and(|card| self.symbols.name(card) == "reverse_time");
            let public_color_trigger = kind == "after_cards_played_trigger"
                && effect.duration == crate::effects::Duration::NextTurn
                && effect.token.is_none()
                && effect.granted_passive.is_none()
                && effect.scope_origin == Some(effect.source)
                && effect.card.is_some_and(|card| {
                    matches!(
                        self.symbols.name(card),
                        "imbue_doubt" | "time_to_reconsider" | "an_illusion_of_choice"
                    )
                });
            let journey = (kind == "delayed_trigger"
                && effect.duration == crate::effects::Duration::ThisTurn
                && effect.token.is_none()
                && effect.granted_passive.is_none())
                .then(|| effect.card.map(|card| self.symbols.name(card)))
                .flatten()
                .filter(|card| matches!(*card,
                    "unexpected_journey" | "there_and_back_again" | "safe_travels"));
            let another_one = kind == "delayed_trigger"
                && effect.duration == crate::effects::Duration::ThisTurn
                && effect.token.is_none()
                && effect.granted_passive.is_none()
                && effect.card.is_some_and(|card| self.symbols.name(card) == "another_one");
            let sentinel = (kind == "delayed_trigger"
                && effect.duration == crate::effects::Duration::ThisTurn
                && effect.token.is_none()
                && effect.granted_passive.is_none())
                .then(|| effect.card.map(|card| self.symbols.name(card)))
                .flatten()
                .filter(|card| matches!(*card, "warning_shot" | "treetop_sentinel"));
            let glitch_cleanup = kind == "delayed_trigger"
                && effect.duration == crate::effects::Duration::ThisTurn
                && effect.token.is_none()
                && effect.granted_passive.is_none()
                && effect.card.is_some_and(|card| {
                    matches!(self.symbols.name(card), "flashback" | "unstable_timeline")
                });
            let round_minion_defeat = kind == "delayed_trigger"
                && effect.duration == crate::effects::Duration::ThisRound
                && effect.token.is_none()
                && effect.granted_passive.is_none()
                && effect.card.is_some_and(|card| {
                    matches!(self.symbols.name(card), "final_embrace" | "constrict")
                });
            let grenade = kind == "delayed_trigger"
                && effect.duration == crate::effects::Duration::ThisTurn
                && effect.token.is_none()
                && effect.granted_passive.is_none()
                && effect.card.is_some_and(|card| {
                    matches!(self.symbols.name(card), "death_grenade" | "holy_death_grenade")
                });
            // Pre-discard triggers use the public board and hand count.
            // Any resulting discard choice runs against the sampled hand.
            let public = matches!(
                behavior.kind,
                Kind::PreMovement
                    | Kind::PreDiscard
                    | Kind::AttackImmunity
                    | Kind::ActionImmunity
                    | Kind::PlacementPrevention
                    | Kind::MinionProtection
                    | Kind::MovementZone
                    | Kind::LosBlocker
                    | Kind::AreaStat
                    | Kind::MovementAura
                    | Kind::StaticBarrier
                    | Kind::Petrify
            ) || action_restriction || round_marker || dominance || reverse_time || public_color_trigger || sentinel.is_some() || journey.is_some() || another_one || glitch_cleanup || grenade || round_minion_defeat;
            if !public {
                let card = effect.card.map_or("cardless", |id| self.symbols.name(id));
                return Err(unsupported(format!(
                    "resolution sampling does not support {:?} effect {kind} from {card}",
                    behavior.kind,
                )));
            }
            // The dense projection omits finishing programs for arena sessions.
            // These programs can still retain references to hidden objects.
            let finishing = objects.finishing_list(node)?;
            if round_minion_defeat {
                self.validate_sampled_round_minion_defeat(objects, finishing, effect.source)?;
                continue;
            }
            if grenade {
                self.validate_sampled_grenade(objects, finishing)?;
                continue;
            }
            if glitch_cleanup {
                self.validate_sampled_glitch_cleanup(objects, finishing)?;
                continue;
            }
            if another_one {
                self.validate_sampled_retrieval(objects, finishing)?;
                continue;
            }
            if let Some(card) = journey {
                self.validate_sampled_journey(
                    objects, finishing, effect.source, card == "safe_travels",
                )?;
                continue;
            }
            if public_color_trigger || sentinel.is_some() {
                // The stored steps target public units and then use the
                // sampled hand for any discard. Require the expected program.
                let steps = objects.steps_in_list(finishing)?;
                let valid_count = if public_color_trigger {
                    matches!(steps.len(), 2 | 4)
                } else {
                    steps.len() == 2
                };
                if valid_count && steps.iter().enumerate().all(|(index, step)| {
                    objects.program(*step).is_some_and(|instruction| {
                        let instruction = &self.steps[instruction].instruction;
                        if index % 2 == 0 {
                            matches!(instruction, Instruction::LiveUnitSelect)
                        } else {
                            match sentinel {
                                Some("warning_shot") => matches!(instruction, Instruction::LiveForceDiscard),
                                Some("treetop_sentinel") => matches!(instruction, Instruction::LiveDiscardOrDefeat),
                                _ => matches!(instruction, Instruction::LiveColorDiscard),
                            }
                        }
                    }) && matches!(
                        objects.arena.field_ref(*step, objects.pending),
                        None | Some(ObjectValue::Atom(Datum::Null))
                    )
                }) {
                    continue;
                }
                return Err(unsupported("unexpected deferred discard program"));
            }
            if objects.arena.sequence(finishing).is_some_and(|steps| !steps.is_empty()) {
                return Err(unsupported(format!(
                    "resolution sampling does not support finishing steps on {kind}",
                )));
            }
        }
        Ok(())
    }

    fn validate_sampled_round_minion_defeat(
        &self,
        objects: &BoundObjects,
        finishing: ObjectId,
        source: Symbol,
    ) -> Result<()> {
        let invalid = || unsupported("unexpected round-end minion defeat program");
        let steps = objects.steps_in_list(finishing)?;
        let [select, defeat] = steps.as_slice() else {
            return Err(invalid());
        };
        let program = |step| objects.program(step).map(|id| &self.steps[id].instruction);
        let text = |step, field| self.passive_optional_text_bound(objects, step, field);
        let output = self.passive_text_bound(objects, *select, Field::Output)?;
        if !matches!(program(*select), Some(Instruction::LiveUnitSelect))
            || !matches!(program(*defeat), Some(Instruction::LiveDefeat))
            || text(*select, Field::TargetType)?.is_none_or(|id| self.symbols.name(id) != "UNIT")
            || text(*defeat, Field::Victim)?.is_some()
            || text(*defeat, Field::VictimKey)? != Some(output)
            || text(*defeat, Field::ActiveIf)? != Some(output)
            || text(*defeat, Field::KillerId)? != Some(source)
        {
            return Err(invalid());
        }
        for &step in &steps {
            if objects.token(step)?.is_some() || !matches!(
                objects.arena.field_ref(step, objects.pending),
                None | Some(ObjectValue::Atom(Datum::Null))
            ) {
                return Err(invalid());
            }
        }
        let filters = self.unit_filter_list_bound(objects, *select, Field::Filters)?;
        let [ObjectValue::Object(team), ObjectValue::Object(minions), ObjectValue::Object(range)] =
            filters.as_slice()
        else {
            return Err(invalid());
        };
        for (filter, expected) in [
            (*team, "team_filter"), (*minions, "minion_type_filter"), (*range, "range_filter"),
        ] {
            if self.symbols.name(self.passive_text_bound(objects, filter, Field::FilterType)?) != expected {
                return Err(invalid());
            }
        }
        for field in [Field::OriginKey, Field::OriginHexKey, Field::MinRangeKey, Field::MaxRangeKey] {
            if text(*range, field)?.is_some() {
                return Err(invalid());
            }
        }
        Ok(())
    }

    fn validate_sampled_grenade(&self, objects: &BoundObjects, finishing: ObjectId) -> Result<()> {
        let invalid = || unsupported("unexpected grenade finishing program");
        let field = |object, name| objects.arena.field_ref(object, self.symbols.lookup(name)?);
        let text = |object, name, expected| {
            matches!(field(object, name), Some(ObjectValue::Atom(Datum::Text(id)))
                if self.symbols.name(*id) == expected)
        };
        let empty = |object, name| {
            matches!(field(object, name), None | Some(ObjectValue::Atom(Datum::Null)))
        };
        let sequence = |object, name| match field(object, name) {
            Some(ObjectValue::Object(list)) => objects.arena.sequence(*list),
            _ => None,
        };
        let program = |step| objects.program(step).map(|index| &self.steps[index].instruction);
        let steps = objects.steps_in_list(finishing)?;
        let [token, targets, repeat, remove] = steps.as_slice() else {
            return Err(invalid());
        };
        let Some([ObjectValue::Object(token_filter)]) = sequence(*token, "filters") else {
            return Err(invalid());
        };
        let Some([ObjectValue::Object(adjacent), ObjectValue::Object(enemy), ObjectValue::Object(hero)]) =
            sequence(*targets, "filters")
        else {
            return Err(invalid());
        };
        let Some([ObjectValue::Object(discard)]) = sequence(*repeat, "steps_template") else {
            return Err(invalid());
        };
        // The selectors read public board geometry and ownership. The
        // downstream discard resolves against the sampled victim's hand.
        if matches!(program(*token), Some(Instruction::LiveUnitSelect))
            && matches!(program(*targets), Some(Instruction::LiveMultiSelect))
            && matches!(program(*repeat), Some(Instruction::ForEach { .. }))
            && matches!(program(*discard), Some(Instruction::LiveDiscardOrDefeat))
            && matches!(program(*remove), Some(Instruction::LiveTokenRemove))
            && steps.iter().chain([discard]).all(|&step| {
                let current_request = self.stack.last().is_some_and(|frame| {
                    frame.object == Some(step)
                        && self.last_request.as_ref().is_some_and(|request| {
                            frame.request == Some(request.id)
                                && objects.token(step).ok().flatten() == Some(request.id)
                        })
                });
                empty(step, "pending_input")
                    && (empty(step, "pending_request_id") || current_request)
                    && empty(step, "skip_if_key")
                    && (step == *targets || empty(step, "active_if_key"))
            })
            && text(*token, "target_type", "UNIT_OR_TOKEN")
            && text(*token, "output_key", "grenade_resolve_id")
            && text(*token_filter, "type", "token_type_filter")
            && text(*token_filter, "token_type", "grenade")
            && text(*targets, "target_type", "UNIT")
            && text(*targets, "active_if_key", "grenade_resolve_id")
            && text(*targets, "output_key", "grenade_victims")
            && text(*adjacent, "type", "adjacency_to_context_filter")
            && text(*adjacent, "target_key", "grenade_resolve_id")
            && text(*enemy, "type", "team_filter")
            && text(*enemy, "relation", "ENEMY")
            && text(*hero, "type", "unit_type_filter")
            && text(*hero, "unit_type", "HERO")
            && text(*repeat, "list_key", "grenade_victims")
            && text(*repeat, "item_key", "current_victim")
            && text(*discard, "victim_key", "current_victim")
            && empty(*discard, "victim_id")
            && text(*remove, "token_key", "grenade_resolve_id")
            && empty(*remove, "token_id")
        {
            Ok(())
        } else {
            Err(invalid())
        }
    }

    fn validate_sampled_glitch_cleanup(
        &self,
        objects: &BoundObjects,
        finishing: ObjectId,
    ) -> Result<()> {
        let invalid = || unsupported("unexpected Glitch cleanup program");
        let field = |object, name| {
            objects.arena.field_ref(object, self.symbols.lookup(name)?)
        };
        let text = |object, name, expected| {
            matches!(field(object, name), Some(ObjectValue::Atom(Datum::Text(id)))
                if self.symbols.name(*id) == expected)
        };
        let empty = |object, name| {
            matches!(field(object, name), None | Some(ObjectValue::Atom(Datum::Null)))
        };
        let fresh = |step| {
            ["pending_input", "pending_request_id", "active_if_key", "skip_if_key"]
                .into_iter().all(|name| empty(step, name))
        };
        let program = |step| objects.program(step).map(|index| &self.steps[index].instruction);
        let steps = objects.steps_in_list(finishing)?;
        let [collect, repeat] = steps.as_slice() else { return Err(invalid()); };
        let Some(ObjectValue::Object(filters)) = field(*collect, "filters") else {
            return Err(invalid());
        };
        let Some([ObjectValue::Object(unit), ObjectValue::Object(token)]) =
            objects.arena.sequence(*filters)
        else {
            return Err(invalid());
        };
        let Some(ObjectValue::Object(templates)) = field(*repeat, "steps_template") else {
            return Err(invalid());
        };
        let children = objects.steps_in_list(*templates)?;
        let [remove] = children.as_slice() else { return Err(invalid()); };
        if matches!(program(*collect), Some(Instruction::LiveQuery { collect: true }))
            && matches!(program(*repeat), Some(Instruction::ForEach { .. }))
            && matches!(program(*remove), Some(Instruction::LiveTokenRemove))
            && fresh(*collect) && fresh(*repeat) && fresh(*remove)
            && text(*collect, "target_type", "UNIT_OR_TOKEN")
            && text(*unit, "type", "unit_type_filter") && text(*unit, "unit_type", "TOKEN")
            && text(*token, "type", "token_type_filter") && text(*token, "token_type", "glitch")
            && text(*collect, "output_key", "_glitch_cleanup_ids")
            && text(*repeat, "list_key", "_glitch_cleanup_ids")
            && text(*repeat, "item_key", "_glitch_cleanup_id")
            && text(*remove, "token_key", "_glitch_cleanup_id")
            && empty(*remove, "token_id")
        {
            Ok(())
        } else {
            Err(invalid())
        }
    }

    fn validate_sampled_retrieval(
        &self,
        objects: &BoundObjects,
        finishing: ObjectId,
    ) -> Result<()> {
        use crate::card_selection::Container;

        let steps = objects.steps_in_list(finishing)?;
        let program = |step| objects.program(step).map(|index| &self.steps[index].instruction);
        let [target, their_card, their_retrieve, own_card, own_retrieve] = steps.as_slice() else {
            return Err(unsupported("unexpected Another One retrieval program"));
        };
        if !matches!(program(*target), Some(Instruction::LiveUnitSelect))
            || !matches!(program(*their_retrieve), Some(Instruction::LiveRetrieve))
            || !matches!(program(*own_retrieve), Some(Instruction::LiveRetrieve))
            || steps.iter().any(|step| !matches!(
                objects.arena.field_ref(*step, objects.pending),
                None | Some(ObjectValue::Atom(Datum::Null))
            ))
        {
            return Err(unsupported("unexpected Another One retrieval program"));
        }
        for select in [*their_card, *own_card] {
            if !matches!(program(select), Some(Instruction::LiveCardSelect)) {
                return Err(unsupported("Another One requires a discard selector"));
            }
            let spec = self.card_spec_bound(objects, select)?;
            if !matches!(spec.containers.as_slice(), [Container::Discard]) || spec.facedown {
                return Err(unsupported("Another One requires a public discard selector"));
            }
        }
        Ok(())
    }

    fn validate_sampled_journey(
        &self,
        objects: &BoundObjects,
        finishing: ObjectId,
        source: Symbol,
        move_after: bool,
    ) -> Result<()> {
        let steps = objects.steps_in_list(finishing)?;
        let program = |step| {
            objects.program(step).map(|index| &self.steps[index].instruction)
        };
        let swap = match steps.as_slice() {
            [swap] if !move_after => *swap,
            [swap, select, movement] if move_after
                && matches!(program(*select), Some(Instruction::LiveHexSelect))
                && matches!(program(*movement), Some(Instruction::LiveMove))
                && self.passive_optional_text_bound(objects, *movement, Field::UnitId)?
                    == Some(source) => *swap,
            _ => return Err(unsupported("unexpected Journey return program")),
        };
        let target = self.passive_optional_text_bound(objects, swap, Field::UnitBId)?;
        if !matches!(program(swap), Some(Instruction::LiveSwap))
            || self.passive_optional_text_bound(objects, swap, Field::UnitAId)? != Some(source)
            || target.is_none_or(|id| id == source || self.world.hero(id).is_none())
            || steps.iter().any(|step| !matches!(
                objects.arena.field_ref(*step, objects.pending),
                None | Some(ObjectValue::Atom(Datum::Null))
            ))
        {
            return Err(unsupported("Journey return requires public hero bindings and fresh steps"));
        }
        Ok(())
    }

    fn validate_sampled_effect_cards(
        &self,
        observation: &super::super::PlayerObservation,
    ) -> Result<()> {
        for (index, effect) in self.world.effects.instances.iter().enumerate() {
            if effect.card.is_none()
                && effect.linked_card.is_none()
                && effect.duration == crate::effects::Duration::Passive
                && let Some(token) = effect.token
                && effect.scope_origin == Some(token)
                && let Some(kind) = self.world.entity(token).and_then(|entity| entity.token_type)
                && match (effect.definition.behavior.kind, self.symbols.name(kind)) {
                    (crate::effect_rules::Kind::LosBlocker, "smoke_bomb")
                    | (crate::effect_rules::Kind::AreaStat, "ice" | "barrier") => true,
                    (crate::effect_rules::Kind::MinionProtection, "totem") => {
                        effect.definition.behavior.sacrifice
                            && effect.definition.behavior.discard_colors.is_empty()
                    }
                    _ => false,
                }
                && let Some(objects) = &self.objects
                && self.token_is_public_bound(objects, token)?
            {
                // Token anchors are public during planning as well. Unlike
                // the resolution root, a sampled planning root has not checked
                // the live finishing programs, so verify the token row here.
                let nodes = objects.effect_nodes()?;
                if nodes.len() != self.world.effects.instances.len() {
                    return Err(unsupported("effect collection binding drift"));
                }
                let finishing = objects.finishing_list(nodes[index])?;
                if objects.arena.sequence(finishing).is_some_and(|steps| steps.is_empty()) {
                    continue;
                }
                return Err(unsupported("sampled worlds cannot preserve token effect finishing steps"));
            }
            let public = self.world.hero(effect.source).and_then(|hero| {
                observation
                    .heroes
                    .iter()
                    .find(|row| row.id == self.symbols.name(hero.id))
            });
            let is_known = |card| {
                let id = self.symbols.name(card);
                public.is_some_and(|hero| {
                    // Resolution sampling preserves both revealed current
                    // slots as fixed physical cards.
                    hero.known_spells.iter().any(|spell| spell == id)
                        || (matches!(observation.phase, Phase::Resolution | Phase::Cleanup)
                        && (hero.revealed_current.iter().any(|current| current == id)
                            || (hero.id == observation.private.hero
                                && observation.private.hand.iter()
                                    .chain(&observation.private.current)
                                    .chain(&observation.private.extra)
                                    .any(|known| known == id))))
                        || hero.played.iter().chain(&hero.discarded).any(|entry| {
                            matches!(entry, super::super::PublicCard::Revealed(revealed) if revealed == id)
                        })
                })
            };
            let unknown = match effect.card {
                None => Some("source card missing"),
                Some(card) if !is_known(card) => Some("source card is outside known zones"),
                _ if effect.linked_card.is_some_and(|card| !is_known(card)) => {
                    Some("linked card is outside known zones")
                }
                _ => None,
            };
            if let Some(reason) = unknown {
                return Err(unsupported(format!(
                    "sampled worlds cannot preserve {} effect: {reason}",
                    self.symbols.name(effect.definition.kind),
                )));
            }
        }
        Ok(())
    }

    fn install_planning_samples(&mut self, samples: Vec<PlanningHandSample>) -> Result<()> {
        self.with_live_objects(|session, objects| {
            for sample in samples {
                match sample.update {
                    SampledCards::Hand(hand) => {
                        session.install_planning_hand(objects, sample.hero, &hand, sample.fixed)?;
                    }
                    SampledCards::ConcealedGold(slot, gold) => {
                        session.bind_sampled_gold(objects, sample.hero, slot, &gold)?;
                    }
                }
            }
            session.sync_card_collections_bound(objects)?;
            session.world.refresh_card_auras();
            objects.readonly.take();
            Ok(())
        })
    }

    fn install_planning_hand(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
        sample: &super::super::SampledHand,
        fixed: [Option<CardId>; 2],
    ) -> Result<()> {
        if fixed[0].is_none() {
            let pending = objects.phases.root(&objects.arena, Field::Pending)?;
            while matches!(
                objects.arena.field(pending, hero),
                Some(ObjectValue::Atom(Datum::Instance(_)))
            ) {
                self.uncommit_card_bound(objects, hero)?;
            }
            if objects.arena.field(pending, hero).is_some() {
                objects
                    .arena
                    .apply(&crate::objects::Mutation::Delete {
                        target: crate::objects::Target::Object(pending),
                        key: hero,
                    })
                    .map_err(unsupported)?;
            }
        }
        self.replace_sampled_hand(objects, hero, sample, fixed)
    }

    fn bind_sampled_gold(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
        slot: HiddenGoldSlot,
        printed: &str,
    ) -> Result<()> {
        use crate::world::CardLifecycle;
        let row = self.world.hero(hero).ok_or_else(|| unsupported("sampled hero missing"))?;
        let (field, index, lifecycle, old) = match slot {
            HiddenGoldSlot::Played(index) => (
                Field::Played, index, CardLifecycle::Resolved, row.played.get(index).copied().flatten(),
            ),
            HiddenGoldSlot::Discard(index) => (
                Field::Discard, index, CardLifecycle::Discard, row.discard.get(index).copied(),
            ),
        };
        let old = old.ok_or_else(|| unsupported("sampled gold slot missing"))?;
        let previous = self.world.card(old).ok_or_else(|| unsupported("sampled gold missing"))?;
        if !previous.facedown || previous.lifecycle != lifecycle
            || self.symbols.name(self.world.definitions.cards[previous.definition].color
                .ok_or_else(|| unsupported("played card color missing"))?) != "GOLD"
        {
            return Err(unsupported("sampled slot is not a facedown gold in the expected lifecycle"));
        }
        let played = previous.played_this_round;
        let old_printed = self.world.definitions.cards[previous.definition].printed_id;
        let chosen = row.deck.iter().copied().find(|id| {
            self.world.card(*id).is_some_and(|card| {
                self.symbols.name(self.world.definitions.cards[card.definition].printed_id) == printed
            })
        }).ok_or_else(|| unsupported("sampled gold absent from deck"))?;
        let chosen_card = self.world.card(chosen).unwrap();
        let chosen_printed = self.world.definitions.cards[chosen_card.definition].printed_id;
        if self.world.effects.instances.iter().any(|effect| {
            [effect.card, effect.linked_card].into_iter().flatten()
                .any(|card| card == old_printed || card == chosen_printed)
        }) {
            return Err(unsupported("sampled facedown gold has a live effect reference"));
        }
        if chosen == old {
            return Ok(());
        }
        if chosen_card.lifecycle != CardLifecycle::Deck {
            return Err(unsupported("sampled gold is not in the deck"));
        }
        let model = self.hero_model_bound(objects, hero)
            .ok_or_else(|| unsupported("sampled hero model missing"))?;
        let Some(ObjectValue::Object(list)) = objects.arena.field(model, objects.phases.k(field))
        else {
            return Err(unsupported("sampled gold collection missing"));
        };
        objects.arena.set_index(list, index, ObjectValue::Atom(Datum::Instance(chosen.0)))
            .map_err(unsupported)?;
        self.swap_card_flags_bound(objects, chosen, lifecycle, true, played)?;
        self.swap_card_flags_bound(objects, old, CardLifecycle::Deck, false, false)?;
        self.sync_card_collections_bound(objects)
    }

    /// Rebuild only the sampled hand and lifecycle rows. Cards in public
    /// zones and protected current slots retain their physical identity.
    fn replace_sampled_hand(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
        sample: &super::super::SampledHand,
        protected: [Option<CardId>; 2],
    ) -> Result<()> {
        use crate::world::CardLifecycle;
        if let Some((slot, printed)) = &sample.hidden_gold {
            self.bind_sampled_gold(objects, hero, *slot, printed)?;
        }
        let model = self
            .hero_model_bound(objects, hero)
            .ok_or_else(|| unsupported("sampled hero model missing"))?;
        let hand = self.planning_hand_bound(objects, model)?;
        let deck = self.hero_card_list_bound(objects, model, Field::Deck)?;
        let row = self
            .world
            .hero(hero)
            .ok_or_else(|| unsupported("sampled hero missing"))?;
        let public_cards = row
            .played
            .iter()
            .flatten()
            .chain(&row.discard)
            .copied()
            .chain(protected.into_iter().flatten())
            .collect::<Vec<_>>();
        if public_cards.iter().any(|card| !deck.contains(&Some(*card))) {
            return Err(unsupported(
                "sampled world has a foreign card in a public zone",
            ));
        }
        let mut selected = Vec::new();
        for card in deck.into_iter().flatten() {
            if public_cards.contains(&card) {
                continue;
            }
            let row = self
                .world
                .card(card)
                .ok_or_else(|| unsupported("sampled card missing"))?;
            let definition = &self.world.definitions.cards[row.definition];
            if definition.spell {
                continue;
            }
            let id = self.symbols.name(definition.printed_id);
            let (state, wire) = if sample.cards.iter().any(|value| value == id) {
                selected.push(card);
                (CardLifecycle::Hand, "HAND")
            } else if sample.loadout.item_card_ids.iter().any(|value| value == id) {
                (CardLifecycle::Item, "ITEM")
            } else if sample.retired.iter().any(|value| value == id) {
                (CardLifecycle::Retired, "RETIRED")
            } else {
                (CardLifecycle::Deck, "DECK")
            };
            let wire = self.choice_symbol(wire);
            objects.arena.symbols = self.symbols.clone();
            self.planning_card_scalars_bound(objects, card, false)?;
            let key = objects.phases.k(Field::CardState);
            objects
                .arena
                .set_physical_field(card.0, key, Datum::Text(wire))
                .map_err(unsupported)?;
            self.world.change_lifecycle(card, state);
            let row = self.world.card_mut(card).unwrap();
            row.facedown = false;
            row.played_this_round = false;
        }
        if selected.len() != sample.cards.len() {
            return Err(unsupported("sampled hand does not match the physical deck"));
        }
        objects.arena.clear_list(hand).map_err(unsupported)?;
        for card in selected {
            objects
                .arena
                .append(hand, ObjectValue::Atom(Datum::Instance(card.0)))
                .map_err(unsupported)?;
        }
        Ok(())
    }

    pub(in crate::interpreter) fn planning_choices_bound(
        &self,
        objects: &BoundObjects,
        hero: Symbol,
        limit: usize,
    ) -> Result<Vec<super::super::DecisionMove>> {
        use super::super::DecisionMove;
        if self.planning.phase != Phase::Planning
            || !objects.phases.heroes.iter().any(|(id, _)| *id == hero)
        {
            return Ok(Vec::new());
        }
        let model = self
            .hero_model_bound(objects, hero)
            .ok_or_else(|| unsupported("planning hero model missing"))?;
        let p = &objects.phases;
        let pending = p.root(&objects.arena, Field::Pending)?;
        let first = objects.arena.field(pending, hero);
        if let Some(first) = &first {
            let second = p.root(&objects.arena, Field::Second)?;
            let done = p.root(&objects.arena, Field::Done)?;
            if matches!(first, ObjectValue::Atom(Datum::Null))
                || !self.planning.two_cards(&self.world, hero)
                || objects.arena.field(second, hero).is_some()
                || objects
                    .arena
                    .sequence(done)
                    .ok_or_else(|| unsupported("planning done list missing"))?
                    .iter()
                    .any(|value| matches!(value, ObjectValue::Atom(Datum::Text(id)) if *id == hero))
            {
                return Ok(Vec::new());
            }
        }
        let hand = self.hero_card_values_bound(objects, model, Field::Hand)?;
        if first.is_some() && hand.is_empty() {
            return Ok(Vec::new());
        }
        let mut choices = Vec::new();
        for value in hand {
            let ObjectValue::Atom(Datum::Instance(id)) = value else {
                return Err(unsupported(
                    "planning physical-card collection binding missing",
                ));
            };
            let card = CardId(*id);
            if self.world.card(card).is_none() {
                return Err(unsupported(
                    "planning physical-card collection binding missing",
                ));
            }
            if self.world.normal_card(card).is_ok() {
                choices.push(DecisionMove::Card { hero, card });
                if choices.len() == limit {
                    return Ok(choices);
                }
            }
        }
        if first.is_some() {
            choices.push(DecisionMove::FinishPlanning { hero });
        } else if hand.is_empty() {
            choices.push(DecisionMove::Pass { hero });
        }
        Ok(choices)
    }

    pub(super) fn hero_card_list_bound(
        &self,
        objects: &BoundObjects,
        model: ObjectId,
        field: Field,
    ) -> Result<Vec<Option<CardId>>> {
        self.decode_hero_cards(self.hero_card_values_bound(objects, model, field)?, field)
    }

    fn hero_card_values_bound<'a>(
        &self,
        objects: &'a BoundObjects,
        model: ObjectId,
        field: Field,
    ) -> Result<&'a [ObjectValue]> {
        let Some(ObjectValue::Object(list)) = objects.arena.field_ref(model, objects.phases.k(field))
        else {
            return Err(unsupported("planning card collection missing"));
        };
        objects
            .arena
            .sequence(*list)
            .ok_or_else(|| unsupported("planning cards must be a list"))
    }

    fn decode_hero_cards(
        &self,
        values: &[ObjectValue],
        field: Field,
    ) -> Result<Vec<Option<CardId>>> {
        values
            .iter()
            .map(|value| match value {
                ObjectValue::Atom(Datum::Instance(card))
                    if self.world.card(CardId(*card)).is_some() =>
                {
                    Ok(Some(CardId(*card)))
                }
                ObjectValue::Atom(Datum::Null) if matches!(field, Field::Played) => Ok(None),
                _ => Err(unsupported(
                    "planning physical-card collection binding missing",
                )),
            })
            .collect()
    }
    // Inspect live handles even when lists share identities across heroes and
    // roles. Unchanged contents can reuse the typed projection.
    pub(super) fn sync_card_collections_bound(&mut self, objects: &BoundObjects) -> Result<()> {
        let mut profile = CARD_SYNC_PROFILE.get();
        let mut compared = 0;
        for &(hero, model) in &objects.phases.heroes {
            let index = self
                .world
                .hero_index(hero)
                .ok_or_else(|| unsupported("planning hero cache missing"))?;
            // The model page covers rebinding and scalar slots. List pages
            // cover edits through every alias, even across different heroes.
            if self.world.heroes.card_sync_stamps[index].is_some_and(|stamps| {
                stamps
                    .into_iter()
                    .all(|stamp| objects.arena.matches_page_stamp(stamp))
            }) {
                if let Some(profile) = &mut profile {
                    profile.skipped_heroes += 1;
                }
                continue;
            }
            compared += 1;
            let fields = [
                Field::Deck,
                Field::Hand,
                Field::Discard,
                Field::Spells,
                Field::Played,
            ];
            let model_stamp = objects
                .arena
                .page_stamp(model)
                .ok_or_else(|| unsupported("planning hero model missing"))?;
            let mut stamps = [model_stamp; 6];
            let mut values = [&[][..]; 5];
            for (index, field) in fields.into_iter().enumerate() {
                let Some(ObjectValue::Object(list)) =
                    objects.arena.field_ref(model, objects.phases.k(field))
                else {
                    return Err(unsupported("planning card collection missing"));
                };
                stamps[index + 1] = objects
                    .arena
                    .page_stamp(*list)
                    .ok_or_else(|| unsupported("planning cards must be a list"))?;
                values[index] = objects
                    .arena
                    .sequence(*list)
                    .ok_or_else(|| unsupported("planning cards must be a list"))?;
            }
            let [deck_values, hand_values, discard_values, spell_values, played_values] = values;
            let row = &self.world.heroes[index];
            let update = |field, values: &[ObjectValue], old: &[CardId]| -> Result<Option<Vec<CardId>>> {
                if values.len() == old.len()
                    && values.iter().zip(old).all(|(value, card)| {
                        matches!(value, ObjectValue::Atom(Datum::Instance(id)) if *id == card.0)
                    })
                {
                    return Ok(None);
                }
                Ok(Some(
                    self.decode_hero_cards(values, field)?
                        .into_iter()
                        .flatten()
                        .collect(),
                ))
            };
            let deck = update(Field::Deck, deck_values, &row.deck)?;
            let hand = update(Field::Hand, hand_values, &row.hand)?;
            let discard = update(Field::Discard, discard_values, &row.discard)?;
            let spells = update(Field::Spells, spell_values, &row.spells)?;
            let played = if played_values.len() == row.played.len()
                && played_values
                    .iter()
                    .zip(&row.played)
                    .all(|(value, card)| match (value, card) {
                        (ObjectValue::Atom(Datum::Instance(id)), Some(card)) => *id == card.0,
                        (ObjectValue::Atom(Datum::Null), None) => true,
                        _ => false,
                    }) {
                None
            } else {
                Some(self.decode_hero_cards(played_values, Field::Played)?)
            };
            let current = self.model_card_bound(objects, model, Field::Current)?;
            let extra = self.model_card_bound(objects, model, Field::Extra)?;
            let ultimate = self.model_card_bound(objects, model, Field::Ultimate)?;
            if deck.is_some()
                || hand.is_some()
                || discard.is_some()
                || spells.is_some()
                || played.is_some()
                || current != row.current
                || extra != row.extra
                || ultimate != row.ultimate
            {
                if let Some(profile) = &mut profile {
                    profile.changed_heroes += 1;
                    profile.changed_lists += [
                        deck.is_some(),
                        hand.is_some(),
                        discard.is_some(),
                        spells.is_some(),
                        played.is_some(),
                    ]
                    .into_iter()
                    .filter(|changed| *changed)
                    .count();
                }
                let row = &mut self.world.heroes[index];
                if let Some(deck) = deck {
                    row.deck = deck;
                }
                if let Some(hand) = hand {
                    row.hand = hand;
                }
                if let Some(discard) = discard {
                    row.discard = discard;
                }
                if let Some(spells) = spells {
                    row.spells = spells;
                }
                if let Some(played) = played {
                    row.played = played;
                }
                row.current = current;
                row.extra = extra;
                row.ultimate = ultimate;
            }
            self.world.heroes.card_sync_stamps[index] = Some(stamps);
        }
        self.world.refresh_card_auras();
        if let Some(mut profile) = profile {
            profile.calls += 1;
            profile.lists += 5 * compared;
            CARD_SYNC_PROFILE.set(Some(profile));
        }
        Ok(())
    }
    fn planning_hand_bound(&self, objects: &BoundObjects, model: ObjectId) -> Result<ObjectId> {
        match objects.arena.field(model, objects.phases.k(Field::Hand)) {
            Some(ObjectValue::Object(hand)) if objects.arena.is_mutable_list(hand) => Ok(hand),
            _ => Err(unsupported("planning hand must be a mutable list")),
        }
    }
    fn planning_card_scalars_bound(
        &mut self,
        objects: &mut BoundObjects,
        card: CardId,
        committed: bool,
    ) -> Result<()> {
        let physical = self
            .world
            .card(card)
            .ok_or_else(|| unsupported("planning physical card missing"))?
            .source_instance;
        let p = &objects.phases;
        for (key, value) in [
            (
                Field::CardState,
                Datum::Text(p.k(if committed {
                    Field::UnresolvedState
                } else {
                    Field::HandState
                })),
            ),
            (Field::Facedown, Datum::Bool(committed)),
            (Field::PlayedRound, Datum::Bool(committed)),
        ] {
            objects
                .arena
                .set_physical_field(physical, p.k(key), value)
                .map_err(unsupported)?;
        }
        if !committed {
            objects
                .arena
                .remove_physical_field(physical, p.k(Field::InitiativeOverride))
                .map_err(unsupported)?;
        }
        Ok(())
    }
    pub(in crate::interpreter) fn commit_card_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
        card: CardId,
    ) -> Result<()> {
        if self.planning.phase != Phase::Planning {
            return Ok(());
        }
        let Some(model) = self.hero_model_bound(objects, hero) else {
            return Ok(());
        };
        self.sync_card_collections_bound(objects)?;
        let pending = objects.phases.root(&objects.arena, Field::Pending)?;
        let second = objects.phases.root(&objects.arena, Field::Second)?;
        let done = objects.phases.root(&objects.arena, Field::Done)?;
        let is_second = if let Some(first) = objects.arena.field(pending, hero) {
            if matches!(first, ObjectValue::Atom(Datum::Null))
                || !self.planning.two_cards(&self.world, hero)
            {
                return Err(EngineError::InvalidInput("already committed".into()));
            }
            if objects.arena.field(second, hero).is_some()
                || objects
                    .arena
                    .sequence(done)
                    .ok_or_else(|| unsupported("planning done list missing"))?
                    .iter()
                    .any(|value| matches!(value, ObjectValue::Atom(Datum::Text(id)) if *id == hero))
            {
                return Err(EngineError::InvalidInput("planning already closed".into()));
            }
            true
        } else {
            false
        };
        let hand = self.planning_hand_bound(objects, model)?;
        let cards = self.hero_card_list_bound(objects, model, Field::Hand)?;
        let Some(index) = cards.iter().position(|id| *id == Some(card)) else {
            let row = self
                .world
                .card(card)
                .ok_or_else(|| unsupported("runtime copied planning card is not bound"))?;
            let printed = self.world.definitions.cards[row.definition].printed_id;
            if cards.iter().flatten().any(|id| {
                self.world.card(*id).is_some_and(|row| {
                    self.world.definitions.cards[row.definition].printed_id == printed
                })
            }) {
                return Err(unsupported(
                    "foreign printed-card membership requires native model equality",
                ));
            }
            return Ok(());
        };
        match self.world.play_card(hero, card) {
            Ok(()) => {}
            Err(crate::cards::CardError::Illegal(_)) => return Ok(()),
            Err(error) => return Err(card_error(error)),
        }
        objects
            .arena
            .apply(&crate::objects::Mutation::Pop {
                target: crate::objects::Target::Object(hand),
                index: index as i64,
            })
            .map_err(unsupported)?;
        self.planning_card_scalars_bound(objects, card, true)?;
        objects
            .arena
            .set_field(
                model,
                objects.phases.k(Field::Current),
                ObjectValue::Atom(Datum::Instance(card.0)),
            )
            .map_err(unsupported)?;
        if !is_second {
            self.clear_planning_requests_bound(objects, hero)?;
        }
        objects
            .arena
            .set_field(
                if is_second { second } else { pending },
                hero,
                ObjectValue::Atom(Datum::Instance(card.0)),
            )
            .map_err(unsupported)?;
        self.sync_card_collections_bound(objects)?;
        objects.readonly.take();
        self.check_planning_transition_bound(objects)
    }
    pub(in crate::interpreter) fn uncommit_card_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
    ) -> Result<()> {
        if self.planning.phase != Phase::Planning {
            return Err(EngineError::InvalidInput(
                "cannot uncommit outside planning".into(),
            ));
        }
        let model = self
            .hero_model_bound(objects, hero)
            .ok_or_else(|| EngineError::InvalidInput("unknown planning hero".into()))?;
        let pending = objects.phases.root(&objects.arena, Field::Pending)?;
        let first = match objects.arena.field(pending, hero) {
            None | Some(ObjectValue::Atom(Datum::Null)) => {
                return Err(EngineError::InvalidInput("no committed card".into()));
            }
            Some(ObjectValue::Atom(Datum::Instance(card))) => CardId(card),
            _ => return Err(unsupported("pending physical card missing")),
        };
        let second = objects.phases.root(&objects.arena, Field::Second)?;
        let extra = match objects.arena.field(second, hero) {
            None | Some(ObjectValue::Atom(Datum::Null)) => None,
            Some(ObjectValue::Atom(Datum::Instance(card))) => Some(CardId(card)),
            _ => return Err(unsupported("second physical card missing")),
        };
        let card = extra.unwrap_or(first);
        self.sync_card_collections_bound(objects)?;
        // Unported equality/instance shapes reject before source-visible edits.
        self.world.lifecycle_hero(hero).map_err(card_error)?;
        let normal = self.world.normal_card(card);
        let hand = self.planning_hand_bound(objects, model)?;
        objects
            .arena
            .apply(&crate::objects::Mutation::Delete {
                target: crate::objects::Target::Object(if extra.is_some() {
                    second
                } else {
                    pending
                }),
                key: hero,
            })
            .map_err(unsupported)?;
        let planning_error = |error| match error {
            crate::cards::CardError::Illegal(message) => EngineError::InvalidInput(message.into()),
            crate::cards::CardError::Unsupported(message) => unsupported(message),
        };
        normal.map_err(planning_error)?;
        self.world.unplay_card(hero, card).map_err(planning_error)?;
        self.planning_card_scalars_bound(objects, card, false)?;
        objects
            .arena
            .append(hand, ObjectValue::Atom(Datum::Instance(card.0)))
            .map_err(unsupported)?;
        objects
            .arena
            .set_field(
                model,
                objects.phases.k(Field::Current),
                ObjectValue::Atom(if extra.is_some() {
                    Datum::Instance(first.0)
                } else {
                    Datum::Null
                }),
            )
            .map_err(unsupported)?;
        let done = objects.phases.root(&objects.arena, Field::Done)?;
        if let Some(index) = objects
            .arena
            .sequence(done)
            .ok_or_else(|| unsupported("planning done list missing"))?
            .iter()
            .position(|value| matches!(value, ObjectValue::Atom(Datum::Text(id)) if *id == hero))
        {
            objects
                .arena
                .apply(&crate::objects::Mutation::Pop {
                    target: crate::objects::Target::Object(done),
                    index: index as i64,
                })
                .map_err(unsupported)?;
        }
        self.sync_card_collections_bound(objects)?;
        objects.readonly.take();
        Ok(())
    }
    pub(super) fn hero_hand_nonempty_bound(
        &self,
        objects: &BoundObjects,
        hero: Symbol,
    ) -> Result<bool> {
        let Some(model) = self.hero_model_bound(objects, hero) else {
            return Ok(false);
        };
        let Some(ObjectValue::Object(hand)) =
            objects.arena.field(model, objects.phases.k(Field::Hand))
        else {
            return Err(unsupported("planning hand binding missing"));
        };
        Ok(!objects
            .arena
            .sequence(hand)
            .ok_or_else(|| unsupported("planning hand must be a list"))?
            .is_empty())
    }
    fn check_planning_transition_bound(&mut self, objects: &mut BoundObjects) -> Result<()> {
        let pending = objects.phases.root(&objects.arena, Field::Pending)?;
        let count = objects
            .arena
            .fields(pending)
            .ok_or_else(|| unsupported("planning inputs must be a dictionary"))?
            .len();
        if count >= objects.phases.heroes.len() && !self.pending_planning_open_bound(objects)? {
            self.start_revelation_bound(objects)?;
        }
        Ok(())
    }
    pub(super) fn clear_planning_requests_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
    ) -> Result<()> {
        let p = &objects.phases;
        // Source clear_requests always replaces the dictionary, retaining each
        // surviving request object. References to the old dictionary detach.
        let requests = p.root(&objects.arena, Field::Requests)?;
        let mut retained = Vec::new();
        for (key, value) in objects
            .arena
            .fields(requests)
            .ok_or_else(|| unsupported("starting requests must be a dictionary"))?
        {
            let ObjectValue::Object(request) = value else {
                return Err(unsupported("starting request row missing"));
            };
            let from = objects
                .arena
                .field(*request, p.k(Field::From))
                .ok_or_else(|| unsupported("starting request sender missing"))?;
            let to = objects
                .arena
                .field(*request, p.k(Field::To))
                .ok_or_else(|| unsupported("starting request recipient missing"))?;
            if !matches!(from, ObjectValue::Atom(Datum::Text(id)) if id == hero)
                && !matches!(to, ObjectValue::Atom(Datum::Text(id)) if id == hero)
            {
                retained.push((*key, value.clone()));
            }
        }
        let requests = objects.arena.dictionary(retained).map_err(unsupported)?;
        objects
            .arena
            .set_root(p.k(Field::Requests), requests)
            .map_err(unsupported)?;
        Ok(())
    }
    pub(in crate::interpreter) fn pass_planning_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
    ) -> Result<()> {
        if self.planning.phase != Phase::Planning {
            return Err(EngineError::InvalidInput(
                "cannot pass outside planning".into(),
            ));
        }
        if self.hero_model_bound(objects, hero).is_none() {
            return Err(EngineError::InvalidInput("unknown planning hero".into()));
        }
        let p = &objects.phases;
        let pending = p.root(&objects.arena, Field::Pending)?;
        if objects.arena.field(pending, hero).is_some() {
            return Err(EngineError::InvalidInput(
                "hero has already committed".into(),
            ));
        }
        if self.hero_hand_nonempty_bound(objects, hero)? {
            return Err(EngineError::InvalidInput(
                "cannot pass with cards in hand".into(),
            ));
        }
        self.clear_planning_requests_bound(objects, hero)?;
        objects
            .arena
            .set_field(pending, hero, ObjectValue::Atom(Datum::Null))
            .map_err(unsupported)?;
        objects.readonly.take();
        self.check_planning_transition_bound(objects)
    }
    pub(in crate::interpreter) fn finish_planning_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
    ) -> Result<()> {
        if self.planning.phase != Phase::Planning {
            return Ok(());
        }
        let p = &objects.phases;
        let pending = p.root(&objects.arena, Field::Pending)?;
        if objects.arena.field(pending, hero).is_none() {
            return Err(EngineError::InvalidInput(
                "must commit before finishing planning".into(),
            ));
        }
        let done = p.root(&objects.arena, Field::Done)?;
        let value = ObjectValue::Atom(Datum::Text(hero));
        if !objects
            .arena
            .sequence(done)
            .ok_or_else(|| unsupported("planning done must be a list"))?
            .iter()
            .any(|item| matches!(item, ObjectValue::Atom(Datum::Text(id)) if *id == hero))
        {
            objects.arena.append(done, value).map_err(unsupported)?;
        }
        self.check_planning_transition_bound(objects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concealed_discard_only_blocks_selectors_that_can_read_it() {
        for (case, reads_concealed) in [("merge-hidden", false), ("include-hidden", true)] {
            let fixture = fixture_case(&format!("object-execution-card-selector-{case}"));
            let mut root = NativeSession::compile(
                serde_json::from_value(fixture["envelope"].clone()).unwrap(),
            ).unwrap();
            assert!(root.has_concealed_card_in_public_zone());
            let request = root.advance_prefix(1).unwrap().request.unwrap();
            let frame = root.stack.last().unwrap();
            assert!(matches!(root.steps[frame.instruction].instruction, Instruction::LiveCardSelect));
            assert_eq!(
                root.search_step_reads_concealed_state(
                    root.objects.as_ref().unwrap(), frame.instruction, frame.object, request.owner,
                ).unwrap(),
                reads_concealed,
            );
        }
    }

    #[test]
    fn sampled_round_minion_defeat_requires_the_selector_binding() {
        for card in ["final_embrace", "constrict"] {
            let fixture = fixture_case(&format!("object-execution-xargatha-primary-{card}-round-ordinary-first"));
            let mut root = NativeSession::compile(
                serde_json::from_value(fixture["envelope"].clone()).unwrap(),
            ).unwrap();
            root.advance_prefix(1).unwrap();
            root.advance_prefix(1).unwrap();
            assert_eq!(root.world.effects.instances.len(), 1);
            root.validate_sampled_resolution_effects().unwrap();

            root.with_live_objects(|_, objects| {
                let node = objects.effect_nodes()?[0];
                let finishing = objects.finishing_list(node)?;
                let defeat = objects.steps_in_list(finishing)?[1];
                objects.arena.set_field(
                    defeat, objects.phases.k(Field::VictimKey), ObjectValue::Atom(Datum::Null),
                ).map_err(unsupported)
            }).unwrap();
            assert!(matches!(root.validate_sampled_resolution_effects(),
                Err(EngineError::Unsupported(reason))
                if reason == "unexpected round-end minion defeat program"));
        }
    }

    #[test]
    fn sampled_resolution_allows_public_double_items() {
        let fixture = fixture_case("object-execution-min-primary-inner_strength-text-first");
        let mut root = NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        ).unwrap();
        for _ in 0..8 {
            if root.world.effects.instances.iter().any(|effect| {
                root.symbols.name(effect.definition.kind) == "double_items"
            }) {
                root.validate_sampled_resolution_effects().unwrap();
                return;
            }
            root.advance_prefix(1).unwrap();
        }
        panic!("fixture must create Double Items");
    }

    #[test]
    fn sampled_grenade_allows_its_issued_target_request() {
        let fixture = fixture_case("object-execution-min-primary-death_grenade-text-first");
        let mut root = NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        ).unwrap();
        for operation in fixture["operations"].as_array().unwrap() {
            if operation["kind"] == "input" {
                let request = root.last_request.as_ref().unwrap().id;
                let selection = root.import_selection(&operation["selection"]).unwrap();
                root.submit(request, selection).unwrap();
                continue;
            }
            let Some(request) = root.advance_prefix(1).unwrap().request else {
                continue;
            };
            if !matches!(request.choices, RequestChoices::Multiple { .. }) {
                continue;
            }

            let objects = root.objects.as_ref().unwrap();
            let step = root.stack.last().unwrap().object.unwrap();
            assert_eq!(objects.token(step).unwrap(), Some(request.id));
            let node = objects.effect_nodes().unwrap()[0];
            let finishing = objects.finishing_list(node).unwrap();
            assert_eq!(objects.steps_in_list(finishing).unwrap()[1], step);
            root.validate_sampled_resolution_effects().unwrap();
            return;
        }
        panic!("fixture must issue the live grenade target prompt");
    }

    #[test]
    fn sampled_grenade_rejects_a_rebound_public_target() {
        let fixture = fixture_case("object-execution-min-primary-death_grenade-text-first");
        let mut root = NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        ).unwrap();
        let index = fixture["operations"].as_array().unwrap().iter().find_map(|operation| {
            if operation["kind"] == "input" {
                let request = root.last_request.as_ref().unwrap().id;
                let selection = root.import_selection(&operation["selection"]).unwrap();
                root.submit(request, selection).unwrap();
            } else {
                root.advance_prefix(1).unwrap();
            }
            root.world.effects.instances.iter().position(|effect| {
                root.symbols.name(effect.definition.kind) == "delayed_trigger"
                    && effect.card.is_some_and(|id| root.symbols.name(id) == "death_grenade")
            })
        }).expect("fixture must create the delayed grenade");
        root.validate_sampled_resolution_effects().unwrap();

        let symbols = &root.symbols;
        let objects = root.objects.as_mut().unwrap();
        let node = objects.effect_nodes().unwrap()[index];
        let finishing = objects.finishing_list(node).unwrap();
        let targets = objects.steps_in_list(finishing).unwrap()[1];
        objects.arena.set_field(
            targets,
            symbols.lookup("active_if_key").unwrap(),
            ObjectValue::Atom(Datum::Text(symbols.lookup("grenade_victims").unwrap())),
        ).unwrap();
        assert!(matches!(
            root.validate_sampled_resolution_effects(),
            Err(EngineError::Unsupported(reason)) if reason == "unexpected grenade finishing program",
        ));
    }

    #[test]
    fn sampled_glitch_cleanup_rejects_a_rebound_remove_token() {
        let fixture = fixture_case("object-execution-defense-body-unstable_timeline-text-first");
        let mut root = NativeSession::compile(
            serde_json::from_value(fixture["envelope"].clone()).unwrap(),
        ).unwrap();
        let index = fixture["operations"].as_array().unwrap().iter().find_map(|operation| {
            if operation["kind"] == "input" {
                let request = root.last_request.as_ref().unwrap().id;
                let selection = root.import_selection(&operation["selection"]).unwrap();
                root.submit(request, selection).unwrap();
            } else {
                root.advance_prefix(1).unwrap();
            }
            root.world.effects.instances.iter().position(|effect| {
                root.symbols.name(effect.definition.kind) == "delayed_trigger"
                    && effect.card.is_some_and(|id| root.symbols.name(id) == "unstable_timeline")
            })
        }).expect("fixture must create the delayed Glitch cleanup");
        root.validate_sampled_resolution_effects().unwrap();

        let symbols = &root.symbols;
        let objects = root.objects.as_mut().unwrap();
        let node = objects.effect_nodes().unwrap()[index];
        let finishing = objects.finishing_list(node).unwrap();
        let repeat = objects.steps_in_list(finishing).unwrap()[1];
        let Some(ObjectValue::Object(templates)) = objects.arena.field(
            repeat, symbols.lookup("steps_template").unwrap(),
        ) else {
            panic!("cleanup loop must have a template");
        };
        let remove = objects.steps_in_list(templates).unwrap()[0];
        objects.arena.set_field(
            remove,
            symbols.lookup("token_key").unwrap(),
            ObjectValue::Atom(Datum::Text(symbols.lookup("_glitch_cleanup_ids").unwrap())),
        ).unwrap();
        assert!(matches!(
            root.validate_sampled_resolution_effects(),
            Err(EngineError::Unsupported(reason)) if reason == "unexpected Glitch cleanup program",
        ));
    }

    #[test]
    fn sampled_glitch_handoff_preserves_choices_after_hidden_hand_changes() {
        let fixture = fixture_case("object-execution-defense-body-unstable_timeline-text-first");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let mut checked = 0;
        for operation in fixture["operations"].as_array().unwrap().iter().take(20) {
            if operation["kind"] == "input" {
                let request = root.last_request.as_ref().unwrap().id;
                let selection = root.import_selection(&operation["selection"]).unwrap();
                root.submit(request, selection).unwrap();
                continue;
            }
            let Some(request) = root.advance_prefix(1).unwrap().request else {
                continue;
            };
            if !matches!(request.choices, RequestChoices::Units { .. }) {
                continue;
            }
            let before = root.semantic_state().unwrap();
            let view = root.player_observation(request.owner).unwrap();
            let sample = super::super::PlanningRolloutSample {
                seed: 39,
                upgrade_checkpoints: None,
            };
            let legal = root.resolution_choices(request.owner).unwrap();
            let searched = root.search_resolution_in_sampled_world(
                request.owner,
                sample,
                super::super::SearchLimits {
                    max_depth: 2,
                    max_nodes: 64,
                    opponent_policy: super::super::OpponentPolicy::Adversarial,
                    evaluation: super::super::SearchEvaluation::MaterialPosition,
                },
                &ExecutionBudget::default(),
            ).unwrap();
            assert!(legal.contains(&searched.decision));
            assert!(searched.nodes > 0);

            // Level-one hands are uniquely determined. Perturb a different
            // hero's concealed hand explicitly to test the prompt dependency.
            let mut samples = root.resolution_samples(request.owner, sample, &[39], &[])
                .unwrap().remove(0);
            let hidden = samples.iter_mut()
                .find(|row| root.symbols.name(row.hero) == "hero_swift").unwrap();
            let hero = hidden.hero;
            let old_hand = root.world.hero(hero).unwrap().hand.clone();
            let SampledCards::Hand(hand) = &mut hidden.update else {
                panic!("expected a sampled hand");
            };
            let replacement = root.with_live_objects(|session, objects| {
                let model = session.hero_model_bound(objects, hero).unwrap();
                let deck = session.hero_card_list_bound(objects, model, Field::Deck)?;
                Ok(deck.into_iter().flatten().find_map(|card| {
                    let id = session.printed_observation_card(card).unwrap();
                    (!old_hand.contains(&card) && !hidden.fixed.contains(&Some(card))
                        && !hand.cards.contains(&id)).then_some(id)
                }).unwrap())
            }).unwrap();
            hand.cards[0] = replacement;
            root.with_installed_resolution_samples(request.owner, samples, None, |worker| {
                assert!(worker.world.hero(hero).unwrap().hand.iter()
                    .any(|card| !old_hand.contains(card)));
                assert_eq!(worker.player_observation(request.owner)?, view);
                Ok(())
            }).unwrap();
            assert_eq!(root.semantic_state().unwrap(), before);
            checked += 1;
        }
        assert_eq!(checked, 2);
    }

    #[test]
    fn sampled_totem_protection_requires_a_public_anchor() {
        let fixture = fixture_case("object-execution-live-combat-effect-protection-totem-False");
        let mut envelope = fixture["envelope"].clone();
        fn adjust_effect_copies(value: &mut serde_json::Value) {
            match value {
                serde_json::Value::Object(fields) => {
                    match fields.get("effect_type").and_then(serde_json::Value::as_str) {
                        Some("minion_defeat_bounty") => {
                            fields.insert("source_card_id".into(), json!("fixture_known_card"));
                        }
                        Some("minion_protection") => {
                            fields.insert("allowed_discard_colors".into(), json!([]));
                        }
                        _ => {}
                    }
                    fields.values_mut().for_each(adjust_effect_copies);
                }
                serde_json::Value::Array(items) => {
                    items.iter_mut().for_each(adjust_effect_copies);
                }
                _ => {}
            }
        }
        // The fixture repeats effect rows at shared object paths.
        adjust_effect_copies(&mut envelope);
        let mut root = NativeSession::compile(serde_json::from_value(envelope).unwrap()).unwrap();
        let viewer = root.world.heroes[0].id;
        let token = root.world.effects.instances[1].token.unwrap();
        let mut observation = root.player_observation(viewer).unwrap();
        observation.heroes.iter_mut().find(|hero| hero.id == "hero_swift").unwrap()
            .known_spells.push("fixture_known_card".into());

        for phase in [Phase::Planning, Phase::Resolution] {
            observation.phase = phase;
            root.validate_sampled_effect_cards(&observation).unwrap();
        }
        root.with_live_objects(|_, objects| {
            let misc = objects.phases.root(&objects.arena, Field::MiscEntities)?;
            let Some(ObjectValue::Atom(Datum::Instance(physical))) = objects.arena.field(misc, token) else {
                panic!("placed Totem missing");
            };
            objects.arena.set_physical_field(
                physical, objects.phases.k(Field::Facedown), Datum::Bool(true),
            ).map_err(unsupported)
        }).unwrap();
        for phase in [Phase::Planning, Phase::Resolution] {
            observation.phase = phase;
            assert!(matches!(root.validate_sampled_effect_cards(&observation),
                Err(EngineError::Unsupported(reason))
                if reason == "sampled worlds cannot preserve minion_protection effect: source card missing"));
        }
    }

    #[test]
    fn sampled_worlds_require_token_effect_anchor_to_be_faceup() {
        let fixture = fixture_case("object-execution-min-primary-smoke_bomb-text-first");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        root.advance_prefix(1).unwrap();
        let request = root.advance_prefix(1).unwrap().request.unwrap();
        let selection = root.import_selection(&fixture["operations"][2]["selection"]).unwrap();
        root.submit(request.id, selection).unwrap();
        for _ in 0..4 {
            if !root.world.effects.instances.is_empty() {
                break;
            }
            root.advance_prefix(1).unwrap();
        }
        let token = root.world.effects.instances[0].token.unwrap();
        let mut observation = root.player_observation(root.world.heroes[0].id).unwrap();
        observation.phase = Phase::Resolution;
        root.validate_sampled_effect_cards(&observation).unwrap();
        observation.phase = Phase::Planning;
        root.validate_sampled_effect_cards(&observation).unwrap();

        root.with_live_objects(|_, objects| {
            let misc = objects.phases.root(&objects.arena, Field::MiscEntities)?;
            let Some(ObjectValue::Atom(Datum::Instance(id))) = objects.arena.field(misc, token)
            else {
                panic!("placed token missing");
            };
            objects.arena.set_physical_field(id, objects.phases.k(Field::Facedown), Datum::Bool(true))
                .map_err(unsupported)
        }).unwrap();
        assert!(root.validate_sampled_effect_cards(&observation).is_err());
        observation.phase = Phase::Resolution;
        assert!(root.validate_sampled_effect_cards(&observation).is_err());
    }

    #[test]
    fn sampled_resolution_accepts_effect_cards_in_revealed_current_slots() {
        let fixture = fixture_case("object-execution-effects-expired-finishing-True");
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let mut observation = root.player_observation(root.world.heroes[0].id).unwrap();
        observation.phase = Phase::Resolution;
        observation.private.hand.clear();
        observation.private.current = None;
        observation.private.extra = None;
        for row in &mut observation.heroes {
            row.played.clear();
            row.discarded.clear();
            row.revealed_current.clear();
        }
        assert!(root.validate_sampled_effect_cards(&observation).is_err());
        for effect in &root.world.effects.instances {
            let source = root.world.hero(effect.source).unwrap();
            let hero = observation.heroes.iter_mut()
                .find(|row| row.id == root.symbols.name(source.id)).unwrap();
            for card in [Some(effect.card.unwrap()), effect.linked_card].into_iter().flatten() {
                hero.revealed_current.push(root.symbols.name(card).into());
            }
        }
        root.validate_sampled_effect_cards(&observation).unwrap();
        observation.phase = Phase::Planning;
        assert!(root.validate_sampled_effect_cards(&observation).is_err());

        observation.phase = Phase::Resolution;
        let private = observation.heroes.iter_mut()
            .find(|hero| hero.id == observation.private.hero).unwrap();
        observation.private.hand = std::mem::take(&mut private.revealed_current);
        root.validate_sampled_effect_cards(&observation).unwrap();

        observation.private.hero = observation.heroes.iter()
            .find(|hero| hero.id != observation.private.hero).unwrap().id.clone();
        assert!(root.validate_sampled_effect_cards(&observation).is_err());
    }

    #[test]
    fn sampled_resolution_checks_finishing_steps_in_the_arena() {
        let fixture = fixture_case("object-execution-effects-expired-finishing-True");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        use crate::effect_rules::Kind;
        for kind in [
            Kind::PreDiscard,
            Kind::ActionImmunity,
            Kind::MovementZone,
            Kind::LosBlocker,
            Kind::AreaStat,
            Kind::MovementAura,
            Kind::StaticBarrier,
            Kind::Petrify,
        ] {
            for effect in &mut root.world.effects.instances {
                let definition = Arc::make_mut(&mut effect.definition);
                assert!(definition.finishing.is_empty());
                definition.behavior.kind = kind;
            }
            let error = root.validate_sampled_resolution_effects().unwrap_err();
            assert!(
                matches!(error, EngineError::Unsupported(message) if message.contains("finishing steps"))
            );
        }
    }

    #[test]
    fn sampled_resolution_ignores_an_unobserved_hand_substitution() {
        use crate::interpreter::{OpponentPolicy, SearchEvaluation, SearchLimits};

        let fixture = fixture_case("object-execution-card-planning-cycle");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let player = root.world.heroes[0].id;
        let mut plan = root.joint_plans(player, true, 1).unwrap().plans.remove(0);
        plan.extend(root.joint_plans(player, false, 1).unwrap().plans.remove(0));
        root.apply_joint_planning_ordered(&plan).unwrap();
        let budget = ExecutionBudget::default();
        root.advance_to_decision(&budget).unwrap();
        let owner = root.last_request.as_ref().unwrap().owner;
        let team = root.world.hero(owner).unwrap().team;
        let other = root
            .world
            .heroes
            .iter()
            .find(|hero| hero.id != owner && hero.team == team)
            .unwrap();
        let (hero, protected) = (other.id, [other.current, other.extra]);
        let view = root.player_observation(owner).unwrap();
        let mut sample = view
            .sample_resolution_hand_with_checkpoints(root.symbols.name(hero), 39, &[])
            .unwrap()
            .unwrap();

        let mut alternate = root.clone();
        alternate
            .with_live_objects(|session, objects| {
                let model = session.hero_model_bound(objects, hero).unwrap();
                let deck = session.hero_card_list_bound(objects, model, Field::Deck)?;
                let replacement = deck
                    .into_iter()
                    .flatten()
                    .find_map(|card| {
                        let id = session.printed_observation_card(card).unwrap();
                        (!protected.contains(&Some(card)) && !sample.cards.contains(&id))
                            .then_some(id)
                    })
                    .expect("fixture has a card outside its public and hand slots");
                sample.cards[0] = replacement;
                session.replace_sampled_hand(objects, hero, &sample, protected)?;
                session.sync_card_collections_bound(objects)
            })
            .unwrap();
        assert_ne!(
            root.world.hero(hero).unwrap().hand,
            alternate.world.hero(hero).unwrap().hand
        );
        assert_eq!(view, alternate.player_observation(owner).unwrap());

        let limits = SearchLimits {
            max_depth: 1,
            max_nodes: 128,
            opponent_policy: OpponentPolicy::Adversarial,
            evaluation: SearchEvaluation::MaterialPosition,
        };
        let sample = super::super::PlanningRolloutSample {
            seed: 39,
            upgrade_checkpoints: None,
        };
        let roster = root.world.heroes.iter().map(|hero| hero.id).collect::<Vec<_>>();
        let scoped = root.resolution_samples(owner, sample, &[39], &roster).unwrap();
        assert!(!scoped[0].iter().any(|row| row.hero == hero));
        for enemy in root.world.heroes.iter().filter(|hero| hero.team != team) {
            assert!(scoped[0].iter().any(|row| row.hero == enemy.id));
        }
        let worlds = alternate.sampled_resolution_workers(owner, sample, 1, &roster, &budget).unwrap();
        assert_eq!(worlds[0].world.hero(hero).unwrap().hand, alternate.world.hero(hero).unwrap().hand);

        let run = |session: &mut NativeSession, worlds, workers| {
            let before = session.semantic_state().unwrap();
            let result = session
                .search_resolution_worlds(
                    owner,
                    (sample, worlds, workers),
                    limits,
                    None,
                    None,
                    &budget,
                )
                .unwrap();
            assert_eq!(session.semantic_state().unwrap(), before);
            assert_eq!(result.completed_depth, 1);
            assert!(result.score.is_some());
            (result.decision, result.score, result.nodes)
        };
        for worlds in [1, 3] {
            assert_eq!(run(&mut root, worlds, 1), run(&mut alternate, worlds, 1));
        }
        let before = root.semantic_state().unwrap();
        assert_eq!(run(&mut root, 3, 1), run(&mut root, 3, 3));
        let limited = root
            .search_resolution_worlds(
                owner,
                (sample, 3, 3),
                SearchLimits {
                    max_nodes: 1,
                    ..limits
                },
                None,
                None,
                &budget,
            )
            .unwrap();
        assert_eq!(limited.nodes, 1);
        assert_eq!(limited.completed_depth, 0);
        assert_eq!(limited.score, None);
        assert_eq!(limited.stop, crate::interpreter::ResolutionStop::NodeLimit);
        assert_eq!(root.semantic_state().unwrap(), before);
    }

    #[test]
    fn probabilistic_search_pauses_before_hidden_defense_and_discard() {
        for (name, discard) in [
            ("object-execution-live-reaction-Takahide-prompt-invalid-pass", false),
            ("object-execution-forced-discard-plain", true),
        ] {
            let fixture = fixture_case(name);
            let mut root = NativeSession::compile(
                serde_json::from_value(fixture["envelope"].clone()).unwrap()
            ).unwrap();
            let budget = ExecutionBudget::default();
            let mut ordinary = root.clone();
            ordinary.advance_to_decision(&budget).unwrap();
            let request = ordinary.last_request.as_ref().unwrap();
            assert_eq!(request.discard, discard);
            let owner = request.owner;
            let team = root.world.hero(owner).unwrap().team;
            let viewer = root.world.heroes.iter().find(|hero| hero.team != team).unwrap().id;
            root.advance_search_objects_before_hand(&budget, viewer, &[viewer]).unwrap();
            assert!(root.last_request.is_none());
            assert_eq!(root.pending_hidden_hand_reader(viewer, &[viewer]).unwrap(), Some(owner));
            root.advance_search_objects(&budget, viewer).unwrap();
            assert_eq!(root.last_request.as_ref(), Some(request));
        }
    }

    #[test]
    fn sampled_resolution_preserves_the_defender_choices_and_root() {
        use crate::interpreter::{OpponentPolicy, SearchEvaluation, SearchLimits};

        let fixture = fixture_case("object-execution-live-reaction-Arien-prompt-invalid-pass");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let budget = ExecutionBudget::default();
        root.advance_to_decision(&budget).unwrap();
        let request = root.last_request.as_ref().unwrap();
        assert!(matches!(request.choices, RequestChoices::Defense(_)));
        let player = request.owner;
        let choices = root.resolution_choices(player).unwrap();
        let before = root.semantic_state().unwrap();
        let result = root
            .search_resolution_in_sampled_world(
                player,
                super::super::PlanningRolloutSample {
                    seed: 39,
                    upgrade_checkpoints: None,
                },
                SearchLimits {
                    max_depth: 1,
                    max_nodes: 128,
                    opponent_policy: OpponentPolicy::Adversarial,
                    evaluation: SearchEvaluation::MaterialPosition,
                },
                &budget,
            )
            .unwrap();
        assert!(choices.contains(&result.decision));
        assert!(result.score.is_some());
        assert_eq!(root.semantic_state().unwrap(), before);
    }

    #[test]
    fn pass_releases_detached_request_guards_but_protects_live_shared_rows() {
        let fixture = fixture_case("object-execution-planning-command-pass-requests");
        let mut session =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let Datum::Text(hero) = session
            .import_selection(&fixture["operations"][0]["hero"])
            .unwrap()
        else {
            unreachable!()
        };
        let objects = session.objects.as_ref().unwrap();
        let old = objects
            .phases
            .root(&objects.arena, Field::Requests)
            .unwrap();
        let key = objects.arena.symbols.lookup("keep").unwrap();
        let Some(ObjectValue::Object(kept)) = objects.arena.field(old, key) else {
            unreachable!()
        };
        assert!(objects.protected(old));
        assert!(objects.protected(kept));
        session.pass_planning(hero).unwrap();
        let objects = session.objects.as_ref().unwrap();
        let current = objects
            .phases
            .root(&objects.arena, Field::Requests)
            .unwrap();
        assert_ne!(old, current);
        assert!(!objects.protected(old));
        assert!(objects.protected(current));
        assert!(objects.protected(kept));
    }

    #[test]
    fn card_planning_refreshes_all_shared_collection_caches_without_changing_other_workers() {
        for name in ["shared-deck", "shared-hero-hand"] {
            let fixture = fixture_case(&format!("object-execution-card-planning-{name}"));
            let root = NativeSession::compile(
                serde_json::from_value(fixture["envelope"].clone()).unwrap(),
            )
            .unwrap();
            let before = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
            let mut worker = root.clone();
            worker
                .with_live_objects(|session, objects| {
                    session.sync_card_collections_bound(objects)?;
                    let (result, profile) =
                        with_card_sync_profile(|| session.sync_card_collections_bound(objects));
                    result?;
                    assert_eq!(profile.skipped_heroes, session.world.heroes.len());

                    // A typed-only change must be repaired even if every
                    // arena page still has the same stamp.
                    let hand = session.world.heroes[0].hand.clone();
                    session.world.heroes[0].hand.clear();
                    session.sync_card_collections_bound(objects)?;
                    assert_eq!(session.world.heroes[0].hand, hand);
                    Ok(())
                })
                .unwrap();
            for operation in fixture["operations"].as_array().unwrap() {
                let Datum::Text(hero) = worker.import_selection(&operation["hero"]).unwrap() else {
                    unreachable!()
                };
                if operation["kind"] == "commit" {
                    worker
                        .commit_card(hero, CardId(operation["card"].as_u64().unwrap() as u32))
                        .unwrap();
                } else {
                    worker.uncommit_card(hero).unwrap();
                }
                if name == "shared-deck" {
                    assert_eq!(worker.world.heroes[0].hand, worker.world.heroes[0].deck);
                } else {
                    assert_eq!(worker.world.heroes[0].hand, worker.world.heroes[1].hand);
                }
                assert_eq!(
                    serde_json::to_value(root.export_envelope().unwrap()).unwrap(),
                    before
                );
            }
        }
    }

    #[test]
    fn two_card_planning_workers_choose_second_or_finish_and_withdraw_independently() {
        let fixture = fixture_case("object-execution-card-planning-dual-cycle");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let Datum::Text(hero) = root
            .import_selection(&fixture["operations"][0]["hero"])
            .unwrap()
        else {
            unreachable!()
        };
        let first = CardId(fixture["operations"][0]["card"].as_u64().unwrap() as u32);
        let second = CardId(fixture["operations"][1]["card"].as_u64().unwrap() as u32);
        root.commit_card(hero, first).unwrap();
        let before = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
        let mut one = root.clone();
        let mut two = root.clone();
        one.commit_card(hero, second).unwrap();
        two.finish_planning(hero).unwrap();
        assert_eq!(one.world.hero(hero).unwrap().current, Some(second));
        assert_eq!(two.world.hero(hero).unwrap().current, Some(first));
        one.uncommit_card(hero).unwrap();
        two.uncommit_card(hero).unwrap();
        assert_eq!(one.world.hero(hero).unwrap().current, Some(first));
        assert_eq!(two.world.hero(hero).unwrap().current, None);
        assert_eq!(one.world.hero(hero).unwrap().hand.last(), Some(&second));
        assert_eq!(two.world.hero(hero).unwrap().hand.last(), Some(&first));
        assert_eq!(
            serde_json::to_value(root.export_envelope().unwrap()).unwrap(),
            before
        );
    }

    #[test]
    fn foreign_equal_printed_card_is_guarded_before_mutation_until_model_equality_is_bound() {
        let fixture = fixture_case("object-execution-card-planning-override-copy");
        let envelope: StateEnvelope = serde_json::from_value(fixture["envelope"].clone()).unwrap();
        let copy = crate::cards::reference(&envelope.state["execution_context"]["copy"])
            .unwrap()
            .unwrap();
        let mut session = NativeSession::compile(envelope).unwrap();
        let Datum::Text(hero) = session
            .import_selection(&fixture["operations"][0]["hero"])
            .unwrap()
        else {
            unreachable!()
        };
        let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
        let error = session.commit_card(hero, copy).unwrap_err();
        assert!(error.to_string().contains("native model equality"));
        assert_eq!(
            serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
            before
        );
    }
}
