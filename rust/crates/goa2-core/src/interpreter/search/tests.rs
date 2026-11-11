use super::{diagnostics::PlanningPair, *};
use crate::interpreter::{NativeRequest, RequestToken};
use crate::{geometry::Hex, world::EntityKind};

#[test]
fn sampled_reply_cap_prefers_greedy_commitments_and_full_search_keeps_every_reply() {
    use std::collections::HashSet;

    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let root = NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
        .unwrap();
    let player = root.world.heroes[0].id;
    let seed = 39;
    let sampled = root.sampled_planning_world(player, seed).unwrap();
    let first = sampled
        .joint_plans_with_reply_order(
            player,
            false,
            1,
            None,
            JointPlanOrder::OpponentReplies(seed),
            &[],
        )
        .unwrap();
    let reply = &first.plans[0];
    assert!(!reply.is_empty());
    for choice in reply {
        assert_eq!(
            Some(*choice),
            sampled
                .greedy_opponent_commitment(choice.hero, seed)
                .unwrap()
        );
    }
    let ordinary = sampled.joint_plans(player, false, 4096).unwrap();
    let ranked = sampled
        .joint_plans_with_reply_order(
            player,
            false,
            4096,
            None,
            JointPlanOrder::OpponentReplies(seed),
            &[],
        )
        .unwrap();
    assert_eq!(ranked.total, ordinary.total);
    assert_eq!(ranked.plans.len(), ranked.total);
    assert_eq!(
        ranked.plans.into_iter().collect::<HashSet<_>>(),
        ordinary.plans.into_iter().collect::<HashSet<_>>()
    );
}

#[test]
fn joint_card_order_uses_score_gaps_and_keeps_unscored_cards() {
    let choice = PlanningCommitment {
        hero: Symbol(0),
        first: None,
        second: None,
    };
    let dimensions = [vec![choice; 2], vec![choice; 3]];
    let costs = [
        Some(vec![
            greedy_card_cost(0, Some(0)),
            greedy_card_cost(0, Some(-150)),
        ]),
        Some(vec![
            greedy_card_cost(0, Some(0)),
            greedy_card_cost(0, Some(-30)),
            greedy_card_cost(0, None),
        ]),
    ];
    assert_eq!(
        ranked_reply_indices(&dimensions, &[1, 2], &costs, 6),
        [0, 2, 1, 3, 4, 5]
    );
}

#[test]
fn shared_rows_average_likelihoods_across_worlds() {
    use crate::cards::CardId;

    let hero = Symbol(0);
    let a = Some(CardId(1));
    let b = Some(CardId(2));
    let rows = [a, b].map(|first| {
        vec![PlanningCommitment {
            hero,
            first,
            second: None,
        }]
    });
    let first = shared_row_likelihoods(&rows, &[(hero, vec![(a, 0), (b, -300)])]);
    let second = shared_row_likelihoods(&rows, &[(hero, vec![(a, -100), (b, 0)])]);
    assert!(first[0] > first[1] && second[0] < second[1]);
    assert!(first[0] + second[0] > first[1] + second[1]);
    assert_eq!(shared_row_likelihoods(&rows, &[]), [0.5, 0.5]);
}

#[test]
fn four_turn_greedy_scores_order_both_sides_of_sampled_planning() {
    std::thread::Builder::new()
        .stack_size(SEARCH_WORKER_STACK)
        .spawn(|| {
            use std::collections::HashSet;
            let fixture = super::super::object_execution::fixture_case(
                "object-execution-card-planning-cycle",
            );
            let root = NativeSession::compile(
                serde_json::from_value(fixture["envelope"].clone()).unwrap(),
            )
            .unwrap();
            let player = root.world.heroes[0].id;
            let seed = 39;
            let mut world = root.sampled_planning_world(player, seed).unwrap();
            let before = world.semantic_state().unwrap();
            let (scores, nodes, _) = world
                .greedy_planning_scores(seed, 1024, false, &ExecutionBudget::default())
                .unwrap();
            assert!(nodes > 0 && nodes <= 1024);
            assert_eq!(scores.len(), world.world.heroes.len());
            assert_eq!(world.semantic_state().unwrap(), before);
            for friendly in [true, false] {
                let ordinary = world.joint_plans(player, friendly, 4096).unwrap();
                let ranked = world
                    .joint_plans_with_reply_order(
                        player,
                        friendly,
                        4096,
                        None,
                        JointPlanOrder::PlayerFirst,
                        &scores,
                    )
                    .unwrap();
                for commitment in &ranked.plans[0] {
                    let (_, row) = scores
                        .iter()
                        .find(|(hero, _)| *hero == commitment.hero)
                        .unwrap();
                    assert!(row.iter().any(|(card, score)| {
                        *card == commitment.first
                            && Some(*score) == row.iter().map(|(_, score)| *score).max()
                    }));
                }
                assert_eq!(
                    ranked.plans.into_iter().collect::<HashSet<_>>(),
                    ordinary.plans.into_iter().collect::<HashSet<_>>()
                );
            }
            let mut outcomes = Vec::new();
            for workers in [1, 2] {
                let result = world
                    .search_plans_on_sample(
                        player,
                        seed,
                        2,
                        SearchLimits {
                            max_depth: 1,
                            max_nodes: 4096,
                            opponent_policy: OpponentPolicy::Adversarial,
                            evaluation: SearchEvaluation::MaterialPosition,
                        },
                        PlanningSearchOptions {
                            rollout_turns: Some(2),
                            workers,
                            upgrade_checkpoints: None,
                            reply_cap: None,
                            reply_probe_cap: None,
                            greedy_card_order: true,
                            tactical_forecast: false,
                            forecast_node_share: ForecastNodeShare::default(),
                            legacy_fixed_followup: false,
                            fixed_future_planning: false,
                            branch_future_planning: false,
                        },
                        &ExecutionBudget::default(),
                    )
                    .unwrap();
                assert!(result.matrix.score.is_some());
                assert!(result.matrix.forecast_checks > 0);
                assert!(result.matrix.nodes <= 4096);
                assert_eq!(world.semantic_state().unwrap(), before);
                outcomes.push((result.commitment, result.matrix.score));
            }
            assert_eq!(outcomes[0], outcomes[1]);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn forecast_search_uses_one_repeatable_opponent_reply() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut world =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let player = world.world.heroes[0].id;
    let budget = ExecutionBudget::default();
    let before = world.semantic_state().unwrap();
    let options = PlanningSearchOptions {
        rollout_turns: Some(2),
        workers: 1,
        upgrade_checkpoints: None,
        reply_cap: Some(4),
        reply_probe_cap: None,
        greedy_card_order: true,
        tactical_forecast: false,
        forecast_node_share: ForecastNodeShare::default(),
        legacy_fixed_followup: false,
        fixed_future_planning: false,
        branch_future_planning: false,
    };
    let limits = SearchLimits {
        max_depth: 1,
        max_nodes: 4096,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::MaterialPosition,
    };
    let first = world
        .search_plans_on_sample(player, 17, 3, limits, options, &budget)
        .unwrap();
    let second = world
        .search_plans_on_sample(player, 17, 3, limits, options, &budget)
        .unwrap();
    assert_eq!(first.total_opposing_plans, 1);
    assert_eq!(first.matrix.total_cells, 3);
    assert!(first.matrix.score.is_some());
    assert!(first.matrix.forecast_checks > 0);
    assert_eq!(first.commitment, second.commitment);
    assert_eq!(first.matrix.score, second.matrix.score);
    let legacy = world
        .search_plans_on_sample(
            player,
            17,
            3,
            limits,
            PlanningSearchOptions {
                legacy_fixed_followup: true,
                ..options
            },
            &budget,
        )
        .unwrap();
    assert_eq!(legacy.total_opposing_plans, 1);
    assert!(legacy.matrix.score.is_some());
    assert!(legacy.matrix.forecast_checks > 0);
    assert_eq!(world.semantic_state().unwrap(), before);
}

#[test]
fn later_planning_expansion_matches_fixed_rollout_matrix_and_requires_an_exact_value() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut world =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let player = world.world.heroes[0].id;
    let team = world.world.heroes[0].team;
    let budget = ExecutionBudget::default();
    let before = world.semantic_state().unwrap();

    let preferred: Vec<_> = world
        .world
        .heroes
        .iter()
        .filter_map(|hero| world.first_planning_commitment(hero.id).unwrap())
        .collect();
    let alternative = |friendly| {
        let (index, choice) = preferred
            .iter()
            .enumerate()
            .find(|(_, choice)| (world.world.hero(choice.hero).unwrap().team == team) == friendly)
            .unwrap();
        let next = world
            .hero_planning_commitments(choice.hero)
            .unwrap()
            .into_iter()
            .find(|next| next.first != choice.first)
            .unwrap();
        (index, next)
    };
    let row = alternative(true);
    let reply = alternative(false);
    let mut scores = [[0; 2]; 2];
    let mut costs = [[0; 2]; 2];
    for (i, row) in [None, Some(row)].into_iter().enumerate() {
        for (j, reply) in [None, Some(reply)].into_iter().enumerate() {
            let mut joint = preferred.clone();
            for (index, choice) in row.into_iter().chain(reply) {
                joint[index] = choice;
            }
            let leaf = world
                .with_joint_planning(&joint, |branch| {
                    branch.rollout_game_turns(player, 1, 4096, &budget)
                })
                .unwrap()
                .unwrap();
            scores[i][j] = leaf.score;
            costs[i][j] = leaf.nodes + 1;
        }
    }
    let first_row = scores[0][0].min(scores[0][1]);
    let expected = first_row.max(scores[1][0].min(scores[1][1]));
    for (limit, score) in [
        (costs[0][0], None),
        (costs[0].iter().sum(), None),
        (costs[0].iter().sum::<usize>() + 1, None),
        (costs.iter().flatten().sum(), Some(expected)),
    ] {
        let mut search = Search {
            player,
            team,
            budget: &budget,
            nodes: 0,
            max_nodes: limit,
            opponent_policy: OpponentPolicy::Adversarial,
            evaluation: SearchEvaluation::MaterialPosition,
            turn_cutoff: None,
            controlled: None,
            shared_nodes: None,
            planning_sample: None,
            guard_hidden: false,
            allow_simulated_upgrades: false,
            hand_belief: None,
            forecast_changes: 0,
            forecast_checks: 0,
            forecast_matches: 0,
            future_planning: None,
        };
        let cutoff = world.search_turn_index() + 1;
        let result = search
            .expand_rollout_planning(&mut world, cutoff, false, false, &[vec![]])
            .unwrap();
        assert_eq!(result.map(|leaf| leaf.score), score);
        assert!(search.nodes <= limit);
        if score.is_none() {
            assert_eq!(search.nodes, limit);
        }
        assert_eq!(world.semantic_state().unwrap(), before);
    }
    let (row, replies): (Vec<_>, Vec<_>) = preferred
        .into_iter()
        .partition(|choice| world.world.hero(choice.hero).unwrap().team == team);
    let mut costs = Vec::new();
    for expand in [false, true] {
        let result = world
            .search_planning_matrix_partition(
                player,
                std::slice::from_ref(&row),
                std::slice::from_ref(&replies),
                SearchLimits {
                    max_depth: 0,
                    max_nodes: 4096,
                    opponent_policy: OpponentPolicy::Adversarial,
                    evaluation: SearchEvaluation::MaterialPosition,
                },
                PlanningPartition {
                    rollout_turns: Some(2),
                    controlled: None,
                    shared_nodes: None,
                    greedy_forecasts: &[],
                    future_branching: Some(expand),
                    legacy_fixed_followup: false,
                    fixed_opponent_seed: None,
                },
                &budget,
            )
            .unwrap();
        assert!(result.score.is_some());
        assert_eq!(result.forecast_checks, 0);
        assert_eq!(world.semantic_state().unwrap(), before);
        costs.push(result.nodes);
    }
    assert!(costs[1] > costs[0]);
}

#[test]
fn resolution_probe_distinguishes_duplicate_prefixes_from_forced_inputs() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut worker =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let player = worker.world.heroes[0].id;
    worker.last_request = Some(NativeRequest {
        controlled: None,
        id: RequestToken::Generated(1),
        owner: player,
        prompt: player,
        movement_range: None,
        pre_action_range: None,
        choices: RequestChoices::Numbers {
            values: vec![1, 1, 2],
            labels: Default::default(),
        },
        can_skip: false,
        discard: false,
    });
    let full = worker.resolution_choices(player).unwrap();
    assert_eq!(full.len(), 2);
    assert_eq!(worker.resolution_branch_probe(player).unwrap(), full);
    worker.last_request.as_mut().unwrap().choices = RequestChoices::ConfirmAction;
    assert_eq!(worker.resolution_branch_probe(player).unwrap().len(), 1);
}

#[test]
fn lab_search_evaluation_matches_fixed_point_reference() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut worker =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let player = worker.world.heroes[0].id;
    let team = worker.world.heroes[0].team;
    worker.world.heroes[0].gold += 7;
    let budget = ExecutionBudget::default();
    let search = Search {
        player,
        team,
        budget: &budget,
        nodes: 0,
        max_nodes: 1,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::LabAttackRangeV1,
        turn_cutoff: None,
        controlled: None,
        shared_nodes: None,
        guard_hidden: false,
        allow_simulated_upgrades: false,
        hand_belief: None,
        planning_sample: None,
        future_planning: None,
        forecast_changes: 0,
        forecast_checks: 0,
        forecast_matches: 0,
    };
    let expected =
        (worker.evaluate_lab_attack_range(player).unwrap() * (WIN - 1) as f64).round() as i64;
    assert_eq!(search.score(&worker).unwrap(), expected);
    assert_ne!(expected, worker.evaluate_resolution(team, player));
}

#[test]
fn alpha_beta_matches_exhaustive_live_attack_and_defense() {
    fn exhaustive(
        session: &mut NativeSession,
        player: Symbol,
        team: Symbol,
        depth: u8,
        branches: &mut [usize; 2],
        budget: &ExecutionBudget<'_>,
    ) -> Result<i64> {
        if matches!(session.planning.phase, Phase::Planning | Phase::GameOver) {
            return Ok(session.evaluate_resolution(team, player));
        }
        let Some((owner, owner_team)) = session.search_player()? else {
            return Ok(session.evaluate_resolution(team, player));
        };
        let moves = session.resolution_choices(owner)?;
        assert!(!moves.is_empty());
        let branching = moves.len() > 1;
        if branching && depth == 0 {
            return Ok(session.evaluate_resolution(team, player));
        }
        let maximizing = team == owner_team;
        if branching {
            branches[usize::from(!maximizing)] += 1;
        }
        let mut scores = Vec::new();
        for choice in moves {
            let DecisionMove::Input {
                request: Some(token),
                selection,
                ..
            } = choice
            else {
                panic!("expected a live resolution input");
            };
            scores.push(session.with_rollback(|child| {
                child.submit(token, selection)?;
                child.last_request = None;
                child.advance_to_decision(budget)?;
                exhaustive(
                    child,
                    player,
                    team,
                    depth - u8::from(branching),
                    branches,
                    budget,
                )
            })?);
        }
        Ok(if maximizing {
            scores.into_iter().max().unwrap()
        } else {
            scores.into_iter().min().unwrap()
        })
    }

    let budget = ExecutionBudget::default();
    let mut branches = [0; 2];
    for name in [
        "object-execution-attack-macro-optional-first",
        "object-execution-attack-macro-existing",
    ] {
        let fixture = super::super::object_execution::fixture_case(name);
        let mut session =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        session.advance_to_decision(&budget).unwrap();
        let player = session.world.heroes[0].id;
        let team = session.world.heroes[0].team;
        let state = session.semantic_state().unwrap();
        let request = session.last_request.clone();
        for depth in 0..=3 {
            let expected =
                exhaustive(&mut session, player, team, depth, &mut branches, &budget).unwrap();
            let mut search = Search {
                player,
                team,
                budget: &budget,
                nodes: 0,
                max_nodes: 2_048,
                opponent_policy: OpponentPolicy::Adversarial,
                evaluation: SearchEvaluation::MaterialPosition,
                turn_cutoff: None,
                controlled: None,
                shared_nodes: None,
                guard_hidden: false,
                allow_simulated_upgrades: false,
                hand_belief: None,
                planning_sample: None,
                forecast_changes: 0,
                forecast_checks: 0,
                forecast_matches: 0,
                future_planning: None,
            };
            let actual = session
                .with_rollback(|branch| search.value(branch, depth, i64::MIN, i64::MAX))
                .unwrap()
                .expect("small fixture must fit the node budget");
            assert_eq!(actual.score, expected, "{name}, depth {depth}");
            assert_eq!(session.semantic_state().unwrap(), state);
            assert_eq!(session.last_request, request);
        }
    }
    assert!(branches.into_iter().all(|count| count > 0), "{branches:?}");
}

#[test]
fn exact_terminal_root_score_can_be_reused_without_expanding_nodes() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-confirmation-repeated-step");
    let mut worker =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let budget = ExecutionBudget {
        cancelled: Some(&cancelled),
        ..ExecutionBudget::default()
    };
    let player = worker.advance(&budget).unwrap().request.unwrap().owner;
    let team = worker.world.hero(player).unwrap().team;
    let moves = worker.resolution_choices(player).unwrap();
    assert_eq!(moves.len(), 1);
    let mut search = Search {
        player,
        team,
        budget: &budget,
        nodes: 0,
        max_nodes: 1,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::MaterialPosition,
        turn_cutoff: None,
        controlled: None,
        shared_nodes: None,
        guard_hidden: false,
        allow_simulated_upgrades: false,
        hand_belief: None,
        planning_sample: None,
        future_planning: None,
        forecast_changes: 0,
        forecast_checks: 0,
        forecast_matches: 0,
    };
    let mut cache = [None];
    let shallow = search
        .choose(&mut worker, &moves, 1, 0, &mut cache)
        .unwrap();
    assert!(shallow.interrupted == Some(ResolutionStop::NodeLimit) && !shallow.resolved);
    assert_eq!(cache, [None]);
    search.nodes = 0;
    search.max_nodes = 2;
    let solved = search
        .choose(&mut worker, &moves, 1, 0, &mut cache)
        .unwrap();
    assert!(solved.resolved);
    assert_eq!(search.nodes, 2);
    let reused = search
        .choose(&mut worker, &moves, 3, 0, &mut cache)
        .unwrap();
    assert!(reused.resolved);
    assert_eq!(reused.best, solved.best);
    assert_eq!(search.nodes, 2);

    cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    let interrupted = search
        .choose(&mut worker, &moves, 3, 0, &mut cache)
        .unwrap();
    assert_eq!(interrupted.interrupted, Some(ResolutionStop::Cancelled));
    assert!(interrupted.best.is_none());
    assert!(!interrupted.resolved);
    assert_eq!(search.nodes, 2);

    let expired = ExecutionBudget {
        deadline: Some(std::time::Instant::now()),
        ..ExecutionBudget::default()
    };
    search.budget = &expired;
    let interrupted = search
        .choose(&mut worker, &moves, 3, 0, &mut cache)
        .unwrap();
    assert_eq!(interrupted.interrupted, Some(ResolutionStop::Deadline));
    assert!(interrupted.best.is_none());
    assert!(!interrupted.resolved);
    assert_eq!(search.nodes, 2);
}

#[test]
fn fixed_rollout_crosses_planning_and_restores_the_root_on_budget_exhaustion() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut worker =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let player = worker.world.heroes[0].id;
    let before = worker.semantic_state().unwrap();
    let budget = ExecutionBudget::default();
    assert_eq!(
        worker.rollout_game_turns(player, 2, 1, &budget).unwrap(),
        None
    );
    assert_eq!(worker.semantic_state().unwrap(), before);
    let one = worker
        .rollout_game_turns(player, 1, 512, &budget)
        .unwrap()
        .unwrap();
    let two = worker
        .rollout_game_turns(player, 2, 512, &budget)
        .unwrap()
        .unwrap();
    let origin = worker.search_turn_index();
    assert_eq!(i128::from(one.round) * 4 + i128::from(one.turn), origin + 1);
    assert_eq!(i128::from(two.round) * 4 + i128::from(two.turn), origin + 2);
    assert!(two.nodes > one.nodes);
    assert_eq!(worker.semantic_state().unwrap(), before);
}

#[test]
fn movement_root_order_explores_approaches_and_retreats() {
    let fixture = super::super::object_execution::fixture_case(
        "object-execution-live-movement-sequence-base-plain-normal",
    );
    let mut root =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    root.advance_prefix(1).unwrap();
    root.advance_prefix(1).unwrap();
    let (owner, team) = root.search_player().unwrap().unwrap();
    let original = root.resolution_choices(owner).unwrap();
    assert!(original.len() >= 3);
    let mut ordered = original.clone();
    root.order_movement_roots(owner, &mut ordered);
    assert_eq!(ordered.len(), original.len());
    assert!(original.iter().all(|choice| ordered.contains(choice)));
    let distance = |choice: &DecisionMove, kinds: &[EntityKind]| {
        let DecisionMove::Input { selection, .. } = choice else {
            return None;
        };
        let hex = super::super::datum_hex(selection, &root.symbols, root.coordinates).ok()?;
        root.world
            .locations
            .iter()
            .filter_map(|&(id, enemy)| {
                let entity = root.world.entity(id)?;
                (entity.team.is_some_and(|owner| owner != team) && kinds.contains(&entity.kind))
                    .then_some(hex.distance(enemy))
            })
            .min()
    };
    for kinds in [
        &[EntityKind::Minion][..],
        &[EntityKind::Hero, EntityKind::Piece][..],
    ] {
        let closest = original
            .iter()
            .filter_map(|choice| distance(choice, kinds))
            .min();
        assert!(closest.is_some());
        assert!(
            ordered[..3]
                .iter()
                .any(|choice| distance(choice, kinds) == closest)
        );
    }
    let threats = [EntityKind::Minion, EntityKind::Hero, EntityKind::Piece];
    let farthest = original
        .iter()
        .filter_map(|choice| distance(choice, &threats))
        .max();
    assert!(farthest.is_some());
    assert!(
        ordered[..3]
            .iter()
            .any(|choice| distance(choice, &threats) == farthest)
    );
}

#[test]
fn movement_extension_retains_first_legal_and_charges_each_probe() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut root =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let plan: Vec<_> = root
        .world
        .heroes
        .iter()
        .map(|hero| PlanningCommitment {
            hero: hero.id,
            first: Some(hero.hand[1]),
            second: None,
        })
        .collect();
    let budget = ExecutionBudget::default();
    root.with_joint_planning(&plan, |worker| {
        worker.advance_to_decision(&budget)?;
        let (owner, team) = worker.search_player()?.unwrap();
        let choices = worker.resolution_choices(owner)?;
        let (token, action) = choices
            .into_iter()
            .find_map(|choice| match choice {
                DecisionMove::Input {
                    request: Some(token),
                    selection: super::super::Datum::Text(id),
                    ..
                } if worker.symbols.name(id) == "FAST_TRAVEL" => Some((token, id)),
                _ => None,
            })
            .expect("opening actor can Fast Travel");
        worker.submit(token, super::super::Datum::Text(action))?;
        worker.last_request = None;
        worker.advance_to_decision(&budget)?;
        assert!(worker.movement_root_actor().is_some());
        assert!(worker.resolution_choices(owner)?.len() > 5);
        let before = worker.semantic_state()?;
        let request = worker.last_request.clone();
        let mut search = Search {
            player: owner,
            team,
            budget: &budget,
            nodes: 0,
            max_nodes: 1024,
            opponent_policy: OpponentPolicy::Adversarial,
            evaluation: SearchEvaluation::MaterialPosition,
            turn_cutoff: worker.search_turn_cutoff(Some(1))?,
            controlled: None,
            shared_nodes: None,
            guard_hidden: false,
            allow_simulated_upgrades: false,
            hand_belief: None,
            planning_sample: None,
            future_planning: None,
            forecast_changes: 0,
            forecast_checks: 0,
            forecast_matches: 0,
        };
        let reference = worker
            .with_rollback(|branch| search.value(branch, 0, i64::MIN, i64::MAX))?
            .unwrap();
        let reference_nodes = search.nodes;
        search.opponent_policy = OpponentPolicy::AdversarialMovementTargets;
        search.nodes = 0;
        let extended = worker
            .with_rollback(|branch| search.value(branch, 0, i64::MIN, i64::MAX))?
            .unwrap();
        assert!(extended.score >= reference.score);
        assert!(search.nodes > reference_nodes);
        assert!(!extended.resolved);

        search.nodes = 0;
        search.max_nodes = 1;
        assert!(
            worker
                .with_rollback(|branch| search.value(branch, 0, i64::MIN, i64::MAX))?
                .is_none()
        );
        assert_eq!(search.nodes, 1);
        assert_eq!(worker.semantic_state()?, before);
        assert_eq!(worker.last_request, request);
        Ok(())
    })
    .unwrap();
}

#[test]
fn mandatory_upgrade_search_retains_a_scored_legal_choice_at_the_node_limit() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-cleanup-live-level-gold-3");
    let mut session =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    session.advance_prefix(1).unwrap();
    session.advance_prefix(1).unwrap().request.unwrap();
    let (player, _) = session.search_player().unwrap().unwrap();
    let legal = session.resolution_choices(player).unwrap();
    assert!(legal.len() > 1);
    let before = session.semantic_state().unwrap();
    let result = session
        .search_resolution(
            player,
            SearchLimits {
                max_depth: 1,
                max_nodes: 1,
                opponent_policy: OpponentPolicy::Adversarial,
                evaluation: SearchEvaluation::MaterialPosition,
            },
            &ExecutionBudget::default(),
        )
        .unwrap();
    assert_eq!(result.stop, ResolutionStop::NodeLimit);
    assert_eq!(result.nodes, 1);
    assert_eq!(result.score_kind, Some(ResolutionScoreKind::Immediate));
    assert!(legal.contains(&result.decision));
    assert_eq!(session.semantic_state().unwrap(), before);
}

#[test]
fn resolution_search_scores_root_moves_at_the_game_turn_cutoff() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut worker =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let player = worker.world.heroes[0].id;
    let mut plan = worker.joint_plans(player, true, 1).unwrap().plans.remove(0);
    plan.extend(
        worker
            .joint_plans(player, false, 1)
            .unwrap()
            .plans
            .remove(0),
    );
    let budget = ExecutionBudget::default();
    worker
        .with_joint_planning(&plan, |worker| {
            worker.advance_to_decision(&budget)?;
            let (owner, _) = worker.search_player()?.unwrap();
            let choices = worker.resolution_choices(owner)?;
            let before = worker.semantic_state()?;
            let limits = SearchLimits {
                max_depth: 1,
                max_nodes: 4096,
                opponent_policy: OpponentPolicy::Adversarial,
                evaluation: SearchEvaluation::MaterialPosition,
            };
            let ordinary = worker.search_resolution(owner, limits, &budget)?;
            let sampled_root = worker.search_resolution_in_sampled_world(
                owner,
                PlanningRolloutSample {
                    seed: 39,
                    upgrade_checkpoints: None,
                },
                limits,
                &budget,
            )?;
            assert!(choices.contains(&sampled_root.decision));
            assert!(sampled_root.score.is_some());
            assert_eq!(worker.semantic_state()?, before);
            let tiny = SearchLimits {
                max_nodes: 1,
                ..limits
            };
            let fallback =
                worker.search_resolution_with_turn_rollout(owner, tiny, Some(2), &budget)?;
            assert!(fallback.score.is_some());
            assert_eq!(fallback.score_kind, Some(ResolutionScoreKind::Immediate));
            assert_eq!((fallback.completed_depth, fallback.nodes), (0, 1));
            assert_eq!(fallback.stop, ResolutionStop::NodeLimit);
            let cancelled = std::sync::atomic::AtomicBool::new(true);
            for (blocked, reason) in [
                (
                    ExecutionBudget {
                        deadline: Some(std::time::Instant::now()),
                        ..budget
                    },
                    ResolutionStop::Deadline,
                ),
                (
                    ExecutionBudget {
                        cancelled: Some(&cancelled),
                        ..budget
                    },
                    ResolutionStop::Cancelled,
                ),
            ] {
                let result = worker.search_resolution(owner, limits, &blocked)?;
                assert_eq!(result.stop, reason);
                assert_eq!(result.score, None);
                assert_eq!(result.nodes, 0);
                assert!(choices.contains(&result.decision));
            }
            assert!(choices.contains(&fallback.decision));
            assert_eq!(worker.semantic_state()?, before);
            assert!(choices.len() > 1);
            let one_turn =
                worker.search_resolution_with_turn_rollout(owner, limits, Some(1), &budget)?;
            let team = worker.world.hero(owner).unwrap().team;
            let request = worker.last_request.clone();
            let mut seed = Search {
                player: owner,
                team,
                budget: &budget,
                nodes: 0,
                max_nodes: limits.max_nodes,
                opponent_policy: limits.opponent_policy,
                evaluation: limits.evaluation,
                turn_cutoff: worker.search_turn_cutoff(Some(2))?,
                controlled: None,
                shared_nodes: None,
                guard_hidden: false,
                allow_simulated_upgrades: false,
                hand_belief: None,
                planning_sample: None,
                future_planning: None,
                forecast_changes: 0,
                forecast_checks: 0,
                forecast_matches: 0,
            };
            let mut ordered = choices.clone();
            worker.order_movement_roots(owner, &mut ordered);
            let (index, score) = seed
                .seed_short_turn(std::slice::from_mut(worker), &ordered)?
                .expect("one-turn pass should fit the seed budget");
            assert_eq!(ordered[index], one_turn.decision);
            assert_eq!(Some(score), one_turn.score);
            assert!(seed.nodes >= choices.len() && seed.nodes <= limits.max_nodes / 8);
            assert_eq!(worker.semantic_state()?, before);
            assert_eq!(worker.last_request, request);

            let rolled =
                worker.search_resolution_with_turn_rollout(owner, limits, Some(2), &budget)?;
            assert!(rolled.score.is_some());
            assert_eq!(rolled.score_kind, Some(ResolutionScoreKind::TurnRollout));
            assert_eq!(rolled.completed_depth, 1);
            assert!(rolled.nodes > ordinary.nodes);
            assert!(choices.contains(&rolled.decision));
            assert_eq!(worker.semantic_state()?, before);
            for turns in [2, 3] {
                let sampled = worker.search_resolution_with_sampled_turn_rollout(
                    owner,
                    limits,
                    Some(turns),
                    Some(PlanningRolloutSample {
                        seed: 39,
                        upgrade_checkpoints: None,
                    }),
                    &budget,
                )?;
                assert_eq!(sampled.score_kind, Some(ResolutionScoreKind::TurnRollout));
                assert_eq!(sampled.completed_depth, 1);
                assert!(choices.contains(&sampled.decision));
                assert_eq!(worker.semantic_state()?, before);
            }
            let invalid_evidence = std::collections::BTreeMap::from([("unknown".into(), vec![])]);
            let invalid = worker.search_resolution_with_sampled_turn_rollout(
                owner,
                limits,
                Some(2),
                Some(PlanningRolloutSample {
                    seed: 39,
                    upgrade_checkpoints: Some(&invalid_evidence),
                }),
                &budget,
            );
            assert!(matches!(invalid, Err(EngineError::InvalidInput(_))));
            let root_sample = PlanningRolloutSample {
                seed: 39,
                upgrade_checkpoints: None,
            };
            for workers in [1, 2] {
                let fallback = worker.search_resolution_worlds(
                    owner,
                    (root_sample, 2, workers),
                    SearchLimits {
                        max_nodes: 2,
                        ..limits
                    },
                    Some(2),
                    None,
                    &budget,
                )?;
                assert!(choices.contains(&fallback.decision));
                assert!(fallback.score.is_some());
                assert_eq!(fallback.score_kind, Some(ResolutionScoreKind::Immediate));
                assert_eq!((fallback.completed_depth, fallback.nodes), (0, 2));
                assert_eq!(fallback.stop, ResolutionStop::NodeLimit);
                assert_eq!(worker.semantic_state()?, before);
            }
            let serial = worker.search_resolution_worlds(
                owner,
                (root_sample, 2, 1),
                limits,
                Some(2),
                None,
                &budget,
            )?;
            let parallel = worker.search_resolution_worlds(
                owner,
                (root_sample, 2, 2),
                limits,
                Some(2),
                None,
                &budget,
            )?;
            assert_eq!(serial.completed_depth, 1);
            assert_eq!(serial.score_kind, Some(ResolutionScoreKind::TurnRollout));
            assert_eq!(serial.completed_depth, parallel.completed_depth);
            assert_eq!(serial.decision, parallel.decision);
            assert_eq!(serial.score, parallel.score);
            assert_eq!(worker.semantic_state()?, before);
            let both = worker.search_sampled_resolution_with_turn_rollout(
                owner,
                root_sample,
                limits,
                Some(2),
                Some(root_sample),
                &budget,
            )?;
            assert_eq!(both.score_kind, Some(ResolutionScoreKind::TurnRollout));
            assert_eq!(both.completed_depth, 1);
            assert!(choices.contains(&both.decision));
            assert_eq!(worker.semantic_state()?, before);
            let lab = worker.search_resolution_with_turn_rollout(
                owner,
                SearchLimits {
                    evaluation: SearchEvaluation::LabAttackRangeV1,
                    ..limits
                },
                Some(2),
                &budget,
            )?;
            assert!(choices.contains(&lab.decision));
            assert_eq!(lab.completed_depth, 1);
            assert_eq!(lab.score_kind, Some(ResolutionScoreKind::TurnRollout));
            assert!(lab.score.is_some());
            assert_eq!(worker.semantic_state()?, before);
            Ok(())
        })
        .unwrap();
}

#[test]
fn arena_v2_interrupts_both_policies_at_the_same_shared_cap() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut root =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let player = root.world.heroes[0].id;
    let budget = ExecutionBudget::default();
    let scope = BotPlanningScope {
        seed: 39,
        bots: &[player],
        worlds: 2,
        mode: super::super::TeamKnowledgeMode::TrueRules,
    };
    let options = PlanningSearchOptions {
        rollout_turns: Some(4),
        workers: 1,
        upgrade_checkpoints: None,
        reply_cap: Some(4),
        reply_probe_cap: None,
        greedy_card_order: true,
        tactical_forecast: false,
        forecast_node_share: ForecastNodeShare::default(),
        legacy_fixed_followup: false,
        fixed_future_planning: false,
        branch_future_planning: false,
    };
    let limits = SearchLimits {
        max_depth: 3,
        max_nodes: 32,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::LanePressureV3,
    };
    let run = || root.search_node_limited_planning(player, scope, 8, limits, options, &budget);
    let planned = run().unwrap();
    assert_eq!(planned, run().unwrap());
    assert_eq!(planned.matrix.nodes, limits.max_nodes);
    assert!(planning_has_score(&planned));
    assert_eq!(planned.commitments.len(), 1);

    let mut joint = root.joint_plans(player, true, 1).unwrap().plans.remove(0);
    joint.extend(root.joint_plans(player, false, 1).unwrap().plans.remove(0));
    root.apply_joint_planning_ordered(&joint).unwrap();
    root.advance_to_decision(&budget).unwrap();
    let player = root.search_player().unwrap().unwrap().0;
    let legal = root.resolution_choices(player).unwrap();
    let before = root.semantic_state().unwrap();
    let result = root
        .search_node_limited_resolution(
            player,
            PlanningRolloutSample {
                seed: 39,
                upgrade_checkpoints: None,
            },
            2,
            &[player],
            false,
            limits,
            4,
            &budget,
        )
        .unwrap();
    assert_eq!(result.nodes, limits.max_nodes);
    assert_eq!(result.stop, ResolutionStop::NodeLimit);
    assert!(result.score.is_some());
    assert!(legal.contains(&result.decision));
    assert_eq!(root.semantic_state().unwrap(), before);
}

#[test]
fn accumulated_team_gold_preserves_upgrade_spend_and_counts_the_opposing_team() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut root =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let player = root.world.heroes[0].id;
    let team = root.world.heroes[0].team;
    let enemy = root
        .world
        .heroes
        .iter()
        .find(|hero| hero.team != team)
        .unwrap()
        .id;
    let hero = root.world.hero_row_mut(player).unwrap();
    hero.level = 1;
    hero.gold = 9;
    let score = |session: &NativeSession, evaluation| {
        session.search_score(evaluation, team, player).unwrap()
    };
    let accumulated = SearchEvaluation::LanePressureV3AccumulatedGold;
    let standard = SearchEvaluation::LanePressureV3;
    let before = score(&root, accumulated);
    let cash_before = score(&root, standard);
    let hero = root.world.hero_row_mut(player).unwrap();
    hero.level = 3;
    hero.gold -= 3;
    assert_eq!(score(&root, accumulated), before);
    assert!(score(&root, standard) < cash_before);

    root.world.hero_row_mut(enemy).unwrap().gold += 1;
    assert!(score(&root, accumulated) < before);
}

#[test]
fn live_resolution_widens_when_the_shallow_ranking_is_uncertain() {
    assert_eq!(
        live_root_width([100_000, 40_000, 10_000].into_iter(), 12),
        3
    );
    assert_eq!(live_root_width([100_000; 10].into_iter(), 12), 8);
    assert!(!live_roots_uncertain(
        Some((0, 90_000)),
        Some(20_000),
        0,
        35_000
    ));
    assert!(live_roots_uncertain(
        Some((0, 90_000)),
        Some(70_000),
        0,
        35_000
    ));
    assert!(live_roots_uncertain(
        Some((1, 90_000)),
        Some(20_000),
        0,
        35_000
    ));
}

#[test]
fn live_sampled_resolution_can_roll_four_turns_across_a_round() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut root =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let player = root.world.heroes[0].id;
    let mut plan = root.joint_plans(player, true, 1).unwrap().plans.remove(0);
    plan.extend(root.joint_plans(player, false, 1).unwrap().plans.remove(0));
    let budget = ExecutionBudget::default();
    root.apply_joint_planning_ordered(&plan).unwrap();
    root.advance_to_decision(&budget).unwrap();
    let player = root.search_player().unwrap().unwrap().0;
    let team = root.world.hero(player).unwrap().team;
    let sample = PlanningRolloutSample {
        seed: 39,
        upgrade_checkpoints: None,
    };
    let mut sampled = root
        .sampled_resolution_workers(player, sample, 1, &[player], &budget)
        .unwrap()
        .remove(0);
    let cutoff = sampled.search_turn_cutoff(Some(4)).unwrap().unwrap();
    let mut search = Search {
        player,
        team,
        budget: &budget,
        nodes: 0,
        max_nodes: 4_096,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::LanePressureV3,
        turn_cutoff: Some(cutoff),
        controlled: None,
        shared_nodes: None,
        guard_hidden: true,
        allow_simulated_upgrades: true,
        hand_belief: None,
        planning_sample: None,
        forecast_changes: 0,
        forecast_checks: 0,
        forecast_matches: 0,
        future_planning: None,
    };
    let leaf = search
        .rollout(&mut sampled, cutoff, false, false)
        .unwrap()
        .unwrap();
    assert_eq!(sampled.search_turn_index(), cutoff);
    assert!(leaf.round > root.round.round);

    let choices = root.resolution_choices(player).unwrap();
    assert!(choices.len() > 1);
    let incumbent = choices.last().unwrap();
    let mut worlds = root
        .sampled_resolution_workers(player, sample, 2, &[player], &budget)
        .unwrap();
    search.nodes = 0;
    let sum = worlds
        .iter_mut()
        .map(|world| {
            i128::from(
                search
                    .score_sampled_root(world, &mut None, incumbent, 0)
                    .unwrap()
                    .unwrap()
                    .score,
            )
        })
        .sum::<i128>();
    let mut ranking = Vec::new();
    let reference = root
        .search_live_bot_resolution(
            player,
            sample,
            2,
            &[player],
            None,
            &mut ranking,
            false,
            SearchLimits {
                max_depth: 1,
                max_nodes: 4_096,
                opponent_policy: search.opponent_policy,
                evaluation: search.evaluation,
            },
            2,
            &budget,
        )
        .unwrap();
    assert!(reference.score.is_some());
    assert!(ranking.len() > 1);
    assert!(ranking.windows(2).all(|pair| pair[0].1 >= pair[1].1));
    let limited = root
        .search_live_bot_resolution(
            player,
            sample,
            2,
            &[player],
            Some(incumbent),
            &mut ranking,
            false,
            SearchLimits {
                max_depth: 1,
                max_nodes: search.nodes,
                opponent_policy: search.opponent_policy,
                evaluation: search.evaluation,
            },
            4,
            &budget,
        )
        .unwrap();
    assert_eq!(limited.stop, ResolutionStop::NodeLimit);
    assert_eq!(limited.completed_depth, 0);
    assert_eq!(limited.decision, *incumbent);
    assert_eq!(limited.score, Some((sum / 2) as i64));
    assert_eq!(limited.score_kind, Some(ResolutionScoreKind::TurnRollout));
}

#[test]
fn short_turn_cutoff_rejects_a_rollout_stopped_at_hidden_upgrades() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut worker =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    worker.planning.phase = Phase::LevelUp;
    assert!(worker.last_request.is_none());
    let budget = ExecutionBudget::default();
    let cutoff = worker.search_turn_index() + 1;
    let mut search = Search {
        player: worker.world.heroes[0].id,
        team: worker.world.heroes[0].team,
        budget: &budget,
        nodes: 0,
        max_nodes: 16,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::MaterialPosition,
        turn_cutoff: Some(cutoff),
        controlled: None,
        shared_nodes: None,
        guard_hidden: true,
        allow_simulated_upgrades: false,
        hand_belief: None,
        planning_sample: None,
        forecast_changes: 0,
        forecast_checks: 0,
        forecast_matches: 0,
        future_planning: None,
    };
    let leaf = search
        .rollout(&mut worker, cutoff, false, false)
        .unwrap()
        .unwrap();
    assert_eq!(leaf.phase, Phase::LevelUp);
    assert_eq!(leaf.nodes, 0);
    assert!(!leaf.reached_cutoff(cutoff));
}

#[test]
fn unsupported_future_planning_sample_cannot_supply_a_rollout_score() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut worker =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    assert_eq!(worker.planning.phase, Phase::Planning);
    let player = worker.world.heroes[0].id;
    let team = worker.world.heroes[0].team;
    let opponent = worker
        .world
        .heroes
        .iter()
        .find(|hero| hero.team != team)
        .unwrap();
    let (opponent, card) = (opponent.id, opponent.hand[0]);
    // Simulate a publicly revealed opposing card at a future planning
    // boundary. The ordinary planning sampler cannot rebuild this root.
    worker.world.card_mut(card).unwrap().facedown = false;
    worker.world.hero_row_mut(opponent).unwrap().current = Some(card);
    let request = worker.last_request.clone();
    let cutoff = worker.search_turn_index() + 1;
    let budget = ExecutionBudget::default();
    let mut search = Search {
        player,
        team,
        budget: &budget,
        nodes: 0,
        max_nodes: 16,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::MaterialPosition,
        turn_cutoff: Some(cutoff),
        controlled: None,
        shared_nodes: None,
        guard_hidden: true,
        allow_simulated_upgrades: false,
        hand_belief: None,
        planning_sample: Some(PlanningRolloutSample {
            seed: 17,
            upgrade_checkpoints: None,
        }),
        forecast_changes: 0,
        forecast_checks: 0,
        forecast_matches: 0,
        future_planning: None,
    };

    assert!(matches!(
        search.rollout(&mut worker, cutoff, false, false),
        Err(EngineError::Unsupported(message)) if message.contains("publicly revealed current cards")
    ));
    assert_eq!(worker.planning.phase, Phase::Planning);
    assert_eq!(worker.last_request, request);
    assert_eq!(worker.world.hero(opponent).unwrap().current, Some(card));
}

#[test]
fn forced_root_move_at_turn_cutoff_reuses_warmup_score() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut root =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let player = root.world.heroes[0].id;
    let mut plan = root.joint_plans(player, true, 1).unwrap().plans.remove(0);
    plan.extend(root.joint_plans(player, false, 1).unwrap().plans.remove(0));
    root.apply_joint_planning_ordered(&plan).unwrap();
    let budget = ExecutionBudget::default();
    root.advance_to_decision(&budget).unwrap();
    let turn = root.search_turn_index();
    for _ in 0..64 {
        let (owner, team) = root.search_player().unwrap().unwrap();
        let moves = root.resolution_choices(owner).unwrap();
        let decision = moves
            .iter()
            .find(|decision| {
                matches!(decision, DecisionMove::Input { selection: crate::data::Datum::Text(id), .. } if root.symbols.name(*id) == "HOLD")
            })
            .unwrap_or(&moves[0]);
        let DecisionMove::Input {
            request: Some(token),
            selection,
            ..
        } = decision
        else {
            panic!("expected a live resolution input");
        };
        let mut branch = root.clone();
        branch.submit(*token, selection.clone()).unwrap();
        branch.last_request = None;
        branch.advance_to_decision(&budget).unwrap();
        if branch.search_turn_index() > turn {
            assert_eq!(moves.len(), 1);
            let limits = SearchLimits {
                max_depth: 1,
                max_nodes: 1,
                opponent_policy: OpponentPolicy::Adversarial,
                evaluation: SearchEvaluation::MaterialPosition,
            };
            let result = root
                .search_resolution_with_turn_rollout(owner, limits, Some(1), &budget)
                .unwrap();
            assert_eq!(result.score, Some(branch.evaluate_resolution(team, owner)));
            assert_eq!(
                (result.completed_depth, result.nodes, result.stop),
                (1, 1, ResolutionStop::Resolved)
            );
            let before = root.semantic_state().unwrap();
            let sampled = root
                .search_sampled_resolution_with_turn_rollout(
                    owner,
                    PlanningRolloutSample {
                        seed: 39,
                        upgrade_checkpoints: None,
                    },
                    limits,
                    Some(1),
                    None,
                    &budget,
                )
                .unwrap();
            assert_eq!(sampled.decision, result.decision);
            assert_eq!(sampled.score, result.score);
            assert_eq!((sampled.completed_depth, sampled.nodes), (1, 1));
            assert_eq!(root.semantic_state().unwrap(), before);
            return;
        }
        root = branch;
    }
    panic!("fixture did not reach a turn boundary");
}

#[test]
fn joint_plans_close_an_open_second_card_slot_and_restore_it_after_scoring() {
    let fixture = super::super::object_execution::fixture_case(
        "object-execution-planning-command-finish-dual-last",
    );
    let mut root =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let hero = root.world.heroes[0].id;
    let first = root.observed_planning_cards(hero).unwrap().0.unwrap();
    let before = root.semantic_state().unwrap();
    let rows = root.joint_plans(hero, true, 32).unwrap();
    assert!(rows.plans.iter().any(|row| row[0].first.is_none()));
    assert!(rows.plans.iter().any(|row| row[0].first.is_some()));
    for row in rows.plans {
        assert_eq!(row.len(), 1);
        assert_eq!(row[0].second, None);
        root.with_joint_planning(&row, |worker| {
            assert_ne!(worker.planning.phase, Phase::Planning);
            let revealed = worker.world.hero(hero).unwrap();
            assert_eq!(
                (revealed.current, revealed.extra),
                (Some(first), row[0].first)
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(root.semantic_state().unwrap(), before);
    }
    // First-legal continuation must cross this window too. With only one
    // node it can apply planning but cannot finish the subsequent turn.
    let budget = ExecutionBudget::default();
    assert!(
        root.rollout_game_turns(hero, 1, 1, &budget)
            .unwrap()
            .is_none()
    );
    assert_eq!(root.semantic_state().unwrap(), before);
}

#[test]
fn lane_pressure_timing_rewards_readiness_in_the_advanced_zone() {
    let fixture = super::super::object_execution::fixture_case(
        "object-execution-live-battle-multi-same-direction-first",
    );
    let mut root =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let red = root.symbols.lookup("RED").unwrap();
    let player = root
        .world
        .heroes
        .iter()
        .find(|hero| hero.team == red)
        .unwrap()
        .id;
    let graph = &root.world.definitions.zones;
    let (lane, ordered) = &graph.lanes[0];
    let zone = ordered
        .iter()
        .enumerate()
        .filter(|(index, _)| index.saturating_sub(1) > ordered.len().saturating_sub(2 + index))
        .filter_map(|(_, id)| graph.zones.iter().find(|zone| zone.id == *id))
        .find(|zone| zone.spawns.iter().any(|spawn| spawn.team != red))
        .unwrap();
    let (lane, other_lane) = (*lane, graph.lanes[1].0);
    root.world.battle_zones = vec![(lane, zone.id)];
    root.world.waves = vec![(lane, 2), (other_lane, 1)];
    let enemy_spawns: Vec<_> = zone
        .spawns
        .iter()
        .filter(|spawn| spawn.team != red)
        .map(|spawn| spawn.hex)
        .collect();
    let distance = |hex: Hex| {
        enemy_spawns
            .iter()
            .map(|&spawn| hex.distance(spawn))
            .min()
            .unwrap()
    };
    let positions = root.world.tile_hexes().iter().copied().filter(|&hex| {
        !root.world.terrain(hex) && root.world.occupant(hex).is_none_or(|id| id == player)
    });
    let near = positions.clone().min_by_key(|&hex| distance(hex)).unwrap();
    let far = positions.max_by_key(|&hex| distance(hex)).unwrap();
    assert!(distance(far) > distance(near).max(1));

    let timing_delta = |session: &NativeSession| {
        session.evaluate_lane_pressure(red, SearchEvaluation::LanePressureV4)
            - session.evaluate_lane_pressure(red, SearchEvaluation::LanePressureV3)
    };
    root.world.place_entity(player, far).unwrap();
    let unready = timing_delta(&root);
    root.world.move_entity(player, near).unwrap();
    assert!(timing_delta(&root) > unready);

    let ally = root
        .world
        .heroes
        .iter()
        .find(|hero| hero.team == red && hero.id != player)
        .unwrap()
        .id;
    let coverage_delta = |session: &NativeSession| {
        session.evaluate_lane_pressure(red, SearchEvaluation::LanePressureV5)
            - session.evaluate_lane_pressure(red, SearchEvaluation::LanePressureV4)
    };
    if root.world.position(ally).is_some() {
        root.world.move_entity(ally, far).unwrap();
    } else {
        root.world.place_entity(ally, far).unwrap();
    }
    let spread = coverage_delta(&root);
    let second_near = root
        .world
        .tile_hexes()
        .iter()
        .copied()
        .filter(|&hex| !root.world.terrain(hex) && root.world.occupant(hex).is_none())
        .min_by_key(|&hex| distance(hex))
        .unwrap();
    assert!(distance(second_near) < distance(far));
    root.world.move_entity(ally, second_near).unwrap();
    assert!(coverage_delta(&root) < spread);

    let ordinary = root.evaluate_lane_pressure(red, SearchEvaluation::LanePressureV4);
    root.world.waves[1].1 = 0;
    let last_push = root.evaluate_lane_pressure(red, SearchEvaluation::LanePressureV4);
    assert!(last_push > ordinary);

    root.world.combat.outcome = Some(crate::combat::GameOutcome {
        team: Some(red),
        hero: None,
        condition: red,
    });
    assert_eq!(
        root.search_score(SearchEvaluation::LanePressureV4, red, player)
            .unwrap(),
        WIN
    );
}

#[test]
fn multi_world_planning_prunes_rows_that_can_only_tie_a_win() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut root =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let hero = &root.world.heroes[0];
    let (player, team) = (hero.id, hero.team);
    root.world.combat.outcome = Some(crate::combat::GameOutcome {
        team: Some(team),
        hero: None,
        condition: team,
    });
    let rows = root.joint_plans(player, true, 2).unwrap().plans;
    let replies = root.joint_plans(player, false, 1).unwrap().plans;
    let mut samples = [
        PlanningWorld {
            session: root.clone(),
            replies: replies.clone(),
            forecasts: vec![],
            fixed_opponent_seed: None,
        },
        PlanningWorld {
            session: root,
            replies,
            forecasts: vec![],
            fixed_opponent_seed: None,
        },
    ];
    let result = search_planning_worlds(
        player,
        &rows,
        &mut samples,
        PlanningWorldOptions {
            limits: SearchLimits {
                max_depth: 0,
                max_nodes: 2,
                opponent_policy: OpponentPolicy::Adversarial,
                evaluation: SearchEvaluation::MaterialPosition,
            },
            workers: 1,
            partition: PlanningPartition {
                rollout_turns: None,
                controlled: None,
                shared_nodes: None,
                greedy_forecasts: &[],
                future_branching: None,
                legacy_fixed_followup: false,
                fixed_opponent_seed: None,
            },
            budget: &ExecutionBudget::default(),
            min_live_rows: None,
        },
    )
    .unwrap();
    assert_eq!(result.row, 0);
    assert_eq!(result.score, Some(WIN));
    assert_eq!((result.completed_rows, result.pruned_rows), (1, 1));
    assert_eq!((result.evaluated_cells, result.nodes), (2, 2));
}

#[test]
fn balanced_joint_plans_vary_the_ally_before_more_player_alternatives() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let root = NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
        .unwrap();
    let player = root.world.heroes[0].id;
    let team = root.world.heroes[0].team;
    let ally = root
        .world
        .heroes
        .iter()
        .find(|hero| hero.id != player && hero.team == team)
        .unwrap()
        .id;
    let controlled = [player, ally];
    let plans = root
        .joint_plans_with_reply_order(
            player,
            true,
            3,
            Some(&controlled),
            JointPlanOrder::Balanced,
            &[],
        )
        .unwrap()
        .plans;
    assert_eq!(plans.len(), 3);
    assert_eq!(plans[0][0].hero, player);
    assert_eq!(plans[0][1].hero, ally);
    assert_ne!(plans[1][0], plans[0][0]);
    assert_eq!(plans[1][1], plans[0][1]);
    assert_eq!(plans[2][0], plans[0][0]);
    assert_ne!(plans[2][1], plans[0][1]);
}

#[test]
fn scoped_planning_returns_only_bot_commitments_and_ignores_hidden_human_choices() {
    use super::super::TeamKnowledgeMode;
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut root =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let player = root.world.heroes[0].id;
    let team = root.world.heroes[0].team;
    let ally = root
        .world
        .heroes
        .iter()
        .find(|hero| hero.id != player && hero.team == team)
        .unwrap();
    let ally_id = ally.id;
    let hand = ally.hand.clone();
    let limits = SearchLimits {
        max_depth: 0,
        max_nodes: 16,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::MaterialPosition,
    };
    let options = PlanningSearchOptions {
        rollout_turns: None,
        workers: 1,
        upgrade_checkpoints: None,
        reply_cap: None,
        reply_probe_cap: None,
        greedy_card_order: false,
        tactical_forecast: false,
        forecast_node_share: ForecastNodeShare::default(),
        legacy_fixed_followup: false,
        fixed_future_planning: false,
        branch_future_planning: false,
    };
    let budget = ExecutionBudget::default();
    let run = |root: &NativeSession, bots: &[Symbol], mode| {
        root.search_bot_team_planning(
            player,
            BotPlanningScope {
                bots,
                mode,
                worlds: 2,
                seed: 39,
            },
            2,
            limits,
            options,
            &budget,
        )
        .unwrap()
    };
    let both = [player, ally_id];
    let coordinated = run(&root, &both, TeamKnowledgeMode::Coordinated);
    assert_eq!(
        coordinated
            .commitments
            .iter()
            .map(|row| row.hero)
            .collect::<Vec<_>>(),
        both
    );
    assert_eq!(
        run(&root, &both, TeamKnowledgeMode::TrueRules)
            .commitments
            .len(),
        1
    );
    let only = [player];
    let before = run(&root, &only, TeamKnowledgeMode::CoordinatedShow);
    assert_eq!(before.matrix.completed_rows, 2);
    assert_eq!(before.matrix.evaluated_cells, 8);
    assert_eq!(before.matrix.total_cells, 8);
    let parallel = root
        .search_bot_team_planning(
            player,
            BotPlanningScope {
                bots: &only,
                mode: TeamKnowledgeMode::CoordinatedShow,
                worlds: 2,
                seed: 39,
            },
            2,
            limits,
            PlanningSearchOptions {
                workers: 2,
                reply_probe_cap: Some(32),
                ..options
            },
            &budget,
        )
        .unwrap();
    assert_eq!(parallel, before);
    let state_before_retry = root.semantic_state().unwrap();
    let retry = root
        .search_bot_team_planning(
            player,
            BotPlanningScope {
                bots: &only,
                mode: TeamKnowledgeMode::TrueRules,
                worlds: 2,
                seed: 39,
            },
            2,
            SearchLimits {
                max_depth: 2,
                max_nodes: 64,
                ..limits
            },
            PlanningSearchOptions {
                rollout_turns: Some(2),
                reply_cap: Some(1),
                ..options
            },
            &budget,
        )
        .unwrap();
    assert_eq!(retry.matrix.completed_rows, 0);
    assert!(retry.matrix.score.is_none());
    assert!(retry.shallow_score.is_some());
    assert!((1..=4).contains(&retry.shallow_nodes));
    assert_eq!(retry.greedy_nodes + retry.probe_nodes, 0);
    assert!(retry.matrix.nodes > retry.shallow_nodes);
    assert!(retry.matrix.nodes <= 64);
    assert_eq!(retry.commitments[0].hero, player);
    assert_eq!(root.semantic_state().unwrap(), state_before_retry);
    let seed_pass = root
        .search_bot_team_planning_with_seed_pass(
            player,
            BotPlanningScope {
                bots: &only,
                mode: TeamKnowledgeMode::TrueRules,
                worlds: 2,
                seed: 39,
            },
            2,
            SearchLimits {
                max_depth: 2,
                max_nodes: 8192,
                ..limits
            },
            PlanningSearchOptions {
                rollout_turns: Some(2),
                reply_cap: Some(1),
                ..options
            },
            &budget,
            true,
        )
        .unwrap();
    assert!((1..=512).contains(&seed_pass.seed_nodes));
    assert!(seed_pass.seed_score.is_some());
    assert!(seed_pass.matrix.score.is_some());
    assert!(seed_pass.matrix.nodes <= 8192);
    assert_eq!(root.semantic_state().unwrap(), state_before_retry);
    let run_fixed = |root: &NativeSession, forecast_node_share| {
        root.search_bot_team_planning(
            player,
            BotPlanningScope {
                bots: &only,
                mode: TeamKnowledgeMode::CoordinatedShow,
                worlds: 2,
                seed: 39,
            },
            2,
            SearchLimits {
                max_nodes: 8192,
                ..limits
            },
            PlanningSearchOptions {
                workers: 2,
                greedy_card_order: true,
                rollout_turns: Some(2),
                forecast_node_share,
                ..options
            },
            &budget,
        )
        .unwrap()
    };
    let seeded = run_fixed(&root, ForecastNodeShare::Quarter);
    let unseeded = run_fixed(&root, ForecastNodeShare::Zero);
    assert_eq!(unseeded.greedy_nodes, 0);
    assert_eq!(unseeded.greedy_card_scores, 0);
    assert_eq!(unseeded.matrix.forecast_matches, 0);
    assert!(unseeded.matrix.forecast_checks > 0);
    assert_eq!(unseeded.matrix.total_cells, unseeded.total_rows * 2);
    assert!(unseeded.matrix.score.is_some());
    assert_eq!(root.semantic_state().unwrap(), state_before_retry);
    assert_eq!(seeded.prepared_worlds, 2);
    assert!(seeded.greedy_card_scores > 0);
    assert_eq!(before.greedy_card_scores, 0);
    assert_eq!(seeded.commitments[0].hero, player);
    assert!(seeded.matrix.score.is_some());
    assert!(seeded.greedy_nodes > 0);
    assert_eq!(seeded.shallow_nodes, 0);
    assert!(
        seeded.matrix.nodes
            >= seeded.greedy_nodes + seeded.probe_nodes + seeded.matrix.evaluated_cells
    );
    assert!(seeded.matrix.nodes <= 8192);

    let partial_parallel = root
        .search_bot_team_planning(
            player,
            BotPlanningScope {
                bots: &only,
                mode: TeamKnowledgeMode::CoordinatedShow,
                worlds: 2,
                seed: 39,
            },
            2,
            SearchLimits {
                max_nodes: 3,
                ..limits
            },
            PlanningSearchOptions {
                workers: 2,
                ..options
            },
            &budget,
        )
        .unwrap();
    assert_eq!(partial_parallel.matrix.completed_rows, 0);
    assert_eq!(partial_parallel.matrix.score, None);
    assert_eq!(partial_parallel.matrix.nodes, 3);

    let limited = root
        .search_bot_team_planning(
            player,
            BotPlanningScope {
                bots: &only,
                mode: TeamKnowledgeMode::CoordinatedShow,
                worlds: 2,
                seed: 39,
            },
            2,
            SearchLimits {
                max_nodes: 3,
                ..limits
            },
            options,
            &budget,
        )
        .unwrap();
    assert_eq!(limited.matrix.completed_rows, 0);
    assert_eq!(limited.matrix.score, None);
    assert_eq!(limited.matrix.nodes, 3);
    assert_eq!(limited.prepared_worlds, 2);
    let expired = ExecutionBudget {
        deadline: Some(std::time::Instant::now()),
        ..budget
    };
    let fallback = root
        .search_bot_team_planning(
            player,
            BotPlanningScope {
                bots: &only,
                mode: TeamKnowledgeMode::CoordinatedShow,
                worlds: 2,
                seed: 39,
            },
            2,
            limits,
            options,
            &expired,
        )
        .unwrap();
    assert_eq!(fallback.prepared_worlds, 0);
    assert_eq!(fallback.requested_worlds, 2);
    assert_eq!(fallback.matrix.score, None);
    assert_eq!(fallback.commitments, limited.commitments);
    root.commit_card(ally_id, hand[0]).unwrap();
    assert_eq!(run_fixed(&root, ForecastNodeShare::Quarter), seeded);
    assert_eq!(
        run(&root, &only, TeamKnowledgeMode::CoordinatedShow),
        before
    );
    let know = run(
        &root,
        &only,
        TeamKnowledgeMode::CoordinatedKnowSameTeamHumans,
    );
    assert_eq!(know.commitments.len(), 1);
    assert_eq!(know.commitments[0].hero, player);
}

#[test]
fn sampled_midround_planning_keeps_public_cards_in_their_live_slots() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let mut worker =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let player = worker.world.heroes[0].id;
    let team = worker.world.heroes[0].team;
    let budget = ExecutionBudget::default();
    let mut search = Search {
        player,
        team,
        budget: &budget,
        nodes: 0,
        max_nodes: 512,
        opponent_policy: OpponentPolicy::FirstLegal,
        evaluation: SearchEvaluation::MaterialPosition,
        turn_cutoff: None,
        controlled: None,
        shared_nodes: None,
        guard_hidden: false,
        allow_simulated_upgrades: false,
        hand_belief: None,
        planning_sample: None,
        future_planning: None,
        forecast_changes: 0,
        forecast_checks: 0,
        forecast_matches: 0,
    };
    worker
        .with_rollback(|worker| {
            let cutoff = worker.search_turn_index() + 1;
            assert!(search.rollout(worker, cutoff, false, false)?.is_some());
            assert_eq!(worker.planning.phase, Phase::Planning);
            let opponent = worker
                .world
                .heroes
                .iter()
                .find(|hero| hero.team != team && hero.played.iter().any(Option::is_some))
                .unwrap();
            let sampled = worker.sampled_planning_world(player, 39)?;
            let sampled_hero = sampled.world.hero(opponent.id).unwrap();
            assert_eq!(sampled_hero.played, opponent.played);
            assert_eq!(sampled_hero.discard, opponent.discard);
            for card in opponent.played.iter().flatten().chain(&opponent.discard) {
                assert_eq!(
                    sampled.world.card(*card).unwrap().lifecycle,
                    worker.world.card(*card).unwrap().lifecycle
                );
                assert!(!sampled_hero.hand.contains(card));
            }
            assert!(!sampled.planning_choices(opponent.id)?.is_empty());
            let expected = sampled.semantic_state()?;
            let before = worker.semantic_state()?;
            let actual =
                worker.with_sampled_planning(player, 39, |branch| branch.semantic_state())?;
            assert_eq!(actual, expected);
            assert_eq!(worker.semantic_state()?, before);
            Ok(())
        })
        .unwrap();
}

#[test]
fn sampled_planning_generates_bounded_joint_choices_and_returns_a_live_card() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let root = NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
        .unwrap();
    let player = root.world.heroes[0].id;
    let live = root.planning_choices(player).unwrap();
    let cap = 2;
    let seed = 39;
    let mut worker = root.sampled_planning_world(player, seed).unwrap();
    let friendly = worker.joint_plans(player, true, cap).unwrap();
    let opposing = worker.joint_plans(player, false, cap).unwrap();
    assert!(friendly.total > cap && opposing.total > cap);
    assert_eq!(friendly.plans.len(), cap);
    assert_eq!(opposing.plans.len(), cap);
    assert_eq!(friendly.plans[0][0].hero, player);
    assert_ne!(friendly.plans[0][0].first, friendly.plans[1][0].first);
    assert_eq!(friendly.plans[0][1..], friendly.plans[1][1..]);
    let limits = SearchLimits {
        max_depth: 0,
        max_nodes: 4,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::MaterialPosition,
    };
    let budget = ExecutionBudget::default();
    let expected = worker
        .search_planning_matrix(player, &friendly.plans, &opposing.plans, limits, &budget)
        .unwrap();
    let result = root
        .search_sampled_planning(player, seed, cap, limits, &budget)
        .unwrap();
    assert_eq!(result.matrix, expected);
    assert_eq!(result.matrix.evaluated_cells, 4);
    assert_eq!(result.total_friendly_plans, friendly.total);
    assert_eq!(result.total_opposing_plans, opposing.total);
    let card = result.commitment.first.unwrap();
    assert!(live.contains(&DecisionMove::Card { hero: player, card }));
}

#[test]
fn planning_matrix_matches_exhaustive_payoffs_and_excludes_incomplete_rows() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let root = NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
        .unwrap();
    let player = root.world.heroes[0].id;
    let team = root.world.heroes[0].team;
    let mut worker = root.sampled_planning_world(player, 71).unwrap();
    let side = |friendly| {
        (0..2)
            .map(|index| {
                worker
                    .world
                    .heroes
                    .iter()
                    .filter(|hero| (hero.team == team) == friendly)
                    .map(|hero| {
                        let moves = worker.planning_choices(hero.id).unwrap();
                        let DecisionMove::Card { card, .. } = moves[index] else {
                            panic!("expected ordinary card");
                        };
                        PlanningCommitment {
                            hero: hero.id,
                            first: Some(card),
                            second: None,
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let rows = side(true);
    let columns = side(false);
    let before = serde_json::to_value(worker.export_envelope().unwrap()).unwrap();
    let budget = ExecutionBudget::default();
    let mut payoffs = Vec::new();
    for row in &rows {
        let mut worst = i64::MAX;
        for column in &columns {
            let choices = [row.as_slice(), column.as_slice()].concat();
            let value = worker
                .with_joint_planning(&choices, |branch| {
                    branch.advance_to_decision(&budget)?;
                    Ok(branch.evaluate_resolution(team, player))
                })
                .unwrap();
            worst = worst.min(value);
        }
        payoffs.push(worst);
    }
    let limits = SearchLimits {
        max_depth: 0,
        max_nodes: 4,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::MaterialPosition,
    };
    let result = worker
        .search_planning_matrix(player, &rows, &columns, limits, &budget)
        .unwrap();
    assert_eq!(result.completed_rows, 2);
    assert_eq!(result.evaluated_cells, result.total_cells);
    assert_eq!(result.score, payoffs.iter().copied().max());
    assert_eq!(result.row, usize::from(payoffs[1] > payoffs[0]));
    let parallel = worker
        .search_planning_matrix_parallel(player, &rows, &columns, limits, 2, &budget)
        .unwrap();
    assert_eq!(parallel, result);
    let partial = worker
        .search_planning_matrix(
            player,
            &rows,
            &columns,
            SearchLimits {
                max_nodes: 3,
                ..limits
            },
            &budget,
        )
        .unwrap();
    assert_eq!(partial.completed_rows, 1);
    assert_eq!(partial.evaluated_cells, 3);
    assert_eq!(partial.score, Some(payoffs[0]));
    assert_eq!(partial.row, 0);
    let rollout_limits = SearchLimits {
        max_nodes: 512,
        ..limits
    };
    let mut ordered_columns = columns
        .iter()
        .map(|column| {
            let cell = worker
                .search_planning_matrix_with_rollout(
                    player,
                    &rows[..1],
                    std::slice::from_ref(column),
                    rollout_limits,
                    Some(2),
                    &budget,
                )
                .unwrap();
            (cell.score.unwrap(), column.clone())
        })
        .collect::<Vec<_>>();
    ordered_columns.sort_by_key(|(score, _)| *score);
    let expected = ordered_columns[0].0;
    let ordered_columns = ordered_columns
        .into_iter()
        .map(|(_, column)| column)
        .collect::<Vec<_>>();
    let tied_rows = vec![rows[0].clone(), rows[0].clone()];
    let pruned = worker
        .search_planning_matrix_with_rollout(
            player,
            &tied_rows,
            &ordered_columns,
            rollout_limits,
            Some(2),
            &budget,
        )
        .unwrap();
    assert_eq!(pruned.score, Some(expected));
    assert_eq!(pruned.row, 0);
    assert_eq!(
        (
            pruned.completed_rows,
            pruned.pruned_rows,
            pruned.evaluated_cells
        ),
        (1, 1, 3)
    );
    assert_eq!(
        serde_json::to_value(worker.export_envelope().unwrap()).unwrap(),
        before
    );
}

#[test]
fn planning_pair_replay_checks_round_rollover_and_detects_world_divergence() {
    let fixture =
        super::super::object_execution::fixture_case("object-execution-card-planning-cycle");
    let world =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    assert!(matches!(
        world
            .clone()
            .compare_planning_continuation(world.clone())
            .unwrap(),
        PlanningPair::NextRound,
    ));
    let mut changed = world.clone();
    changed.world.heroes[0].gold += 1;
    assert!(matches!(
        world
            .clone()
            .compare_planning_continuation(changed)
            .unwrap(),
        PlanningPair::Diverged,
    ));
}

#[test]
fn resolution_reuse_census_respects_caps_and_restores_the_prompt() {
    let fixture = super::super::object_execution::fixture_case(
        "object-execution-attack-macro-optional-first",
    );
    let mut worker =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    worker
        .advance_to_decision(&ExecutionBudget::default())
        .unwrap();
    let before = worker.semantic_state().unwrap();
    let request = worker.last_request.clone();
    let census = worker.resolution_reuse_census(4, 2).unwrap();
    assert_eq!(census.visited, 2);
    assert!(census.internal > 0);
    assert_eq!(worker.semantic_state().unwrap(), before);
    assert_eq!(worker.last_request, request);
}

#[test]
fn attack_target_extension_searches_beyond_depth_zero_within_the_node_cap() {
    let fixture = super::super::object_execution::fixture_case(
        "object-execution-attack-macro-optional-first",
    );
    let mut worker =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    worker.advance_prefix(1).unwrap();
    let owner = worker.advance_prefix(1).unwrap().request.unwrap().owner;
    let team = worker.world.hero(owner).unwrap().team;
    assert!(worker.pending_attack_target());
    let choices = worker.resolution_choices(owner).unwrap().len();
    assert!(choices > 1);
    let before = worker.semantic_state().unwrap();
    let budget = ExecutionBudget::default();
    let mut search = Search {
        player: owner,
        team,
        budget: &budget,
        nodes: 0,
        max_nodes: 100,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::MaterialPosition,
        turn_cutoff: None,
        controlled: None,
        shared_nodes: None,
        guard_hidden: false,
        allow_simulated_upgrades: false,
        hand_belief: None,
        planning_sample: None,
        future_planning: None,
        forecast_changes: 0,
        forecast_checks: 0,
        forecast_matches: 0,
    };
    worker
        .with_rollback(|branch| search.value(branch, 0, i64::MIN, i64::MAX))
        .unwrap()
        .unwrap();
    assert_eq!(search.nodes, 0);

    search.opponent_policy = OpponentPolicy::AdversarialAttackTargets;
    worker
        .with_rollback(|branch| search.value(branch, 0, i64::MIN, i64::MAX))
        .unwrap()
        .unwrap();
    assert!(search.nodes >= choices);

    search.nodes = 0;
    search.max_nodes = 1;
    assert!(
        worker
            .with_rollback(|branch| search.value(branch, 0, i64::MIN, i64::MAX))
            .unwrap()
            .is_none()
    );
    assert_eq!(search.nodes, 1);
    assert_eq!(worker.semantic_state().unwrap(), before);
}

#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
)]
fn scoped_search_prefers_an_eligible_bot_for_a_team_prompt() {
    let fixture = super::super::object_execution::fixture_case(
        "object-execution-hero-respawn-live-ordinary-first",
    );
    let mut worker =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let budget = ExecutionBudget::default();
    let owner = worker.advance(&budget).unwrap().request.unwrap().owner;
    let team = worker.world.hero(owner).unwrap().team;
    let mut allies = worker.world.heroes.iter().filter(|hero| hero.team == team);
    let first = allies.next().unwrap().id;
    let bot = allies.next().unwrap().id;
    let enemy = worker
        .world
        .heroes
        .iter()
        .find(|hero| hero.team != team)
        .unwrap()
        .id;

    assert_eq!(
        worker
            .search_player_preferring(Some(&[enemy, bot]))
            .unwrap(),
        Some((owner, team)),
    );
    worker.last_request.as_mut().unwrap().choices = RequestChoices::Actors {
        team,
        values: vec![first, bot],
    };
    assert_eq!(worker.search_player().unwrap(), Some((first, team)));
    assert_eq!(
        worker
            .search_player_preferring(Some(&[enemy, bot]))
            .unwrap(),
        Some((bot, team)),
    );
    assert_eq!(
        worker.search_player_preferring(Some(&[enemy])).unwrap(),
        Some((first, team)),
    );
}

#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
)]
fn fixed_policy_expands_only_the_first_opposing_move() {
    let fixture = super::super::object_execution::fixture_case(
        "object-execution-hero-respawn-live-ordinary-first",
    );
    let mut worker =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let budget = ExecutionBudget::default();
    let owner = worker.advance(&budget).unwrap().request.unwrap().owner;
    let owner_team = worker.world.hero(owner).unwrap().team;
    let root = worker
        .world
        .heroes
        .iter()
        .find(|hero| hero.team != owner_team)
        .unwrap();
    let (player, team) = (root.id, root.team);
    let moves = worker.resolution_choices(owner).unwrap();
    assert!(moves.len() > 1);
    let mut search = Search {
        player,
        team,
        budget: &budget,
        nodes: 0,
        max_nodes: 100,
        opponent_policy: OpponentPolicy::FirstLegal,
        evaluation: SearchEvaluation::MaterialPosition,
        turn_cutoff: None,
        controlled: None,
        shared_nodes: None,
        guard_hidden: false,
        allow_simulated_upgrades: false,
        hand_belief: None,
        planning_sample: None,
        future_planning: None,
        forecast_changes: 0,
        forecast_checks: 0,
        forecast_matches: 0,
    };
    let before = worker.semantic_state().unwrap();
    let fixed = worker
        .with_rollback(|branch| search.value(branch, 1, i64::MIN, i64::MAX))
        .unwrap()
        .unwrap();
    assert_eq!(search.nodes, 1);
    assert_eq!(worker.semantic_state().unwrap(), before);
    let first = search
        .branch(&mut worker, moves[0].clone(), 0, i64::MIN, i64::MAX)
        .unwrap()
        .unwrap();
    assert_eq!(fixed.score, first.score);
    search.nodes = 0;
    search.opponent_policy = OpponentPolicy::Adversarial;
    let adversarial = worker
        .with_rollback(|branch| search.value(branch, 1, i64::MIN, i64::MAX))
        .unwrap()
        .unwrap();
    assert_eq!(search.nodes, moves.len());
    assert!(adversarial.score <= fixed.score);
}

#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
)]
fn preferred_root_move_is_searched_first_without_changing_complete_result() {
    let fixture = super::super::object_execution::fixture_case(
        "object-execution-hero-respawn-live-ordinary-first",
    );
    let mut worker =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let budget = ExecutionBudget::default();
    let request = worker.advance(&budget).unwrap().request.unwrap();
    let player = request.owner;
    let team = worker.world.hero(player).unwrap().team;
    let moves = worker.resolution_choices(player).unwrap();
    assert!(moves.len() > 1);
    let preferred = moves.len() - 1;
    let mut search = Search {
        player,
        team,
        budget: &budget,
        nodes: 0,
        max_nodes: 1,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::MaterialPosition,
        turn_cutoff: None,
        controlled: None,
        shared_nodes: None,
        guard_hidden: false,
        allow_simulated_upgrades: false,
        hand_belief: None,
        planning_sample: None,
        future_planning: None,
        forecast_changes: 0,
        forecast_checks: 0,
        forecast_matches: 0,
    };
    let limited = search
        .choose(
            &mut worker,
            &moves,
            1,
            preferred,
            &mut vec![None; moves.len()],
        )
        .unwrap();
    assert_eq!(limited.interrupted, Some(ResolutionStop::NodeLimit));
    assert_eq!(limited.best.unwrap().0, preferred);
    search.max_nodes = 1000;
    let ordered = search
        .choose(
            &mut worker,
            &moves,
            2,
            preferred,
            &mut vec![None; moves.len()],
        )
        .unwrap();
    let baseline = search
        .choose(&mut worker, &moves, 2, 0, &mut vec![None; moves.len()])
        .unwrap();
    assert!(ordered.interrupted.is_none() && baseline.interrupted.is_none());
    assert_eq!(ordered.best, baseline.best);
}
