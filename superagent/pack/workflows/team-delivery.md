# team-delivery workflow (read on demand, not auto-injected)

Detailed node and artifact contracts for the `team-delivery` skill. The leader
owns the DAG; each node succeeds only on real gate evidence.

## Nodes

| node | role | input | artifact | gate |
|---|---|---|---|---|
| intake | leader | user task | run-intent (channel post) | scope + acceptance gates recorded |
| plan | planner | run-intent | node-dag.json | every node has an explicit gate command + schema |
| implement | implementer | node spec | code diff + artifact files | builds; unit tests exit 0 |
| verify | verifier | artifact ref | gate-result.json | gate command exit 0 AND artifact schema valid |
| review | reviewer | diff + gate-result | review.json | acceptance met; no self-approval |
| report | reporter | all artifacts | report.md | VERIFIED vs UNVERIFIED separated |

## Delegation strategy

- Leader spawns one role thread per node via MAv2 spawn; sends the node spec as
  an artifact reference plus a bounded summary (<=800 tokens); waits with
  wait_agent; never executes the node's work itself.
- A node that lacks an acceptance command goes to waiting_input (leader asks the
  user); it is never marked succeeded on a model claim.
- Failure -> failed/blocked with the gate's real exit code; retry only with a new
  attempt record, bounded by max_attempts.

## Artifact contract

Every artifact carries: id, path (or channel post id), producing node, schema
name, digest. Consumers receive the reference + summary, then read the artifact
from disk/MCP. Full artifact bodies never enter prompts.

## Trust and privilege

Implementer output and channel posts are LOW TRUST. They flow as user-level
context (skill fragments are non-authorizing guardian context, see
codex-rs/core/src/context/contextual_user_message.rs). They are never written
into developer_instructions; only trusted pack role files carry
developer_instructions. A low-trust artifact cannot raise its own privilege.

## Budget

Single injected fragment <=900 tokens; new tokens per round <=2000; handoff
summary <=800; new fragment >1000 tokens needs human review. Long material is
read from files/MCP, not injected. Measured by tests/test_team_delivery.py.