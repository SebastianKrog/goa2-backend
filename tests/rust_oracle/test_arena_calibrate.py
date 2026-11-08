import pytest

from arena_calibrate import run_cases, summarize


class Worker:
    def __init__(self):
        self.requests = []

    def request(self, operation, **fields):
        self.requests.append((operation, fields))
        return {"score": 10, "nodes": fields["max_nodes"], "completed_rows": 1}


def test_calibration_uses_fixed_caps_and_reverses_order_between_repeats():
    worker = Worker()
    request = {"work_budget": "nodes_v1", "max_nodes": 1}
    rows = run_cases(worker, [{"phase": "planning", "operation": "search_sampled_planning",
                               "request": request}], {"planning": [2, 8]}, 2)
    assert request["max_nodes"] == 1
    assert [fields["max_nodes"] for _, fields in worker.requests] == [2, 8, 8, 2]
    assert all(fields["work_budget"] == "nodes_v1" and "time_ms" not in fields
               for _, fields in worker.requests)
    assert [(row["cap"], row["scored"], row["max_nodes"]) for row in summarize(rows)] == [
        (2, 2, 2), (8, 2, 8),
    ]


def test_repeated_budget_rejects_a_changed_result():
    class VaryingWorker(Worker):
        def request(self, operation, **fields):
            result = super().request(operation, **fields)
            return dict(result, score=len(self.requests))

    with pytest.raises(ValueError, match="node-budget result changed"):
        run_cases(VaryingWorker(), [{"phase": "planning", "operation": "planning", "request": {}}],
                  {"planning": [2]}, 2)
