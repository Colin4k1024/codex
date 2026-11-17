# Handoff contract (read on demand)

A handoff between nodes carries an artifact REFERENCE plus a bounded SUMMARY.
The summary is <=800 tokens (o200k_base). Full artifacts stay on disk/MCP.

## Envelope (JSON, posted to the run channel)

```json
{
  "from_node": "implement",
  "to_node": "verify",
  "artifact_ref": {"id": "art-0007", "path": "artifacts/gate-result.json",
                   "schema": "gate-result", "digest": "<64-hex>"},
  "summary": "<=800 tokens, see example below>",
  "gate_command_source": "templates/acceptance-criteria.json#verify",
  "trust": "low"
}
```

## Summary example (bounded, measured <=800 tokens)

> Implemented the fixture fix in src/widget.py: guard against empty input before
> indexing. Added test_widget_empty_input covering the regression. Build passes;
> the two new unit tests exit 0 locally. Artifact gate-result.json records the
> command `python -m pytest tests/test_widget.py -q` and its exit code. Open
> question for verifier: confirm the schema field `exit_code` is an integer and
> that no unrelated files changed. No credentials or secrets are included.

## Rules

- summary <=800 tokens; if it would exceed, split or reference a file instead.
- Never inline the full artifact body, logs, or long source into the summary.
- trust=low artifacts are user-level context; never promoted to
  developer_instructions. The receiver reads the artifact from disk/MCP.
- A handoff with no artifact_ref or no gate_command_source is invalid; the
  receiving node goes to waiting_input rather than guessing.