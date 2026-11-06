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
    hand: super::super::SampledHand,
    fixed: [Option<CardId>; 2],
}

impl NativeSession {
    pub(in crate::interpreter) fn observed_planning_cards(
        &self,
        hero: Symbol,
    ) -> Result<(Option<CardId>, Option<CardId>)> {
        let Some(objects) = &self.objects else {
            return Ok(self.planning.committed_cards(hero));
        };
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
            .resolution_samples(player, sample, &[sample.seed])?
            .pop()
            .unwrap();
        self.with_installed_resolution_samples(player, samples, operation)
    }

    pub(in crate::interpreter) fn sampled_resolution_workers(
        &mut self,
        player: Symbol,
        sample: super::super::PlanningRolloutSample<'_>,
        worlds: usize,
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
        let samples = self.resolution_samples(player, sample, &seeds)?;
        samples
            .into_iter()
            .map(|hands| {
                budget.check(0)?;
                self.with_installed_resolution_samples(player, hands, |worker| Ok(worker.clone()))
            })
            .collect()
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
            Instruction::LiveCardGuess { .. } => true,
            // Bushido can put a card back into a public slot face down.
            // Its printed identity is unavailable even to the slot owner.
            Instruction::LiveCardSelect | Instruction::LiveCardSwap
            | Instruction::LiveResolvedCardSwap | Instruction::LiveDeckCardSwap
            | Instruction::LiveRetrieve | Instruction::LiveDiscard
            | Instruction::LiveColorDiscard => self.world.heroes.iter().any(|hero| {
                hero.played.iter().flatten().chain(&hero.discard)
                    .any(|id| self.world.card(*id).is_some_and(|card| card.facedown))
            }),
            _ => false,
        })
    }

    fn resolution_samples(
        &self,
        player: Symbol,
        sample: super::super::PlanningRolloutSample<'_>,
        seeds: &[u64],
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
            && self.search_step_reads_concealed_state(
                objects, frame.instruction, frame.object, player,
            )?
        {
            return Err(unsupported("resolution sampling cannot reconstruct this concealed choice"));
        }
        let observation = self.player_observation(player)?;
        self.validate_sampled_resolution_effects()?;
        self.validate_sampled_effect_cards(&observation)?;
        if sample.upgrade_checkpoints.is_some_and(|rows| {
            rows.keys()
                .any(|id| !observation.heroes.iter().any(|hero| hero.id == *id))
        }) {
            return Err(EngineError::InvalidInput(
                "upgrade checkpoints name an unknown hero".into(),
            ));
        }
        let mut samples = (0..seeds.len()).map(|_| Vec::new()).collect::<Vec<_>>();
        for (index, hero) in self.world.heroes.iter().enumerate() {
            if hero.id == player {
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
                    hand,
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
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.with_rollback(|worker| {
            let root_choices = worker.resolution_choices(player)?;
            let request = worker.last_request.clone()
                .ok_or_else(|| unsupported("sampled resolution root request missing"))?;
            worker.with_live_objects(|session, objects| {
                for sample in samples {
                    session.replace_sampled_hand(
                        objects,
                        sample.hero,
                        &sample.hand,
                        sample.fixed,
                    )?;
                }
                session.sync_card_collections_bound(objects)?;
                session.world.refresh_card_auras();
                objects.readonly.take();
                Ok(())
            })?;
            // A cached request survives hand replacement. Regenerate it to
            // validate the actual rules, including filters on public prompts.
            worker.with_rollback(|probe| {
                probe.advance_objects(&ExecutionBudget::default(), Some(1), false)?;
                let unchanged = probe.last_request.as_ref().is_some_and(|generated| {
                    generated.id == request.id
                        && generated.owner == request.owner
                        && generated.controlled == request.controlled
                        && generated.prompt == request.prompt
                        && generated.can_skip == request.can_skip
                        && generated.discard == request.discard
                });
                if !unchanged || probe.resolution_choices(player)? != root_choices {
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
        if checkpoints.is_some_and(|rows| {
            rows.keys()
                .any(|id| !observation.heroes.iter().any(|hero| hero.id == *id))
        }) {
            return Err(EngineError::InvalidInput(
                "upgrade checkpoints name an unknown hero".into(),
            ));
        }
        if self.planning.phase != Phase::Planning {
            return Err(unsupported("sampled worlds require planning"));
        }
        self.validate_sampled_effect_cards(&observation)?;
        let mut samples = Vec::new();
        for (index, hero) in self.world.heroes.iter().enumerate() {
            if hero.id == player || preserved.contains(&hero.id) {
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
                hand,
                fixed,
            });
        }
        Ok(samples)
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
            let round_marker = matches!(kind, "enraged" | "equilibrium")
                && effect.duration == crate::effects::Duration::ThisRound
                && effect.token.is_none()
                && effect.granted_passive.is_none();
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
            let sentinel = (kind == "delayed_trigger"
                && effect.duration == crate::effects::Duration::ThisTurn
                && effect.token.is_none()
                && effect.granted_passive.is_none())
                .then(|| effect.card.map(|card| self.symbols.name(card)))
                .flatten()
                .filter(|card| matches!(*card, "warning_shot" | "treetop_sentinel"));
            // Pre-discard triggers use the public board and hand count.
            // Any resulting discard choice runs against the sampled hand.
            let public = matches!(
                behavior.kind,
                Kind::PreMovement
                    | Kind::PreDiscard
                    | Kind::AttackImmunity
                    | Kind::PlacementPrevention
                    | Kind::MinionProtection
                    | Kind::MovementZone
                    | Kind::LosBlocker
                    | Kind::AreaStat
                    | Kind::MovementAura
                    | Kind::StaticBarrier
                    | Kind::Petrify
            ) || action_restriction || round_marker || reverse_time || public_color_trigger || sentinel.is_some();
            if !public {
                return Err(unsupported(format!(
                    "resolution sampling does not support {:?} effect {kind}",
                    behavior.kind,
                )));
            }
            // The dense projection omits finishing programs for arena sessions.
            // These programs can still retain references to hidden objects.
            let finishing = objects.finishing_list(node)?;
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
                && matches!(
                    (effect.definition.behavior.kind, self.symbols.name(kind)),
                    (crate::effect_rules::Kind::LosBlocker, "smoke_bomb")
                        | (crate::effect_rules::Kind::AreaStat, "ice" | "barrier")
                )
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
                session.install_planning_hand(objects, sample.hero, &sample.hand, sample.fixed)?;
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

    fn hero_card_list_bound(
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
            for (field, stamp) in fields.into_iter().zip(&mut stamps[1..]) {
                let Some(ObjectValue::Object(list)) =
                    objects.arena.field_ref(model, objects.phases.k(field))
                else {
                    return Err(unsupported("planning card collection missing"));
                };
                *stamp = objects
                    .arena
                    .page_stamp(*list)
                    .ok_or_else(|| unsupported("planning cards must be a list"))?;
            }
            let row = &self.world.heroes[index];
            let update = |field, old: &[CardId]| -> Result<Option<Vec<CardId>>> {
                let values = self.hero_card_values_bound(objects, model, field)?;
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
            let deck = update(Field::Deck, &row.deck)?;
            let hand = update(Field::Hand, &row.hand)?;
            let discard = update(Field::Discard, &row.discard)?;
            let spells = update(Field::Spells, &row.spells)?;
            let values = self.hero_card_values_bound(objects, model, Field::Played)?;
            let played = if values.len() == row.played.len()
                && values
                    .iter()
                    .zip(&row.played)
                    .all(|(value, card)| match (value, card) {
                        (ObjectValue::Atom(Datum::Instance(id)), Some(card)) => *id == card.0,
                        (ObjectValue::Atom(Datum::Null), None) => true,
                        _ => false,
                    }) {
                None
            } else {
                Some(self.decode_hero_cards(values, Field::Played)?)
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
    fn clear_planning_requests_bound(
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
            let mut samples = root.resolution_samples(request.owner, sample, &[39])
                .unwrap().remove(0);
            let hidden = samples.iter_mut()
                .find(|row| root.symbols.name(row.hero) == "hero_swift").unwrap();
            let hero = hidden.hero;
            let old_hand = root.world.hero(hero).unwrap().hand.clone();
            let replacement = root.with_live_objects(|session, objects| {
                let model = session.hero_model_bound(objects, hero).unwrap();
                let deck = session.hero_card_list_bound(objects, model, Field::Deck)?;
                Ok(deck.into_iter().flatten().find_map(|card| {
                    let id = session.printed_observation_card(card).unwrap();
                    (!old_hand.contains(&card) && !hidden.fixed.contains(&Some(card))
                        && !hidden.hand.cards.contains(&id)).then_some(id)
                }).unwrap())
            }).unwrap();
            hidden.hand.cards[0] = replacement;
            root.with_installed_resolution_samples(request.owner, samples, |worker| {
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
        let other = root
            .world
            .heroes
            .iter()
            .find(|hero| hero.id != owner)
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
