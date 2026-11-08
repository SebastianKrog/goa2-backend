import json
import subprocess
from contextlib import closing
from threading import Barrier, Event

import pytest

import arena_queue
from arena_reports import load_report, save_start, save_verification
from oracle import canonical, digest


class WorkerCrash(BaseException):
    pass


@pytest.fixture
def job(tmp_path, monkeypatch):
    start = save_start(tmp_path, {"state": {}})
    profile = {"starts": [{"start_sha256": start, "seed": 12}], "parameters": {},
               "scorer": "arena_search_v1"}
    manifest = {
        "candidate": "candidate-commit", "standard": "standard-commit", "referee": "rules",
        "repo": str(tmp_path), "profile": profile, "profile_sha256": digest(profile),
    }
    with closing(arena_queue.open_queue(tmp_path)) as db, db:
        db.execute("INSERT INTO jobs VALUES (?, ?, ?, NULL, ?)",
                   ("job", canonical(manifest), "queued", 1))
        db.executemany("INSERT INTO matches (job, pair, side) VALUES (?, ?, ?)",
                       (("job", 0, side) for side in arena_queue.SIDES))

    monkeypatch.setattr(arena_queue, "build_policy", lambda _, __, commit, ___: commit)
    monkeypatch.setattr(arena_queue, "preflight", lambda *_: None)

    def verified(db, store, job, _referee):
        with db:
            rows = db.execute("SELECT report FROM matches WHERE job = ? AND report IS NOT NULL", (job,))
            for row in rows:
                identity = row["report"]
                save_verification(store, identity, {"report_id": identity, "status": "pass"})
            db.execute("UPDATE matches SET verdict = 'pass' WHERE job = ? AND report IS NOT NULL", (job,))

    monkeypatch.setattr(arena_queue, "verify_pending", verified)
    return tmp_path


def result():
    return {"endpoint": {"phase": "GAME_OVER", "winner_team": "RED", "winner_hero": None},
            "trace": [{"choice": "SKIP"}],
            "work": {side: {"forced": 1, "planning_requests": 1, "resolution_requests": 1}
                     for side in arena_queue.SIDES}}


def run(store, match_workers=1):
    with closing(arena_queue.open_queue(store)) as db:
        arena_queue.run_queue(db, store, False, match_workers)
        return arena_queue.describe_jobs(db, store, "job")[0]


def test_resume_does_not_rescore_a_game_with_a_durable_completion(job, monkeypatch):
    seen = []
    publish = arena_queue.publish_receipt

    def score(_paths, _envelope, *, expected_binaries, **_parameters):
        seen.append(expected_binaries["RED"])
        return result()

    def crash(*_):
        raise WorkerCrash

    monkeypatch.setattr(arena_queue, "score_match", score)
    monkeypatch.setattr(arena_queue, "publish_receipt", crash)
    with pytest.raises(WorkerCrash):
        run(job)

    monkeypatch.setattr(arena_queue, "publish_receipt", publish)
    resumed = run(job)
    assert resumed["status"] == "verified"
    assert seen == ["candidate-commit", "standard-commit"]
    assert resumed["result"]["paired_advantage"] == 0
    assert resumed["result"]["outcomes"] == {"win": 1, "loss": 1, "verified": 2}
    assert len(resumed["attempts"]) == 2
    for row in resumed["matches"]:
        report, _ = load_report(job, row["report"])
        assert report["queue"]["candidate_side"] == row["side"]


def test_resume_retains_partial_attempt_without_scoring_it(job, monkeypatch):
    def crash(*_, record, **_parameters):
        record({"choice": "SKIP"})
        raise WorkerCrash

    monkeypatch.setattr(arena_queue, "score_match", crash)
    with pytest.raises(WorkerCrash):
        run(job)
    monkeypatch.setattr(arena_queue, "score_match", lambda *_, **__: result())

    resumed = run(job)
    assert resumed["status"] == "verified"
    assert len(resumed["matches"]) == 2
    old = resumed["attempts"][0]
    assert old["status"] == "interrupted"
    assert old["report"] is None
    assert json.loads((job / "attempts" / old["id"] / "trace.jsonl").read_text()) == {"choice": "SKIP"}
    assert all(row["status"] == "complete" for row in resumed["attempts"][1:])


def test_retry_preserves_finished_side(job, monkeypatch):
    seen = []

    def score(_paths, _envelope, *, expected_binaries, **_parameters):
        seen.append(expected_binaries["RED"])
        if len(seen) == 2:
            raise TimeoutError("watchdog")
        return result()

    monkeypatch.setattr(arena_queue, "score_match", score)
    failed = run(job)
    assert failed["status"] == "failed"
    assert failed["matches"][0]["report"] is not None
    assert failed["matches"][0]["verdict"] == "pass"
    assert failed["matches"][1]["report"] is None
    with closing(arena_queue.open_queue(job)) as db:
        arena_queue.cancel_or_retry(db, "job", retry=True)
    resumed = run(job)
    assert resumed["status"] == "verified"
    assert seen == ["candidate-commit", "standard-commit", "standard-commit"]
    assert resumed["matches"][0]["report"] == failed["matches"][0]["report"]


def test_parallel_pair_retains_the_finished_side_when_its_peer_fails(job, monkeypatch):
    started = Barrier(2, timeout=10)

    def score(_paths, _envelope, *, expected_binaries, record, **_parameters):
        record({"choice": "SKIP"})
        started.wait()
        if expected_binaries["RED"] == "candidate-commit":
            raise TimeoutError("watchdog")
        return result()

    monkeypatch.setattr(arena_queue, "score_match", score)
    failed = run(job, match_workers=2)
    assert failed["status"] == "failed"
    matches = {match["side"]: match for match in failed["matches"]}
    assert matches["RED"]["report"] is None
    completed = matches["BLUE"]["report"]
    assert completed and matches["BLUE"]["verdict"] == "pass"
    with closing(arena_queue.open_queue(job)) as db:
        arena_queue.cancel_or_retry(db, "job", retry=True)

    def retry(_paths, _envelope, *, expected_binaries, **_parameters):
        assert expected_binaries["RED"] == "candidate-commit"
        return result()

    monkeypatch.setattr(arena_queue, "score_match", retry)
    resumed = run(job, match_workers=2)
    assert resumed["status"] == "verified"
    assert {match["side"]: match for match in resumed["matches"]}["BLUE"]["report"] == completed
    assert len(resumed["attempts"]) == 3


def test_parallel_keyboard_interrupt_stops_both_matches_at_a_decision(job, monkeypatch):
    started = Barrier(3, timeout=10)
    interrupted = Event()

    def score(*_, record, **_parameters):
        record({"choice": "SKIP"})
        started.wait()
        assert interrupted.wait(10)
        record({"choice": "SKIP"})
        pytest.fail("scoring continued past the interruption")

    def interrupt(_futures):
        started.wait()
        raise KeyboardInterrupt

    with monkeypatch.context() as patch:
        patch.setattr(arena_queue, "score_match", score)
        patch.setattr(arena_queue, "Event", lambda: interrupted)
        patch.setattr(arena_queue, "wait", interrupt)
        with pytest.raises(KeyboardInterrupt):
            run(job, match_workers=2)

    monkeypatch.setattr(arena_queue, "score_match", lambda *_, **__: result())
    resumed = run(job, match_workers=2)
    assert resumed["status"] == "verified"
    interrupted_attempts = resumed["attempts"][:2]
    assert all(attempt["reason"] == "KeyboardInterrupt: " for attempt in interrupted_attempts)
    assert all(attempt["report"] is None for attempt in interrupted_attempts)
    for attempt in interrupted_attempts:
        assert len((job / "attempts" / attempt["id"] / "trace.jsonl").read_text().splitlines()) == 2
    assert all(attempt["status"] == "complete" for attempt in resumed["attempts"][2:])


def test_enqueue_retains_commits_and_normalizes_start_paths(tmp_path):
    repo = tmp_path / "repo"
    repo.mkdir()

    def git(*args):
        return subprocess.check_output(["git", "-C", repo, *args], text=True).strip()

    git("init")
    git("-c", "user.name=Arena Test", "-c", "user.email=arena@example.test",
        "commit", "--quiet", "--allow-empty", "-m", "standard")
    git("branch", "candidate")
    first = git("rev-parse", "HEAD")
    store = tmp_path / "store"
    referee = tmp_path / "referee"
    referee.write_bytes(b"pinned rules worker")
    start = tmp_path / "start.json"
    start.write_text(canonical({"state": {}}))
    profile = tmp_path / "profile.json"
    profile.write_text(canonical({
        "schema_version": 1, "name": "smoke",
        "starts": [{"envelope": "start.json", "seed": 1}],
        "parameters": {"turns": 1, "planning_nodes": 12, "resolution_nodes": 5,
                       "coordinated": False},
    }))

    with closing(arena_queue.open_queue(store)) as db:
        baseline_policy = tmp_path / "baseline-policy.json"
        baseline_policy.write_text(canonical({"planning": {"forecast_node_share": "eighth"}}))
        candidate_policy = tmp_path / "candidate-policy.json"
        candidate_policy.write_text(canonical({"resolution": {"rollout_turns": 3}}))
        arena_queue.init_standard(db, store, repo, "HEAD", referee, baseline_policy)
        identity = arena_queue.enqueue(db, store, repo, "candidate", profile, candidate_policy=candidate_policy)
        candidate_policy.write_text("{}")
        inherited = arena_queue.enqueue(db, store, repo, "candidate", profile)
        assert arena_queue.describe_jobs(db, store, inherited)[0]["manifest"]["candidate_policy"] == {
            "planning": {"forecast_node_share": "eighth"},
        }
        git("-c", "user.name=Arena Test", "-c", "user.email=arena@example.test",
            "commit", "--quiet", "--allow-empty", "-m", "new commit")
        git("branch", "-f", "candidate", "HEAD")
        start.write_text(canonical({"state": {"new": True}}))
        manifest = arena_queue.describe_jobs(db, store, identity)[0]["manifest"]

    assert manifest["candidate"] == manifest["standard"] == first
    assert manifest["standard_policy"] == {"planning": {"forecast_node_share": "eighth"}}
    assert manifest["candidate_policy"] == {"resolution": {"rollout_turns": 3}}
    assert manifest["profile"]["starts"][0]["start_sha256"] == digest({"state": {}})
    assert git("rev-parse", f"refs/goa2-arena/pinned/{first}") == first

    with closing(arena_queue.open_queue(store)) as db:
        with pytest.raises(ValueError, match="accepted standard changed"):
            arena_queue.set_standard(db, repo, "HEAD", "HEAD", None, "Use combined control")
        new_standard = arena_queue.set_standard(
            db, repo, "HEAD", first, baseline_policy, "Use combined control",
        )
        assert new_standard["commit"] == git("rev-parse", "HEAD")
        assert new_standard["policy"] == {"planning": {"forecast_node_share": "eighth"}}
        previous, updated, reason = db.execute("SELECT previous, next, reason FROM standard_changes").fetchone()
        assert json.loads(previous)["commit"] == first
        assert json.loads(updated) == new_standard
        assert reason == "Use combined control"
        assert arena_queue.describe_jobs(db, store, identity)[0]["manifest"] == manifest


def stage_profile(store, *, track="stronger", confirm=12, advance_at=0.25, method=None, triage=False):
    starts = [{"start_sha256": save_start(store, {"state": {"fixture": index}}), "seed": index}
              for index in range(4 + confirm)]
    starts[1] = starts[0]
    confidence = {} if method is None else {"method": method}
    if triage:
        confidence["screening"] = "triage_v2"
    return {"starts": starts, "parameters": {"turns": 64},
            "scorer": "arena_search_v2" if triage else "arena_search_v1",
            "acceptance": {"preflight_pairs": 1, "screen_pairs": 1, "extend_batches": [1],
                           "confirm_pairs": confirm, "advance_at": advance_at, "stop_below": 0 if triage else -0.5,
                           "max_unfinished_rate": 0, "max_fallback_rate": 0,
                           "track": track, "effect": 0.3 if track != "stronger" else 0,
                           "alpha": 0.05, **confidence}}


def configure_stages(store, **options):
    profile = stage_profile(store, **options)
    with closing(arena_queue.open_queue(store)) as db, db:
        manifest = json.loads(db.execute("SELECT manifest FROM jobs WHERE id = 'job'").fetchone()[0])
        identity = digest(profile)
        db.execute("INSERT INTO registered_profiles VALUES (?, ?)", (identity, canonical(profile)))
        manifest.update(profile=profile, profile_sha256=identity, registered_profile=identity)
        db.execute("UPDATE jobs SET manifest = ? WHERE id = 'job'", (canonical(manifest),))
        db.execute("INSERT INTO settings VALUES ('standard', ?)",
                   (canonical({"commit": manifest["standard"], "referee": manifest["referee"]}),))
        db.executemany("INSERT INTO matches (job, pair, side) VALUES (?, ?, ?)",
                       (("job", pair, side) for pair in range(1, len(profile["starts"]))
                        for side in arena_queue.SIDES))


@pytest.mark.parametrize("track", ["stronger", "non_inferior"])
def test_stages_advance_and_only_explicit_review_promotes(job, monkeypatch, track):
    configure_stages(job, track=track)
    calls = []

    def score(_paths, _envelope, *, seed, turns, expected_binaries, **_):
        calls.append((seed, turns))
        game = result()
        game["endpoint"]["winner_team"] = next(
            side for side in arena_queue.SIDES if expected_binaries[side] == "candidate-commit"
        )
        return game

    monkeypatch.setattr(arena_queue, "score_match", score)
    finished = run(job)
    assert [stage["stage"] for stage in finished["stages"]] == ["preflight", "screen", "extend", "confirm"]
    assert calls[:4] == [(0, 1)] * 4
    assert all(turns == 64 for _, turns in calls[4:])
    assert finished["status"] == "passed"
    assert finished["stages"][-1]["analysis"]["pairs"] == 12
    with closing(arena_queue.open_queue(job)) as db:
        previous = db.execute("SELECT value FROM settings WHERE name = 'standard'").fetchone()[0]
        manifest = finished["manifest"]
        with db:
            db.execute("UPDATE jobs SET manifest = ? WHERE id = 'job'",
                       (canonical(dict(manifest, registered_profile=None)),))
        with pytest.raises(ValueError, match="registered before submission"):
            arena_queue.promote(db, job, "job", None)
        with db:
            db.execute("UPDATE jobs SET manifest = ? WHERE id = 'job'", (canonical(manifest),))
        with db:
            db.execute("UPDATE settings SET value = ? WHERE name = 'standard'",
                       (canonical({"commit": "new-standard", "referee": "rules"}),))
        with pytest.raises(ValueError, match="standard or referee changed"):
            arena_queue.promote(db, job, "job", None)
        changed = dict(json.loads(previous), policy={"planning": {"rollout_turns": 3}})
        with db:
            db.execute("UPDATE settings SET value = ? WHERE name = 'standard'", (canonical(changed),))
        with pytest.raises(ValueError, match="standard or referee changed"):
            arena_queue.promote(db, job, "job", None)
        candidate_policy = {"resolution": {"evaluation": "lane_pressure_v4"}}
        with db:
            db.execute("UPDATE settings SET value = ? WHERE name = 'standard'", (previous,))
            db.execute("UPDATE jobs SET manifest = ? WHERE id = 'job'",
                       (canonical(dict(manifest, candidate_policy=candidate_policy)),))
        evidence = None
        if track != "stronger":
            with pytest.raises(ValueError, match="separate reviewed benefit evidence"):
                arena_queue.promote(db, job, "job", None)
            evidence = job / "reviewed-benefit.txt"
            evidence.write_text("Measured throughput on a controlled workload.")
        accepted = arena_queue.promote(db, job, "job", evidence)
        assert accepted["commit"] == "candidate-commit"
        assert accepted["policy"] == candidate_policy
        if evidence is not None:
            identity = db.execute("SELECT evidence FROM promotions WHERE job = 'job'").fetchone()[0]
            assert (job / "benefits" / identity).read_bytes() == evidence.read_bytes()


def test_tied_tuning_stays_inconclusive_and_leaves_confirmation_unplayed(job, monkeypatch):
    configure_stages(job, track="non_inferior")
    monkeypatch.setattr(arena_queue, "score_match", lambda *_, **__: result())
    finished = run(job)
    assert finished["status"] == "inconclusive"
    assert [stage["outcome"] for stage in finished["stages"]] == ["advance", "extend_low", "inconclusive"]
    assert len(finished["attempts"]) == 8
    assert all(match["report"] is None for match in finished["matches"] if match["pair"] >= 4)


def test_v2_tie_stops_until_evidence_and_then_waits_for_a_shortlist(job, monkeypatch):
    configure_stages(job, triage=True)

    def score(*_, seed, expected_binaries, **__):
        game = result()
        if seed != 2:
            game["endpoint"]["winner_team"] = next(
                side for side in arena_queue.SIDES if expected_binaries[side] == "candidate-commit"
            )
        return game

    monkeypatch.setattr(arena_queue, "score_match", score)
    screened = run(job)
    assert screened["status"] == "screened_out"
    assert len(screened["attempts"]) == 6
    with closing(arena_queue.open_queue(job)) as db:
        with pytest.raises(ValueError, match="requires evidence"):
            arena_queue.extend_tie(db, "job", " ")
        arena_queue.extend_tie(db, "job", "Verified replay shows an earlier lane push.")
    waiting = run(job)
    assert waiting["status"] == "awaiting_confirmation"
    assert waiting["stages"][1]["extension_evidence"] == "Verified replay shows an earlier lane push."
    assert len(waiting["attempts"]) == 8
    assert all(row["report"] is None for row in waiting["matches"] if row["pair"] >= 4)
    with closing(arena_queue.open_queue(job)) as db:
        arena_queue.select_confirmation(db, ["job"])
    assert run(job)["status"] == "passed"


def test_v2_weak_screen_is_distinct_from_a_health_failure(job, monkeypatch):
    configure_stages(job, triage=True)

    def score(*_, expected_binaries, **__):
        game = result()
        game["endpoint"]["winner_team"] = next(
            side for side in arena_queue.SIDES if expected_binaries[side] == "standard-commit"
        )
        return game

    monkeypatch.setattr(arena_queue, "score_match", score)
    screened = run(job)
    assert screened["status"] == "screened_out"
    assert screened["stages"][-1]["paired_advantage"] == -1
    with closing(arena_queue.open_queue(job)) as db:
        with pytest.raises(ValueError, match="only a tied screening result"):
            arena_queue.extend_tie(db, "job", "Unrelated evidence")


def test_v2_reports_when_both_workers_exceed_fallback_guardrail(job, monkeypatch):
    configure_stages(job, triage=True)

    def score(*_, seed, **__):
        game = result()
        if seed == 2:
            for work in game["work"].values():
                work["resolution_unsupported"] = 1
        return game

    monkeypatch.setattr(arena_queue, "score_match", score)
    screened = run(job)
    assert screened["status"] == "unhealthy"
    assert screened["reason"] == "candidate and baseline both exceeded the fallback guardrail"
    assert len(screened["attempts"]) == 6


def test_v2_shortlist_has_only_two_slots_per_registered_profile(job):
    configure_stages(job, triage=True)
    with closing(arena_queue.open_queue(job)) as db:
        with db:
            manifest = db.execute("SELECT manifest FROM jobs WHERE id = 'job'").fetchone()[0]
            db.execute("UPDATE jobs SET status = 'awaiting_confirmation' WHERE id = 'job'")
            db.executemany("INSERT INTO jobs VALUES (?, ?, 'awaiting_confirmation', NULL, 2)",
                           ((name, manifest) for name in ("second", "third")))
        arena_queue.select_confirmation(db, ["job", "second"])
        with pytest.raises(ValueError, match="two confirmation slots"):
            arena_queue.select_confirmation(db, ["third"])
        assert arena_queue.job_status(db, "third") == "awaiting_confirmation"


def test_preflight_stops_when_repeated_result_changes(job, monkeypatch):
    configure_stages(job)
    count = 0

    def score(*_, **__):
        nonlocal count
        count += 1
        game = result()
        game["work"]["RED"]["forced"] = count
        return game

    monkeypatch.setattr(arena_queue, "score_match", score)
    finished = run(job)
    assert finished["status"] == "rejected"
    assert finished["stages"][0]["reason"] == "repeated node-budget preflight differed"
    assert len(finished["attempts"]) == 4


def test_tied_held_out_stage_does_not_promote(job, monkeypatch):
    configure_stages(job, track="non_inferior", advance_at=0)
    monkeypatch.setattr(arena_queue, "score_match", lambda *_, **__: result())
    finished = run(job)
    assert finished["status"] == "inconclusive"
    assert finished["stages"][-1]["stage"] == "confirm"
    assert finished["stages"][-1]["analysis"]["estimate"] == 0
    with closing(arena_queue.open_queue(job)) as db:
        with pytest.raises(ValueError, match="passing held-out result"):
            arena_queue.promote(db, job, "job", None)


def test_scheduler_reserves_a_dispatch_for_screening(job):
    with closing(arena_queue.open_queue(job)) as db, db:
        manifest = db.execute("SELECT manifest FROM jobs WHERE id = 'job'").fetchone()[0]
        for identity in ("promising", "uncertain"):
            db.execute("INSERT INTO jobs VALUES (?, ?, 'queued', NULL, 2)", (identity, manifest))
            db.execute("INSERT INTO stage_runs VALUES (?, 0, 'advance', '{}')", (identity,))
        db.execute("INSERT INTO stage_runs VALUES ('promising', 1, 'advance', '{}')")
        db.execute("INSERT INTO stage_runs VALUES ('uncertain', 1, 'extend_low', '{}')")
        assert arena_queue.next_job(db)["id"] == "promising"
        db.execute("INSERT INTO settings VALUES ('dispatches', '3')")
        assert arena_queue.next_job(db)["id"] == "job"
        db.execute("UPDATE jobs SET status = 'cancelled' WHERE id = 'job'")
        assert arena_queue.next_job(db)["id"] == "promising"


def test_pinned_policy_settings_follow_candidate_and_standard_when_sides_swap(job, monkeypatch):
    candidate = {"planning": {"rollout_turns": 3}}
    standard = {"planning": {"forecast_node_share": "eighth"}}
    with closing(arena_queue.open_queue(job)) as db, db:
        manifest = json.loads(db.execute("SELECT manifest FROM jobs WHERE id = 'job'").fetchone()[0])
        manifest.update(candidate_policy=candidate, standard_policy=standard)
        db.execute("UPDATE jobs SET manifest = ? WHERE id = 'job'", (canonical(manifest),))

    def score(*_, policy_profiles, **__):
        return dict(result(), parameters={"policy_profiles": policy_profiles})

    monkeypatch.setattr(arena_queue, "score_match", score)
    completed = run(job)
    for row in completed["matches"]:
        report, _ = load_report(job, row["report"])
        policies = report["parameters"]["policy_profiles"]
        assert policies[row["side"]] == candidate
        assert [policy for side, policy in policies.items() if side != row["side"]] == [standard]


def test_import_preflight_checks_the_whole_stage_before_scoring(job, monkeypatch):
    rejected = save_start(job, {"state": {"rng_seed": 2**64 - 1}})
    with closing(arena_queue.open_queue(job)) as db, db:
        manifest = json.loads(db.execute("SELECT manifest FROM jobs WHERE id = 'job'").fetchone()[0])
        manifest["profile"]["starts"].append({"start_sha256": rejected, "seed": 12})
        manifest["profile_sha256"] = digest(manifest["profile"])
        db.execute("UPDATE jobs SET manifest = ? WHERE id = 'job'", (canonical(manifest),))
        db.executemany("INSERT INTO matches (job, pair, side) VALUES (?, ?, ?)",
                       (("job", 1, side) for side in arena_queue.SIDES))

    def check(_store, _manifest, _binaries, starts):
        assert len(starts) == 2 and starts[-1] == rejected
        raise ValueError("second start could not be imported")

    def should_not_score(*_, **__):
        pytest.fail("a stage started scoring before all its starts were checked")

    monkeypatch.setattr(arena_queue, "preflight", check)
    monkeypatch.setattr(arena_queue, "score_match", should_not_score)
    failed = run(job)
    assert failed["status"] == "failed"
    assert failed["attempts"] == []
    assert failed["reason"] == "ValueError: second start could not be imported"


def test_queue_records_the_predeclared_betting_analysis(job, monkeypatch):
    configure_stages(job, track="non_inferior", advance_at=0, method="betting_paired_v1")
    monkeypatch.setattr(arena_queue, "score_match", lambda *_, **__: result())
    finished = run(job)
    assert finished["stages"][-1]["analysis"]["method"] == "betting_paired_v1"
    assert finished["stages"][-1]["analysis"]["estimate"] == 0
