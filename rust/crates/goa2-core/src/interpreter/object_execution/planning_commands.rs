//! Live planning commands preserve source collection replacement and aliasing.
use super::super::{TeamKnowledgeMode, UpgradeCheckpoint};
use super::phases::Field;
use super::*;
use crate::cards::CardId;
use std::collections::BTreeMap;

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

    fn resolution_samples(
        &self,
        player: Symbol,
        sample: super::super::PlanningRolloutSample<'_>,
        seeds: &[u64],
    ) -> Result<Vec<Vec<PlanningHandSample>>> {
        let supported = self.last_request.as_ref().is_some_and(|request| {
            self.resolution_request_addresses(request, player)
                && request.controlled.is_none()
                && self.stack.last().is_some_and(|frame| {
                    match (&request.choices, &self.steps[frame.instruction].instruction) {
                        (RequestChoices::Action(_), Instruction::LiveCardAction)
                        | (RequestChoices::Defense(_), Instruction::LiveReaction)
                        | (RequestChoices::CardColors(_), Instruction::LiveChooseCardColor)
                        | (
                            RequestChoices::ConfirmAction,
                            Instruction::Flow(crate::turn::FlowStep::Confirm(_)),
                        )
                        | (
                            RequestChoices::Actors { .. },
                            Instruction::Flow(
                                crate::turn::FlowStep::Next | crate::turn::FlowStep::Tie(_),
                            ),
                        ) => true,
                        (RequestChoices::Hexes(_), Instruction::LiveHexSelect) => {
                            !request.discard
                                && frame.object.is_some_and(|step| {
                                    self.is_public_board_selector(step, player)
                                })
                        }
                        (RequestChoices::Numbers { values, .. }, Instruction::Number { .. }) => {
                            !request.discard
                                && values == &[1, 2]
                                && frame.object.is_some_and(|step| {
                                    self.is_public_number_selector(step)
                                })
                        }
                        _ => false,
                    }
                })
        });
        if self.planning.phase != Phase::Resolution || !supported {
            return Err(unsupported(
                "resolution sampling requires a supported public resolution prompt",
            ));
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
            if !hero.spells.is_empty()
                || protected
                    .iter()
                    .flatten()
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

    fn is_public_number_selector(&self, step: ObjectId) -> bool {
        let Some(objects) = &self.objects else {
            return false;
        };
        // These printed modes always offer the same two choices. Other
        // numeric requests can depend on private cards or saved operands.
        self.passive_text_bound(objects, step, Field::Output)
            .is_ok_and(|output| {
                matches!(
                    self.symbols.name(output),
                    "trap_choice" | "fg_choice" | "coin_choice" | "charge_choice"
                        | "plank_choice" | "soar_choice" | "melee_choice" | "design_choice"
                )
            })
    }

    fn is_public_board_selector(&self, step: ObjectId, player: Symbol) -> bool {
        let Some(next) = self.stack.iter().rev().nth(1) else {
            return false;
        };
        let instruction = &self.steps[next.instruction].instruction;
        let travel = matches!(instruction, Instruction::LiveTravel { sequence: false });
        let placement = matches!(
            instruction,
            Instruction::LiveTokenPlace | Instruction::LiveTurret { remove: false }
        );
        if !travel && !placement && !matches!(instruction, Instruction::LiveMove) {
            return false;
        }
        let Some(objects) = &self.objects else {
            return false;
        };
        let Some(ObjectValue::Object(filters)) =
            objects.arena.field(step, objects.phases.k(Field::Filters))
        else {
            return false;
        };
        let Some(filters) = objects.arena.sequence(filters) else {
            return false;
        };
        let mut kinds = filters.iter().map(|filter| {
            let ObjectValue::Object(filter) = filter else {
                return None;
            };
            match objects
                .arena
                .field(*filter, objects.phases.k(Field::FilterType))
            {
                Some(ObjectValue::Atom(Datum::Text(kind))) => Some(self.symbols.name(kind)),
                _ => None,
            }
        });
        if placement {
            let Some(continuation) = next.object else {
                return false;
            };
            let Ok(output) = self.passive_text_bound(objects, step, Field::Output) else {
                return false;
            };
            let token = self.passive_text_bound(objects, continuation, Field::TokenType)
                .ok().map(|kind| self.symbols.name(kind));
            let public_token = matches!(instruction, Instruction::LiveTurret { remove: false })
                || matches!(token, Some("smoke_bomb" | "tree" | "pyro" | "glitch"));
            if !public_token || !self.passive_text_bound(objects, continuation, Field::HexKey)
                .is_ok_and(|input| input == output)
            {
                return false;
            }
            if token == Some("glitch")
                && kinds.clone().next_back() == Some(Some("hex_batch_completable_filter"))
                && let Some(ObjectValue::Object(batch)) = filters.last()
            {
                use crate::spatial::HexFilter;

                kinds.next_back();
                return kinds.next_back() == Some(Some("occupied_filter"))
                    && kinds.next() == Some(Some("range_filter"))
                    && kinds.all(|kind| kind == Some("range_filter"))
                    && self.batch_slots_bound(objects, *batch).is_ok_and(|slots| {
                        !slots.is_empty() && slots.iter().all(|slot| {
                            matches!(slot.split_last(), Some((HexFilter::Obstacle { wanted: false, .. }, ranges))
                                if !ranges.is_empty() && ranges.iter().all(|filter| matches!(filter, HexFilter::Range { .. })))
                        })
                    });
            }
            return kinds.next() == Some(Some("range_filter"))
                && kinds.next() == Some(Some("occupied_filter"))
                && kinds.next().is_none();
        }
        if travel {
            let ([ObjectValue::Object(filter)], Some(continuation)) = (filters, next.object) else {
                return false;
            };
            let keys = &objects.phases;
            let ordinary_source =
                |object| match objects.arena.field(object, keys.k(Field::SourceCard)) {
                    None | Some(ObjectValue::Atom(Datum::Null)) => true,
                    Some(ObjectValue::Atom(Datum::Text(source))) => {
                        self.world.hero(player).is_some_and(|hero| {
                            [hero.current, hero.extra].into_iter().flatten().any(|id| {
                                self.world.card(id).is_some_and(|card| {
                                    !card.facedown
                                        && self.world.definitions.cards[card.definition].printed_id
                                            == source
                                })
                            })
                        })
                    }
                    _ => false,
                };
            return matches!(
                objects.arena.field(*filter, keys.k(Field::FilterType)),
                Some(ObjectValue::Atom(Datum::Text(kind)))
                    if self.symbols.name(kind) == "fast_travel_destination_filter"
            ) && ordinary_source(*filter)
                && ordinary_source(continuation);
        }
        kinds.next() == Some(Some("occupied_filter"))
            && kinds.next() == Some(Some("movement_path_filter"))
            && kinds.all(|kind| {
                matches!(
                    kind,
                    Some("in_straight_line_filter" | "straight_line_path_filter" | "range_filter")
                )
            })
    }

    fn with_installed_resolution_samples<T>(
        &mut self,
        player: Symbol,
        samples: Vec<PlanningHandSample>,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.with_rollback(|worker| {
            let root_choices = worker.resolution_choices(player)?;
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
            if worker.resolution_choices(player)? != root_choices {
                return Err(unsupported("sampled resolution changed the root choices"));
            }
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
            if !hero.spells.is_empty() || !public.revealed_current.is_empty() {
                return Err(unsupported(
                    "sampled world requires ordinary cards without current reveals",
                ));
            }
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
            let public = matches!(
                behavior.kind,
                Kind::PreMovement
                    | Kind::AttackImmunity
                    | Kind::PlacementPrevention
                    | Kind::MinionProtection
                    | Kind::MovementZone
                    | Kind::LosBlocker
                    | Kind::AreaStat
                    | Kind::MovementAura
                    | Kind::StaticBarrier
                    | Kind::Petrify
            ) || action_restriction;
            if !public {
                return Err(unsupported(format!(
                    "resolution sampling does not support {:?} effect {kind}",
                    behavior.kind,
                )));
            }
            // The dense projection omits finishing programs for arena sessions.
            // These programs can still retain references to hidden objects.
            let finishing = objects.finishing_list(node)?;
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
                    (observation.phase == Phase::Resolution
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
            let id = self
                .symbols
                .name(self.world.definitions.cards[row.definition].printed_id);
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
        for &(hero, model) in &objects.phases.heroes {
            let index = self
                .world
                .hero_index(hero)
                .ok_or_else(|| unsupported("planning hero cache missing"))?;
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
        }
        self.world.refresh_card_auras();
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
    fn sampled_resolution_accepts_public_glitch_batch_placement() {
        let fixture = fixture_case("object-execution-token-batch-card-unstable_timeline-False-first");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        root.advance_prefix(1).unwrap();
        let request = root.advance_prefix(1).unwrap().request.unwrap();
        assert_eq!(root.symbols.name(request.prompt), "Place glitch token 1/2");
        let step = root.stack.last().unwrap().object.unwrap();
        assert!(root.is_public_board_selector(step, request.owner));
    }

    #[test]
    fn sampled_worlds_require_token_effect_anchor_to_be_faceup() {
        let fixture = fixture_case("object-execution-min-primary-smoke_bomb-text-first");
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        root.advance_prefix(1).unwrap();
        let request = root.advance_prefix(1).unwrap().request.unwrap();
        let step = root.stack.last().unwrap().object.unwrap();
        assert!(root.is_public_board_selector(step, request.owner));
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
