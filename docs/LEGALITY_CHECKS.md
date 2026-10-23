# Batch and sequence legality checks

These offline helpers test proposed decisions without modifying the supplied
game state. Register card effects before loading/creating a game, as for search.
They live in the bots checkout; no HTTP endpoint or lab restart is needed.

## Candidate decisions at one state

```python
from automata.runtime.legality import DecisionMove, check_moves
from goa2.domain.input import selection_value

# request is the current engine prompt addressed to a specific hero.
moves = [DecisionMove("INPUT", request.player_id, selection_value(option),
                      request_id=request.id) for option in request.options]
checked = check_moves(state, moves)
assert all(result.legal is True for result in checked.results)
```

One private clone prepares the current prompt and indexes its legal selections.
Input candidates are checked against that same unchanged state; they do not run
one after another. Repeated candidates share their verdict. Planning candidates
use actual planning methods in isolated sessions, preserving normal and Emmitt
second-card/finish-planning rules. Structured rune batches and simultaneous
upgrades use the engine's `validate_input` hook rather than scalar membership.
For simultaneous upgrades, use the acting hero ID, also present in the payload.

`DecisionMove.kind` is `CARD`, `INPUT`, `PASS`, or `FINISH_PLANNING`. `selection`
is the card ID for `CARD`, or the normal client selection for `INPUT`. Explicit
input request IDs are strict. Omit the ID for hypothetical future prompts whose
IDs are not known yet. The actor must match the current prompt or its team;
upgrade payloads must belong to the acting player.

## Complete proposed sequences

```python
from automata.runtime.legality import check_sequences

checked = check_sequences(state, [
    [first_decision, second_decision, candidate_a],
    [first_decision, second_decision, candidate_b],
])
for result in checked.results:
    print(result.legal, result.failed_at, result.reason)
```

The sequence batch builds a prefix trie. Shared decisions execute once through
`GameSession`, including automatic rules steps and changed prompts/positions.
Worlds fork only where distinct continuations need independent mutations.
Terminal input candidates require only the current engine legality check; their
consequences need not run to prove that submission legal. No opponent decisions
or missing player inputs are invented. A sequence may end at a decision boundary;
this checks legality, not whether its outcome is strategically desirable.

Results retain input order. `checked` counts accepted decisions; `failed_at`
is the zero-based first failure. Invalid prefixes reject their descendants.
`legal=None` means a custom unenumerated request has no engine validation hook,
so legality cannot be established; it is never silently called legal or illegal.
Unexpected engine errors propagate. `applied_decisions` and `state_clones` count
actual work rather than the number of submitted sequences.

Caches are per call. Reusing a filter or changing the source board, acting unit,
range, card, or context cannot retain a verdict from an earlier call. Movement
filters prepare one reachable set per unchanged scan; direct calls are fresh.

## Many destinations from one origin

```python
from goa2.engine.rules import validate_movement_paths

legal = validate_movement_paths(state.board, start, destinations, 4,
                               state=state, actor_id=hero_id,
                               topology_unit_ids=[hero_id], moved_ids=[hero_id])
```

The booleans follow destination order, including duplicates. Prepared search
boards use one exact bitset traversal, plus a terminal-neighbor pass when
`ignore_obstacles=True` permits an occupied destination. This is distinct from
`pass_through_obstacles=True`, which permits crossing intermediate obstacles.
The single-destination `validate_movement_path` also uses this path. Special
spatial effects, passable tokens, custom validators and ordinary dict boards
retain the original target-specific rules implementation.

## Measurements and validation

Three-run median on the recorded four-hero map, with independently verified
identical results:

| Workload | Independent checks | Batch checks |
| --- | ---: | ---: |
| 572 destinations, movement range 4, prepared search board | 395.08 ms | 1.07 ms |
| 64 distinct sequences sharing five decisions, one valid tail and 63 rejected tails | 148.74 ms | 2.82 ms |

The sequence batch executed five prefix decisions using one world clone.
These are legality throughput measurements, not completed ISMCTS simulations.
A separate full 200-iteration search retained all root visits, scores and choice.

Regression coverage: `tests/automata/test_legality.py` and
`tests/automata/test_movement_legality_batch.py`. It compares batches against the
original BFS across obstacle flags, origins, ranges and destinations; exercises
real game traces, stale IDs, ownership, structured rune/upgrade payloads, Emmitt
planning, branch isolation, and destinations made legal by an earlier move.


Validation completed: 3,555 engine/domain and affected search tests passed.
After tightening unknown-input error classification, all 29 focused legality
regressions passed again. Lint, formatting and whitespace checks are clean.
The earlier search checkpoint is commit `601df58` (`Search optimization`);
the legality optimization is separate from that earlier checkpoint.
