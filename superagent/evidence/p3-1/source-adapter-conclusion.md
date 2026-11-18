# P3-1 / #87 — memory-service runtime source adapter (real-fixture integration)

Status: implementation + integration tests PASS. Closure gated on #77 (P1-5)
ready + grok independent review APPROVE on the latest SHA (per the #87 closure
gate comment). This document records the runtime integration evidence.

## What was added

`crates/memory-service/src/source_adapter.rs` — a model-free, process-free
`SourceAdapter::sync(svc, roots)` that connects the FTS5 index to the genuine
product write paths:

- `SourceKind::Rollout` ← `*.jsonl` session rollout files;
- `SourceKind::Memory` ← `*.md` / `*.txt` memory notes;
- `SourceKind::Board` ← `*.json` / `*.jsonl` board posts.

Each pass:
1. scans each root recursively, extracts searchable prose (rollout/board JSON
   message/text/content/body/title/summary fields; memory raw prose), and calls
   `MemoryService::index(kind, source_id, text, metadata)` — new files Inserted,
   changed files Reindexed (content-hash gated), unchanged files Unchanged;
2. queries `MemoryService::source_ids(kind)` (new accessor) and calls
   `remove(kind, id)` for every indexed source whose file disappeared.

No engine, model, or vector backend is spawned; the reserved `VectorIndex`
embeddings branch remains an explicit unsupported stub. The adapter reads files
already on disk, so it can run inline on the write path without blocking on
inference.

## Real fixtures (genuine engine output, not synthetic)

`crates/memory-service/tests/fixtures/rollouts/rollout-{0,1,2}.jsonl` are real
captured `codex-tui` session rollouts from the P1-5 fixture demo home
(`evidence/p1-5/home-p1-5-tui/sessions`):

- rollout-0: originator=codex-tui, agent_role=reporter
- rollout-1: originator=codex-tui, leader (no agent_role)
- rollout-2: originator=codex-tui, agent_role=planner

Each contains the genuine calc.py fix task conversation (the term `calc`
appears 18/22/16 times respectively).

## Closure-gate criteria → evidence (tests/source_adapter.rs, 5 tests)

| Gate requirement | Test | Result |
| --- | --- | --- |
| new session/post auto-indexed + retrievable hit | `real_rollouts_auto_index_and_retrieve` | PASS — 5 inserted (3 rollout+1 memory+1 board); `search("calc")` returns a `rollout` hit citing source_id + snippet |
| changed content → old query no longer hits | `changed_content_invalidates_old_query` | PASS — board marker hit before rewrite; after rewrite reindexed≥1 and old marker query empty, new term hits |
| deleted file removed from index | `deleted_file_is_removed_from_index` | PASS — removed≥1, count(Rollout) 3→2, id gone from source_ids |
| Chinese query hits (trigram CJK) | `chinese_trigram_query_hits_memory` | PASS — `search("元智能体")` hits the memory note |
| incremental / non-blocking / latency baseline | `unchanged_pass_is_idempotent_and_search_is_bounded` | PASS — unchanged second pass: inserted/reindexed/removed=0, unchanged=5; search latency baseline **6.28 ms** for 4 hits (< 500 ms bound) |

## Gate (CI toolchain 1.99.1)

- `cargo +1.99.1 fmt --check`: exit 0 (see `fmt-clippy.log`)
- `cargo +1.99.1 clippy --all-targets -- -D warnings`: exit 0, error-lines=0 (see `fmt-clippy.log`)
- `cargo +1.99.1 test`: 0 unit + 20 integration (15 core `memory_service` + 5 `source_adapter`) = **20 passed, 0 failed**

See `test-output.log` for the full test run including the latency baseline line,
and `fmt-clippy.log` for the committed fmt/clippy command trace.

Refs #87

## Honesty / scope

- This is the runtime write-path adapter + real-fixture integration proof
  required by the #87 closure gate. It does NOT claim embeddings/vector
  retrieval (reserved optional posterior, explicit unsupported stub).
- Closure still requires #77 ready and grok APPROVE on the latest SHA; this
  document is evidence回填, not a self-approval.