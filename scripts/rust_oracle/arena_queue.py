"""Persistent local worker for paired Rust arena matches in WSL."""

import argparse
import json
import os
import shutil
import sqlite3
import subprocess
import time
import uuid
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, wait
from contextlib import ExitStack, closing
from hashlib import sha256
from pathlib import Path
from tempfile import NamedTemporaryFile
from threading import Event

from arena_acceptance import guardrail, held_out, stages, validate_acceptance
from arena_match import score_match
from arena_policy import load_policy, validate_policy
from arena_reports import (
    _sync_directory, _write, load_report, load_start, read_report, save_report, save_start,
    save_verification, sha256_file,
)
from oracle import ROOT, Worker, canonical, digest

SIDES = ("RED", "BLUE")
SCHEMA = """
CREATE TABLE IF NOT EXISTS settings (
    name TEXT PRIMARY KEY, value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY, manifest TEXT NOT NULL, status TEXT NOT NULL,
    reason TEXT, created REAL NOT NULL
);
CREATE TABLE IF NOT EXISTS matches (
    job TEXT NOT NULL REFERENCES jobs(id), pair INTEGER NOT NULL, side TEXT NOT NULL,
    report TEXT, verdict TEXT, PRIMARY KEY (job, pair, side)
);
CREATE TABLE IF NOT EXISTS stage_runs (
    job TEXT NOT NULL REFERENCES jobs(id), stage INTEGER NOT NULL,
    outcome TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY (job, stage)
);
CREATE TABLE IF NOT EXISTS stage_preflights (
    job TEXT NOT NULL REFERENCES jobs(id), stage INTEGER NOT NULL,
    binaries TEXT NOT NULL, PRIMARY KEY (job, stage)
);
CREATE TABLE IF NOT EXISTS registered_profiles (
    id TEXT PRIMARY KEY, profile TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS promotions (
    job TEXT PRIMARY KEY REFERENCES jobs(id), evidence TEXT, created REAL NOT NULL
);
CREATE TABLE IF NOT EXISTS attempts (
    id TEXT PRIMARY KEY, job TEXT NOT NULL, pair INTEGER NOT NULL, side TEXT NOT NULL,
    status TEXT NOT NULL, reason TEXT, report TEXT, created REAL NOT NULL,
    FOREIGN KEY (job, pair, side) REFERENCES matches(job, pair, side)
);
"""


class Cancelled(Exception):
    pass


def open_queue(store):
    store.mkdir(parents=True, exist_ok=True)
    db = sqlite3.connect(store / "queue.sqlite3", timeout=30)
    db.row_factory = sqlite3.Row
    db.execute("PRAGMA foreign_keys = ON")
    db.execute("PRAGMA synchronous = FULL")
    db.executescript(SCHEMA)
    return db


def pin_executable(store, source):
    identity = sha256_file(source)
    target = store / "binaries" / identity
    if not target.is_file():
        target.parent.mkdir(parents=True, exist_ok=True)
        with NamedTemporaryFile(dir=target.parent, delete=False) as temp:
            try:
                with source.open("rb") as stream:
                    shutil.copyfileobj(stream, temp)
                temp.flush()
                os.fchmod(temp.fileno(), 0o755)
                os.fsync(temp.fileno())
                if sha256_file(Path(temp.name)) != identity:
                    raise ValueError("executable changed while pinning it")
                os.replace(temp.name, target)
                _sync_directory(target.parent)
            finally:
                Path(temp.name).unlink(missing_ok=True)
    if sha256_file(target) != identity:
        raise ValueError(f"pinned arena executable {identity} is damaged")
    return identity


def resolve_ref(repo, ref):
    return subprocess.check_output(
        ["git", "-C", repo, "rev-parse", "--verify", "--end-of-options", f"{ref}^{{commit}}"],
        text=True,
    ).strip()


def pin_commit(repo, ref):
    commit = resolve_ref(repo, ref)
    subprocess.run(["git", "-C", repo, "update-ref", f"refs/goa2-arena/pinned/{commit}", commit],
                   check=True)
    return commit


def build_policy(store, repo, commit, job_id):
    receipt = store / "builds" / f"{commit}.json"
    if receipt.is_file():
        return json.loads(receipt.read_text(encoding="utf-8"))["binary"]
    checkout = store / "checkouts" / f"{job_id}-{commit}"
    checkout.parent.mkdir(parents=True, exist_ok=True)
    if checkout.exists():
        # A worker can die after adding its detached checkout. Reuse only
        # the checkout for this exact commit and require an unmodified tree.
        if (resolve_ref(checkout, "HEAD") != commit or subprocess.check_output(
            ["git", "-C", checkout, "status", "--porcelain", "--untracked-files=all"],
        )):
            raise ValueError(f"cannot reuse modified arena checkout {checkout}")
    else:
        subprocess.run(["git", "-C", repo, "worktree", "add", "--detach", checkout, commit],
                       check=True, stdout=subprocess.DEVNULL)
    target = store / "build-target"
    env = dict(os.environ, CARGO_TARGET_DIR=str(target))
    log = checkout.parent / f"{checkout.name}.log"
    with log.open("ab") as output:
        try:
            subprocess.run(["sh", "rust/cargo-wsl.sh", "build", "--locked", "--offline",
                            "--profile", "iterate", "-p", "goa2-search-cli"],
                           cwd=checkout, env=env, check=True, stdout=output, stderr=output)
        except subprocess.CalledProcessError as error:
            raise ValueError(f"arena build failed for {commit}, see {log}") from error
    binary = pin_executable(store, target / "iterate" / "goa2-search-cli")
    build = {
        "binary": binary,
        "commit": commit,
        "toolchain_sha256": sha256_file(checkout / "rust" / "rust-toolchain.toml"),
        "lockfile_sha256": sha256_file(checkout / "rust" / "Cargo.lock"),
    }
    _write(receipt, canonical(build).encode("utf-8"))
    subprocess.run(["git", "-C", repo, "worktree", "remove", checkout], check=True)
    return binary


def init_standard(db, store, repo, ref, referee, policy_path=None):
    policy = load_policy(policy_path) if policy_path is not None else {}
    standard = {"commit": pin_commit(repo, ref), "referee": pin_executable(store, referee)}
    if policy:
        standard["policy"] = policy
    with db:
        db.execute("INSERT INTO settings VALUES (?, ?)", ("standard", canonical(standard)))
    return standard


def load_profile(store, path):
    profile = json.loads(path.read_text(encoding="utf-8"))
    if (not isinstance(profile, dict) or profile.get("schema_version") != 1
            or not isinstance(profile.get("name"), str) or not profile["name"]
            or not isinstance(profile.get("starts"), list)):
        raise ValueError("arena profile requires schema_version 1, a name and a starts list")
    parameters = profile.get("parameters")
    if (not isinstance(parameters, dict) or set(parameters) != {"turns", "planning_nodes", "resolution_nodes", "coordinated"}
            or type(parameters["turns"]) is not int or not 1 <= parameters["turns"] <= 64
            or type(parameters["coordinated"]) is not bool
            or any(type(parameters[key]) is not int or not 1 <= parameters[key] < 2**64
                   for key in ("planning_nodes", "resolution_nodes"))):
        raise ValueError("invalid arena match parameters")
    starts = []
    acceptance = profile.get("acceptance")
    if acceptance is not None:
        validate_acceptance(acceptance, profile["starts"])
        if parameters["turns"] != 64:
            raise ValueError("arena strength profiles require the 64-turn limit")
    for row in profile["starts"]:
        if (not isinstance(row, dict) or set(row) != {"envelope", "seed"}
                or not isinstance(row["envelope"], str) or type(row["seed"]) is not int
                or not 0 <= row["seed"] < 2**64):
            raise ValueError("invalid arena start")
        envelope = json.loads((path.parent / row["envelope"]).read_text(encoding="utf-8"))
        if acceptance is not None:
            state = envelope["state"]
            if state.get("game_type") != "LONG" or state.get("phase") == "GAME_OVER":
                raise ValueError("arena strength profiles require active LONG starting states")
        starts.append({"start_sha256": save_start(store, envelope), "seed": row["seed"]})
    if not starts or len({row["start_sha256"] for row in starts}) != len(starts):
        raise ValueError("arena profile requires distinct starting states")
    if acceptance is not None:
        # Replay the same preflight starts twice and compare their full results.
        count = acceptance["preflight_pairs"]
        starts = starts[:count] + starts
    return dict(profile, starts=starts, scorer="arena_search_v1")


def register_profile(db, store, path):
    profile = load_profile(store, path)
    if profile.get("acceptance") is None:
        raise ValueError("registered profiles require predeclared acceptance tiers")
    identity = digest(profile)
    with db:
        db.execute("INSERT OR IGNORE INTO registered_profiles VALUES (?, ?)", (identity, canonical(profile)))
    return identity


def registered_profile(db, identity):
    row = db.execute("SELECT profile FROM registered_profiles WHERE id = ?", (identity,)).fetchone()
    if row is None:
        raise ValueError(f"unknown registered arena profile {identity}")
    profile = json.loads(row["profile"])
    if digest(profile) != identity:
        raise ValueError("registered arena profile integrity check failed")
    return profile


def enqueue(db, store, repo, candidate, profile_path=None, *, profile_id=None, candidate_policy=None):
    accepted = db.execute("SELECT value FROM settings WHERE name = ?", ("standard",)).fetchone()
    if accepted is None:
        raise ValueError("initialize the accepted arena standard first")
    standard = json.loads(accepted["value"])
    profile = registered_profile(db, profile_id) if profile_id else load_profile(store, profile_path)
    baseline_policy = validate_policy(standard.get("policy", {}))
    policy = load_policy(candidate_policy) if candidate_policy is not None else baseline_policy
    manifest = {
        "candidate_policy": policy,
        "standard_policy": baseline_policy,
        "candidate": pin_commit(repo, candidate),
        "standard": standard["commit"],
        "referee": standard["referee"],
        "repo": str(repo),
        "profile": profile,
        "profile_sha256": digest(profile),
        "registered_profile": profile_id,
    }
    job = uuid.uuid4().hex
    with db:
        db.execute("INSERT INTO jobs VALUES (?, ?, ?, NULL, ?)",
                   (job, canonical(manifest), "queued", time.time()))
        db.executemany("INSERT INTO matches (job, pair, side) VALUES (?, ?, ?)", (
            (job, index, side) for index in range(len(profile["starts"])) for side in SIDES
        ))
    return job


def set_status(db, job, status, reason=None):
    db.execute("UPDATE jobs SET status = ?, reason = ? WHERE id = ?", (status, reason, job))


def job_status(db, job):
    return db.execute("SELECT status FROM jobs WHERE id = ?", (job,)).fetchone()["status"]


def cancel_or_retry(db, job, retry):
    with db:
        if retry:
            result = db.execute(
                "UPDATE jobs SET status = ?, reason = NULL WHERE id = ? AND status IN (?, ?)",
                ("queued", job, "cancelled", "failed"),
            )
            if result.rowcount:
                db.execute("UPDATE matches SET verdict = NULL WHERE job = ? AND verdict = ?",
                           (job, "unsupported"))
        else:
            result = db.execute(
                """UPDATE jobs SET status = CASE status WHEN 'running' THEN 'cancel_requested'
                   ELSE 'cancelled' END
                   WHERE id = ? AND status IN ('queued', 'running', 'awaiting_verification')""",
                (job,),
            )
        if result.rowcount != 1:
            raise ValueError(f"job {job} is missing or cannot change state")


def finish_attempt(db, attempt, report):
    with db:
        db.execute("UPDATE attempts SET status = ?, report = ? WHERE id = ?",
                   ("complete", report, attempt["id"]))
        db.execute("UPDATE matches SET report = ? WHERE job = ? AND pair = ? AND side = ?",
                   (report, attempt["job"], attempt["pair"], attempt["side"]))


def publish_receipt(store, receipt):
    if "result" in receipt:
        envelope = load_start(store, receipt["start_sha256"])
        return save_report(store, envelope, receipt["result"])
    load_report(store, receipt["report_id"])
    return receipt["report_id"]


def recover_attempts(db, store, job=None):
    for attempt in db.execute(
        "SELECT * FROM attempts WHERE status = ? AND (? IS NULL OR job = ?)",
        ("running", job, job),
    ).fetchall():
        directory = store / "attempts" / attempt["id"]
        receipt = directory / "complete.json"
        if receipt.is_file():
            report = publish_receipt(store, json.loads(receipt.read_text(encoding="utf-8")))
            _write(receipt, canonical({"report_id": report}).encode("utf-8"))
            finish_attempt(db, attempt, report)
            (directory / "trace.jsonl").unlink(missing_ok=True)
        else:
            with db:
                db.execute("UPDATE attempts SET status = ?, reason = ? WHERE id = ?",
                           ("interrupted", "worker exited before match completion", attempt["id"]))


def recover_interrupted(db, store):
    recover_attempts(db, store)
    with db:
        db.execute("UPDATE jobs SET status = ? WHERE status = ?", ("queued", "running"))
        db.execute("UPDATE jobs SET status = ? WHERE status = ?", ("cancelled", "cancel_requested"))


def check_cancelled(db, job):
    if job_status(db, job) == "cancel_requested":
        raise Cancelled("job cancelled")


def run_match(db, store, job, match, manifest, binaries, *, interrupted=None):
    check_cancelled(db, job)
    attempt_id = uuid.uuid4().hex
    side = match["side"]
    profile = manifest["profile"]
    start = profile["starts"][match["pair"]]
    parameters = profile["parameters"].copy()
    config = profile.get("acceptance")
    if config is not None and match["pair"] < 2 * config["preflight_pairs"]:
        parameters["turns"] = 1
    directory = store / "attempts" / attempt_id
    directory.mkdir(parents=True)
    with db:
        db.execute("INSERT INTO attempts VALUES (?, ?, ?, ?, ?, NULL, NULL, ?)",
                   (attempt_id, job, match["pair"], side, "running", time.time()))
    attempt = {"id": attempt_id, "job": job, "pair": match["pair"], "side": side}
    _sync_directory(directory.parent)
    other_side = SIDES[1 - SIDES.index(side)]
    roles = {"referee": manifest["referee"], side: binaries["candidate"], other_side: binaries["standard"]}
    policies = {side: manifest.get("candidate_policy", {}), other_side: manifest.get("standard_policy", {})}
    if any(policies.values()):
        parameters["policy_profiles"] = policies
    paths = {role: store / "binaries" / identity for role, identity in roles.items()}
    trace = directory / "trace.jsonl"
    try:
        with trace.open("ab") as output:
            _sync_directory(directory)

            def record(decision):
                output.write((canonical(decision) + "\n").encode("utf-8"))
                output.flush()
                os.fsync(output.fileno())
                if interrupted is not None and interrupted.is_set():
                    raise KeyboardInterrupt
                check_cancelled(db, job)

            result = score_match(paths, load_start(store, start["start_sha256"]),
                                 record=record, expected_binaries=roles, seed=start["seed"],
                                 **parameters)
    except (Exception, KeyboardInterrupt) as error:
        with db:
            db.execute("UPDATE attempts SET status = ?, reason = ? WHERE id = ?",
                       ("failed", f"{type(error).__name__}: {error}", attempt_id))
        raise
    result["queue"] = {"job": job, "pair": match["pair"], "candidate_side": side,
                       "profile_sha256": manifest["profile_sha256"],
                       "candidate": manifest["candidate"], "standard": manifest["standard"]}
    receipt = directory / "complete.json"
    payload = {"start_sha256": start["start_sha256"], "result": result}
    _write(receipt, canonical(payload).encode("utf-8"))
    report = publish_receipt(store, payload)
    _write(receipt, canonical({"report_id": report}).encode("utf-8"))
    finish_attempt(db, attempt, report)
    trace.unlink(missing_ok=True)


def preflight(store, manifest, binaries, starts):
    roles = dict(binaries, referee=manifest["referee"])
    with ExitStack() as stack:
        workers = {}
        for role, identity in roles.items():
            if identity not in workers:
                path = store / "binaries" / identity
                if sha256_file(path) != identity:
                    raise ValueError(f"arena executable changed for {role}")
                worker = stack.enter_context(closing(Worker([str(path)])))
                workers[identity] = (role, worker, worker.request("capabilities", timeout=30))
            caps = workers[identity][2]
            required = ("arena_match_v1",) if role == "referee" else ("state_codec", "nodes_v1")
            if not all(caps.get(item) for item in (*required, "sequence_legality")):
                raise ValueError(f"arena protocol is incompatible for {role}")
        for identity in dict.fromkeys(starts):
            envelope = load_start(store, identity)
            for role, worker, _ in workers.values():
                try:
                    worker.request("check_sequences", envelope=envelope, sequences=[], timeout=30)
                except (RuntimeError, TimeoutError) as error:
                    raise ValueError(f"{role} could not import arena start {identity}: {error}") from error

def verify_pending(db, store, job, referee):
    from arena_verify import VERIFIER_VERSION, ReplayMismatch, ReplayUnsupported, verify_report

    rows = db.execute("""SELECT * FROM matches WHERE job = ? AND report IS NOT NULL
                          AND verdict IS NULL ORDER BY pair, side DESC""", (job,)).fetchall()
    for row in rows:
        identity = row["report"]
        verdict = {"verifier_version": VERIFIER_VERSION, "report_id": identity}
        try:
            report, envelope = load_report(store, identity)
            verdict.update(status="pass", **verify_report(report, envelope, referee))
        except ReplayMismatch as error:
            verdict.update(status="mismatch", reason=str(error), **error.details)
        except (ReplayUnsupported, ValueError, OSError) as error:
            verdict.update(status="unsupported", reason=str(error))
        save_verification(store, identity, verdict)
        with db:
            db.execute("UPDATE matches SET verdict = ? WHERE job = ? AND pair = ? AND side = ?",
                       (verdict["status"], job, row["pair"], row["side"]))



def stage_index(db, job):
    return db.execute("SELECT COUNT(*) FROM stage_runs WHERE job = ?", (job,)).fetchone()[0]


def stage_matches(db, job, begin, end):
    return db.execute("""SELECT * FROM matches WHERE job = ? AND pair >= ? AND pair < ?
                         ORDER BY pair, side DESC""", (job, begin, end)).fetchall()


def run_pair(db, store, job, matches, manifest, binaries, match_workers):
    if match_workers == 1 or len(matches) == 1:
        for match in matches:
            run_match(db, store, job, match, manifest, binaries)
        return

    interrupted = Event()

    def score(match):
        with closing(open_queue(store)) as connection:
            run_match(connection, store, job, match, manifest, binaries, interrupted=interrupted)

    # Wait for both assignments even if one fails. The other can publish a
    # completed report and be skipped if the job is retried.
    with ThreadPoolExecutor(max_workers=match_workers) as pool:
        try:
            futures = [pool.submit(score, match) for match in matches]
            wait(futures)
        except KeyboardInterrupt:
            interrupted.set()
            raise
        for future in futures:
            future.result()


def run_job(db, store, row, match_workers=1):
    job = row["id"]
    manifest = json.loads(row["manifest"])
    with db:
        claimed = db.execute("UPDATE jobs SET status = ? WHERE id = ? AND status = ?",
                             ("running", job, "queued"))
        if claimed.rowcount != 1:
            return
        db.execute("""
            INSERT INTO settings VALUES ('dispatches', '1')
            ON CONFLICT(name) DO UPDATE SET value = CAST(value AS INTEGER) + 1
        """)
    try:
        recover_attempts(db, store, job)
        if digest(manifest["profile"]) != manifest["profile_sha256"]:
            raise ValueError("queued arena profile integrity check failed")
        if manifest["profile"].get("scorer") != "arena_search_v1":
            raise ValueError("unsupported arena scoring profile")
        repo = Path(manifest["repo"])
        binaries = {}
        for role in ("candidate", "standard"):
            check_cancelled(db, job)
            binaries[role] = build_policy(store, repo, manifest[role], job)
        index = stage_index(db, job)
        profile = manifest["profile"]
        _, begin, end = stages(profile)[index]
        pin = canonical(dict(binaries, referee=manifest["referee"]))
        checked = db.execute("SELECT binaries FROM stage_preflights WHERE job = ? AND stage = ?",
                             (job, index)).fetchone()
        if checked is None or checked["binaries"] != pin:
            starts = [row["start_sha256"] for row in profile["starts"][begin:end]]
            preflight(store, manifest, binaries, starts)
            with db:
                check_cancelled(db, job)
                db.execute("INSERT OR REPLACE INTO stage_preflights VALUES (?, ?, ?)", (job, index, pin))
        remaining = [match for match in stage_matches(db, job, begin, end)
                     if match["report"] is None]
        if remaining:
            pair = remaining[0]["pair"]
            matches = [match for match in remaining if match["pair"] == pair]
            run_pair(db, store, job, matches, manifest, binaries, match_workers)
        with db:
            check_cancelled(db, job)
            pending = any(match["report"] is None for match in stage_matches(db, job, begin, end))
            if pending:
                db.execute("UPDATE jobs SET status = ? WHERE id = ?", ("queued", job))
            else:
                set_status(db, job, "awaiting_verification")
    except Cancelled as error:
        with db:
            set_status(db, job, "cancelled", str(error))
    except Exception as error:
        with db:
            set_status(db, job, "failed", f"{type(error).__name__}: {error}")


def compare_preflight(store, rows, count):
    """Compare repeated fixed-budget reports apart from their queue identity."""
    results = {}
    for row in rows:
        report = read_report(store, row["report"])
        if not report["trace"]:
            return False
        report.pop("queue")
        results[row["pair"], row["side"]] = digest(report)
    return all(results[index, side] == results[index + count, side]
               for index in range(count) for side in SIDES)


def stage_result(db, store, job, profile, index):
    definitions = stages(profile)
    name, begin, end = definitions[index]
    rows = stage_matches(db, job, begin, end)
    if any(row["verdict"] == "mismatch" for row in rows):
        return "rejected", "oracle found a rules mismatch", {}
    if any(row["verdict"] != "pass" for row in rows):
        return "failed", "arena rules verification requires review", {}
    if name == "diagnostic":
        return "verified", "diagnostic matches verified", {}
    config = profile["acceptance"]
    if name == "preflight":
        if not compare_preflight(store, rows, config["preflight_pairs"]):
            return "rejected", "repeated node-budget preflight differed", {}
        work = summarize_matches(store, rows)["work"]
        for role, counters in work.items():
            for phase in ("planning", "resolution"):
                completed = (counters.get(f"{phase}_requests", 0)
                             - counters.get(f"{phase}_unsupported", 0)
                             - counters.get(f"{phase}_unscored", 0))
                if completed <= 0:
                    return "rejected", f"preflight did not score a {role} {phase} decision", {"work": work}
        return "advance", "preflight verified and deterministic", {"work": work}

    if name == "extend":
        rows = stage_matches(db, job, definitions[1][1], end)
    summary = summarize_matches(store, rows)
    reason = guardrail(summary, config)
    if reason is not None:
        return "rejected", reason, summary
    if name == "confirm":
        analysis = held_out(paired_values(summary["pairs"]), config)
        outcome = analysis["status"]
        return outcome, f"held-out {config['track']} result is {outcome}", dict(summary, analysis=analysis)
    score = summary["paired_advantage"]
    if score is not None and score < config["stop_below"]:
        return "rejected", "tuning result below the futility threshold", summary
    next_is_confirm = definitions[index + 1][0] == "confirm"
    if score is not None and score >= config["advance_at"]:
        return "advance", "tuning advancement threshold met", summary
    if next_is_confirm:
        return "inconclusive", "tuning allowance ended below the advancement threshold", summary
    return "extend_low", "uncertain tuning result receives lower priority", summary


def finalize_stage(db, store, job, profile):
    index = stage_index(db, job)
    outcome, reason, details = stage_result(db, store, job, profile, index)
    status = "queued" if outcome in {"advance", "extend_low"} else outcome
    with db:
        updated = db.execute("UPDATE jobs SET status = ?, reason = ? WHERE id = ? AND status = ?",
                             (status, reason, job, "awaiting_verification"))
        if updated.rowcount and outcome != "failed":
            name = stages(profile)[index][0]
            data = dict(details, stage=name, reason=reason)
            db.execute("INSERT INTO stage_runs VALUES (?, ?, ?, ?)",
                       (job, index, outcome, canonical(data)))


def drain_verifications(db, store):
    rows = db.execute("""
        SELECT * FROM jobs WHERE status = 'awaiting_verification' OR (status IN ('failed', 'cancelled', 'rejected') AND EXISTS (
            SELECT 1 FROM matches WHERE matches.job = jobs.id
            AND report IS NOT NULL AND verdict IS NULL
        )) ORDER BY created
    """).fetchall()
    for row in rows:
        job = row["id"]
        manifest = json.loads(row["manifest"])
        referee = store / "binaries" / manifest["referee"]
        verify_pending(db, store, job, referee)
        if job_status(db, job) == "awaiting_verification":
            finalize_stage(db, store, job, manifest["profile"])


def next_job(db):
    clock = db.execute("SELECT value FROM settings WHERE name = 'dispatches'").fetchone()
    reserve_screen = (int(clock["value"]) + 1) % 4 == 0 if clock else False
    return db.execute("""
        WITH pending AS (
            SELECT jobs.*, COUNT(stage_runs.stage) AS stage_count,
                   (SELECT outcome FROM stage_runs AS latest WHERE latest.job = jobs.id
                    ORDER BY stage DESC LIMIT 1) AS last_outcome
            FROM jobs LEFT JOIN stage_runs ON stage_runs.job = jobs.id
            WHERE jobs.status = 'queued' GROUP BY jobs.id
        )
        SELECT * FROM pending
        ORDER BY CASE
            WHEN stage_count < 2 AND ? THEN 0
            WHEN stage_count >= 2 AND last_outcome = 'advance' THEN 1
            WHEN stage_count < 2 THEN 2
            ELSE 3 END, created, id
        LIMIT 1
    """, (reserve_screen,)).fetchone()


def run_queue(db, store, watch, match_workers=1):
    import fcntl

    with (store / "worker.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ValueError("an arena worker already owns this queue") from error
        recover_interrupted(db, store)
        while True:
            drain_verifications(db, store)
            row = next_job(db)
            if row is not None:
                run_job(db, store, row, match_workers)
            elif watch:
                time.sleep(2)
            else:
                return


def paired_values(pairs):
    values = {"win": 1, "draw": 0, "loss": -1}
    return [sum(values[outcome] for outcome in pair.values()) / 2
            if len(pair) == 2 and all(outcome in values for outcome in pair.values()) else None
            for pair in pairs.values()]


def summarize_matches(store, matches):
    totals = Counter()
    work = {"candidate": Counter(), "standard": Counter()}
    pairs = {}
    for row in matches:
        pair = pairs.setdefault(row["pair"], {})
        if row["report"] is None:
            continue
        report = read_report(store, row["report"])
        endpoint = report["endpoint"]
        if endpoint["phase"] != "GAME_OVER":
            outcome = "unfinished"
        else:
            winner = endpoint["winner_team"]
            if winner not in SIDES and endpoint["winner_hero"] is not None:
                outcome = "individual_winner"
            elif winner == row["side"]:
                outcome = "win"
            elif winner in SIDES:
                outcome = "loss"
            else:
                outcome = "draw"
        totals[outcome] += 1
        totals["verified"] += row["verdict"] == "pass"
        pair[row["side"]] = outcome
        for side, counters in report["work"].items():
            work["candidate" if side == row["side"] else "standard"].update(counters)
    paired = [value for value in paired_values(pairs) if value is not None]
    return {"outcomes": dict(totals), "work": {role: dict(row) for role, row in work.items()},
            "pairs": pairs, "scored_pairs": len(paired),
            "paired_advantage": sum(paired) / len(paired) if paired else None}


def describe_jobs(db, store, job):
    rows = db.execute("SELECT * FROM jobs WHERE (? IS NULL OR id = ?) ORDER BY created",
                      (job, job)).fetchall()
    if job is not None and not rows:
        raise ValueError(f"unknown arena job {job}")
    result = []
    for row in rows:
        item = dict(row, manifest=json.loads(row["manifest"]))
        item["matches"] = [dict(match) for match in db.execute(
            "SELECT * FROM matches WHERE job = ? ORDER BY pair, side DESC", (row["id"],)
        )]
        item["stages"] = [dict(json.loads(stage["data"]), index=stage["stage"], outcome=stage["outcome"])
                          for stage in db.execute(
                              "SELECT * FROM stage_runs WHERE job = ? ORDER BY stage", (row["id"],)
                          )]
        if item["manifest"]["profile"].get("acceptance") is None:
            item["result"] = summarize_matches(store, item["matches"])
        item["attempts"] = [dict(attempt) for attempt in db.execute(
            "SELECT * FROM attempts WHERE job = ? ORDER BY created", (row["id"],)
        )]
        result.append(item)
    return result


def promote(db, store, job, benefit_evidence):
    with db:
        db.execute("BEGIN IMMEDIATE")
        row = db.execute("SELECT * FROM jobs WHERE id = ?", (job,)).fetchone()
        if row is None or row["status"] != "passed":
            raise ValueError("promotion requires a passing held-out result")
        manifest = json.loads(row["manifest"])
        profile_id = manifest.get("registered_profile")
        if not profile_id or registered_profile(db, profile_id) != manifest["profile"]:
            raise ValueError("promotion requires a profile registered before submission")
        current = json.loads(db.execute(
            "SELECT value FROM settings WHERE name = 'standard'"
        ).fetchone()["value"])
        if (manifest["standard"] != current["commit"] or manifest["referee"] != current["referee"]
                or manifest.get("standard_policy", {}) != current.get("policy", {})):
            raise ValueError("the accepted standard or referee changed; confirm against the current standard")
        matches = db.execute("SELECT report, verdict FROM matches WHERE job = ?", (job,)).fetchall()
        for match in matches:
            identity = match["report"]
            if not identity or match["verdict"] != "pass":
                raise ValueError("promotion requires a verified report for every match")
            load_report(store, identity)
            verdict = json.loads((store / "verification" / f"{identity}.json").read_text(encoding="utf-8"))
            if verdict.get("status") != "pass" or verdict.get("report_id") != identity:
                raise ValueError(f"missing passing oracle evidence for {identity}")
        track = manifest["profile"]["acceptance"]["track"]
        evidence_hash = None
        if track != "stronger" and benefit_evidence is None:
            raise ValueError("comparable strength requires separate reviewed benefit evidence")
        if benefit_evidence is not None:
            evidence = benefit_evidence.read_bytes()
            if not evidence.strip():
                raise ValueError("benefit evidence is empty")
            evidence_hash = sha256(evidence).hexdigest()
            _write(store / "benefits" / evidence_hash, evidence)
        current["commit"] = manifest["candidate"]
        candidate_policy = manifest.get("candidate_policy", {})
        if candidate_policy:
            current["policy"] = candidate_policy
        else:
            current.pop("policy", None)
        db.execute("UPDATE settings SET value = ? WHERE name = 'standard'", (canonical(current),))
        db.execute("INSERT INTO promotions VALUES (?, ?, ?)", (job, evidence_hash, time.time()))
        set_status(db, job, "promoted", "held-out result accepted by explicit review")
    return current


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--store", type=Path, required=True)
    parser.add_argument("--repo", type=Path, default=ROOT)
    commands = parser.add_subparsers(dest="command", required=True)
    init = commands.add_parser("init")
    init.add_argument("--standard", required=True)
    init.add_argument("--referee", type=Path, required=True)
    init.add_argument("--policy", type=Path)
    registration = commands.add_parser("register-profile")
    registration.add_argument("--profile", type=Path, required=True)
    submit = commands.add_parser("enqueue")
    submit.add_argument("candidate")
    submit.add_argument("--candidate-policy", type=Path)
    source = submit.add_mutually_exclusive_group(required=True)
    source.add_argument("--profile", type=Path)
    source.add_argument("--profile-id")
    status = commands.add_parser("status")
    status.add_argument("job", nargs="?")
    for name in ("cancel", "retry"):
        commands.add_parser(name).add_argument("job")
    review = commands.add_parser("promote")
    review.add_argument("job")
    review.add_argument("--benefit-evidence", type=Path)
    worker = commands.add_parser("run")
    worker.add_argument("--watch", action="store_true")
    worker.add_argument("--match-workers", type=int, choices=(1, 2), default=1,
                        help="score both side assignments concurrently with 2")
    args = parser.parse_args()
    if args.command == "run":
        from arena_verify import ReplayUnsupported, verified_oracle

        try:
            verified_oracle()
        except ReplayUnsupported as error:
            parser.error(str(error))
    store, repo = args.store.resolve(), args.repo.resolve()
    with closing(open_queue(store)) as db:
        try:
            if args.command == "init":
                print(canonical(init_standard(db, store, repo, args.standard, args.referee, args.policy)))
            elif args.command == "register-profile":
                print(register_profile(db, store, args.profile))
            elif args.command == "enqueue":
                print(enqueue(db, store, repo, args.candidate, args.profile,
                              profile_id=args.profile_id, candidate_policy=args.candidate_policy))
            elif args.command == "status":
                print(canonical(describe_jobs(db, store, args.job)))
            elif args.command == "promote":
                print(canonical(promote(db, store, args.job, args.benefit_evidence)))
            elif args.command in ("cancel", "retry"):
                cancel_or_retry(db, args.job, args.command == "retry")
            else:
                run_queue(db, store, args.watch, args.match_workers)
        except (ValueError, OSError, sqlite3.Error, subprocess.CalledProcessError) as error:
            parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
