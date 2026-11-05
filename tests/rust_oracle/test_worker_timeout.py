import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/rust_oracle"))

from oracle import Worker, WorkerUnavailable


def test_worker_reuses_process_until_a_request_times_out():
    program = """
import json
import sys
import time

for line in sys.stdin:
    request = json.loads(line)
    if request["operation"] == "stall":
        time.sleep(60)
    if request["operation"] == "unsupported":
        print(json.dumps({"schema_version": 1, "id": request["id"],
                          "ok": False, "error": "unsupported rule"}), flush=True)
        continue
    print(json.dumps({"schema_version": 1, "id": request["id"],
                      "ok": True, "result": request["id"]}), flush=True)
"""
    worker = Worker([sys.executable, "-c", program])
    try:
        assert worker.request("ready", timeout=5) == 1
        with pytest.raises(RuntimeError, match="unsupported rule"):
            worker.request("unsupported", timeout=5)
        assert worker.request("ready", timeout=5) == 3
        with pytest.raises(TimeoutError, match="stall"):
            worker.request("stall", timeout=0.1)
        assert worker.process.poll() is not None
        with pytest.raises(RuntimeError, match="retired"):
            worker.request("ready")
    finally:
        worker.close()


@pytest.mark.parametrize("action", [
    "raise SystemExit(7)",
    "print('not json', flush=True)",
    "print('{\"schema_version\":1,\"id\":999,\"ok\":true,\"result\":0}', flush=True)",
])
def test_broken_worker_is_retired_after_transport_or_protocol_failure(action):
    program = f"import sys; sys.stdin.readline(); {action}"
    worker = Worker([sys.executable, "-c", program])
    try:
        with pytest.raises(WorkerUnavailable):
            worker.request("ready", timeout=5)
        assert worker.process.poll() is not None
        with pytest.raises(WorkerUnavailable, match="retired"):
            worker.request("ready")
    finally:
        worker.close()
