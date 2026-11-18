//! SuperAgent independent state store (P2-3 / #80).
//!
//! A product-side SQLite database that is deliberately separate from the engine
//! rollout schema. It persists run/node/attempt/event/artifact state, holds an
//! exclusive per-run lease, records intents transactionally before any external
//! ID exists, and exposes an auto-increment event sequence for UI replay.
//!
//! Design rules inherited from HARNESS_STANDALONE §8:
//! - logs live in files; the database stores only bounded summaries / indexes /
//!   references (oversize summaries are rejected explicitly, never silently
//!   truncated or fabricated);
//! - an intent is committed in its own transaction first; external thread/turn/
//!   process bindings are written afterwards in a separate transaction, so a
//!   crash between the two leaves a recoverable, unbound intent;
//! - send/persist breakpoints are injectable (see [`Breakpoint`]) so P2-7 (#84)
//!   can drive fault-injection tests against real commit boundaries.
//!
//! This crate never spawns a process or calls a model.

use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Hard cap on any single bounded summary stored in the database (bytes).
/// Larger material must live in a file and be referenced by path/digest.
pub const MAX_SUMMARY_BYTES: usize = 4096;

/// Current schema version migrated by [`Store::open`].
pub const SCHEMA_VERSION: u32 = 1;

/// Injectable commit boundaries for fault-injection tests (P2-7 / #84).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breakpoint {
    /// After the intent row is committed, before external IDs are bound.
    AfterIntentPersist,
    /// Before the external-ID binding transaction commits.
    BeforeExternalBind,
    /// Before an event append commits.
    BeforeEventAppend,
}

type FaultHook = Arc<dyn Fn(Breakpoint) -> std::result::Result<(), StoreError> + Send + Sync>;

/// Errors surfaced by the store. `LeaseHeld` and `SummaryTooLarge` are explicit
/// domain failures, not silent fallbacks.
#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    /// Another owner holds the run lease (exclusive lease contention).
    LeaseHeld {
        run_id: String,
        owner: String,
    },
    /// A summary exceeds [`MAX_SUMMARY_BYTES`]; store a file reference instead.
    SummaryTooLarge {
        field: String,
        len: usize,
        cap: usize,
    },
    /// A fault-injection hook aborted the operation at a commit boundary.
    FaultInjected(Breakpoint),
    /// The referenced row does not exist.
    NotFound(String),
    Json(serde_json::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sqlite(e) => write!(f, "sqlite: {e}"),
            StoreError::LeaseHeld { run_id, owner } => {
                write!(f, "run '{run_id}' lease already held by '{owner}'")
            }
            StoreError::SummaryTooLarge { field, len, cap } => {
                write!(f, "summary '{field}' is {len} bytes, exceeds cap {cap}")
            }
            StoreError::FaultInjected(bp) => write!(f, "fault injected at {bp:?}"),
            StoreError::NotFound(what) => write!(f, "not found: {what}"),
            StoreError::Json(e) => write!(f, "json: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sqlite(e)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Json(e)
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Fixed identity captured when a run is created. These fields are immutable
/// afterwards: they pin exactly what was executed for audit/replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpec {
    pub id: String,
    pub workflow_digest: String,
    pub bundle_digest: String,
    pub input_summary: String,
    pub repo_identity: String,
    pub start_head: String,
    pub config_summary: String,
}

/// Lifecycle state of an attempt. `IntentPersisted` means the intent is durable
/// but no external ID has been bound yet (recoverable after a crash).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptState {
    IntentPersisted,
    Bound,
    Running,
    Succeeded,
    Failed,
    Uncertain,
}

impl AttemptState {
    fn as_str(self) -> &'static str {
        match self {
            AttemptState::IntentPersisted => "intent_persisted",
            AttemptState::Bound => "bound",
            AttemptState::Running => "running",
            AttemptState::Succeeded => "succeeded",
            AttemptState::Failed => "failed",
            AttemptState::Uncertain => "uncertain",
        }
    }
    fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "intent_persisted" => AttemptState::IntentPersisted,
            "bound" => AttemptState::Bound,
            "running" => AttemptState::Running,
            "succeeded" => AttemptState::Succeeded,
            "failed" => AttemptState::Failed,
            "uncertain" => AttemptState::Uncertain,
            _ => return None,
        })
    }
}

/// An attempt row as read back from the database.
#[derive(Debug, Clone)]
pub struct Attempt {
    pub id: i64,
    pub run_id: String,
    pub node_id: String,
    pub attempt_no: u32,
    pub intent_json: String,
    pub state: AttemptState,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub process_id: Option<i64>,
}

/// A replayable event with its monotonic sequence number.
#[derive(Debug, Clone)]
pub struct Event {
    pub seq: i64,
    pub run_id: String,
    pub kind: String,
    pub summary: String,
    pub created_at: i64,
}

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS runs (
    id              TEXT PRIMARY KEY,
    workflow_digest TEXT NOT NULL,
    bundle_digest   TEXT NOT NULL,
    input_summary   TEXT NOT NULL,
    repo_identity   TEXT NOT NULL,
    start_head      TEXT NOT NULL,
    config_summary  TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'pending',
    created_at      INTEGER NOT NULL,
    lease_owner     TEXT,
    lease_expires   INTEGER
);
CREATE TABLE IF NOT EXISTS nodes (
    run_id     TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    node_id    TEXT NOT NULL,
    state      TEXT NOT NULL DEFAULT 'pending',
    depends_on TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (run_id, node_id)
);
CREATE TABLE IF NOT EXISTS attempts (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id      TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    node_id     TEXT NOT NULL,
    attempt_no  INTEGER NOT NULL,
    intent_json TEXT NOT NULL,
    state       TEXT NOT NULL,
    thread_id   TEXT,
    turn_id     TEXT,
    process_id  INTEGER,
    created_at  INTEGER NOT NULL,
    UNIQUE (run_id, node_id, attempt_no)
);
CREATE TABLE IF NOT EXISTS events (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id     TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,
    summary    TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS artifacts (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id     TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    node_id    TEXT,
    path       TEXT NOT NULL,
    media_type TEXT NOT NULL,
    digest     TEXT,
    size_bytes INTEGER,
    schema_ref TEXT,
    summary    TEXT
);
CREATE INDEX IF NOT EXISTS idx_events_run_seq ON events(run_id, seq);
CREATE INDEX IF NOT EXISTS idx_attempts_run ON attempts(run_id, node_id);
"#;

/// The independent state store. Cheap to clone? No — wrap in your own mutex if
/// shared across threads; each [`Store`] owns one connection.
pub struct Store {
    conn: Connection,
    fault_hook: Mutex<Option<FaultHook>>,
}

impl Store {
    /// Open (creating if absent) the database at `path` and migrate to the
    /// current schema. WAL + foreign keys + busy timeout are enabled so two
    /// processes can contend for leases without corrupting state.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    /// Open an in-memory database (tests).
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        let store = Store {
            conn,
            fault_hook: Mutex::new(None),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let version: u32 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < 1 {
            self.conn.execute_batch(SCHEMA_V1)?;
            self.conn
                .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(())
    }

    /// The migrated schema version currently recorded in the database.
    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    /// Install a fault-injection hook invoked at each [`Breakpoint`]. Used by
    /// P2-7 (#84) to abort operations at real commit boundaries.
    pub fn set_fault_hook<F>(&self, hook: F)
    where
        F: Fn(Breakpoint) -> Result<()> + Send + Sync + 'static,
    {
        *self.fault_hook.lock().unwrap() = Some(Arc::new(hook));
    }

    /// Remove any fault-injection hook.
    pub fn clear_fault_hook(&self) {
        *self.fault_hook.lock().unwrap() = None;
    }

    fn hit(&self, bp: Breakpoint) -> Result<()> {
        let hook = self.fault_hook.lock().unwrap().clone();
        if let Some(h) = hook {
            h(bp)?;
        }
        Ok(())
    }

    fn bounded(field: &str, s: &str) -> Result<()> {
        if s.len() > MAX_SUMMARY_BYTES {
            return Err(StoreError::SummaryTooLarge {
                field: field.to_string(),
                len: s.len(),
                cap: MAX_SUMMARY_BYTES,
            });
        }
        Ok(())
    }

    /// Create a run with its fixed identity. Rejects oversize summaries and
    /// duplicate run ids explicitly.
    pub fn create_run(&self, spec: &RunSpec, now: i64) -> Result<()> {
        Self::bounded("input_summary", &spec.input_summary)?;
        Self::bounded("config_summary", &spec.config_summary)?;
        self.conn.execute(
            "INSERT INTO runs (id, workflow_digest, bundle_digest, input_summary,
                repo_identity, start_head, config_summary, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8)",
            params![
                spec.id,
                spec.workflow_digest,
                spec.bundle_digest,
                spec.input_summary,
                spec.repo_identity,
                spec.start_head,
                spec.config_summary,
                now
            ],
        )?;
        Ok(())
    }

    /// Seed node rows from a contracts workflow (id + depends_on). Idempotent.
    pub fn seed_nodes(&self, run_id: &str, wf: &superagent_contracts::Workflow) -> Result<()> {
        for node in &wf.nodes {
            let deps = serde_json::to_string(&node.depends_on)?;
            self.conn.execute(
                "INSERT INTO nodes (run_id, node_id, state, depends_on)
                 VALUES (?1, ?2, 'pending', ?3)
                 ON CONFLICT(run_id, node_id) DO NOTHING",
                params![run_id, node.id, deps],
            )?;
        }
        Ok(())
    }

    /// Acquire (or renew, for the same owner) the exclusive run lease.
    /// Returns `LeaseHeld` if a different owner holds an unexpired lease.
    pub fn acquire_lease(&self, run_id: &str, owner: &str, now: i64, ttl_secs: i64) -> Result<()> {
        let expires = now + ttl_secs;
        let changed = self.conn.execute(
            "UPDATE runs SET lease_owner = ?1, lease_expires = ?2
             WHERE id = ?3 AND (lease_owner IS NULL OR lease_expires <= ?4 OR lease_owner = ?1)",
            params![owner, expires, run_id, now],
        )?;
        if changed == 0 {
            let holder: Option<Option<String>> = self
                .conn
                .query_row(
                    "SELECT lease_owner FROM runs WHERE id = ?1",
                    params![run_id],
                    |r| r.get::<_, Option<String>>(0),
                )
                .optional()?;
            return match holder {
                None => Err(StoreError::NotFound(format!("run {run_id}"))),
                Some(h) => Err(StoreError::LeaseHeld {
                    run_id: run_id.to_string(),
                    owner: h.unwrap_or_else(|| "<unknown>".to_string()),
                }),
            };
        }
        Ok(())
    }

    /// Release the lease held by `owner`. A no-op if a different owner holds it.
    pub fn release_lease(&self, run_id: &str, owner: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET lease_owner = NULL, lease_expires = NULL
             WHERE id = ?1 AND lease_owner = ?2",
            params![run_id, owner],
        )?;
        Ok(())
    }

    /// Persist an intent in its own committed transaction. The attempt is left
    /// in `IntentPersisted` with no external bindings; binding happens later via
    /// [`Store::bind_external_ids`]. A crash in between is recoverable.
    pub fn persist_intent(
        &self,
        run_id: &str,
        node_id: &str,
        attempt_no: u32,
        intent_json: &str,
        now: i64,
    ) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO attempts (run_id, node_id, attempt_no, intent_json, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                run_id,
                node_id,
                attempt_no,
                intent_json,
                AttemptState::IntentPersisted.as_str(),
                now
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        self.hit(Breakpoint::AfterIntentPersist)?;
        Ok(id)
    }

    /// Bind external thread/turn/process IDs to a previously persisted intent,
    /// in a separate transaction. Moves the attempt to `Bound`.
    pub fn bind_external_ids(
        &self,
        attempt_id: i64,
        thread_id: Option<&str>,
        turn_id: Option<&str>,
        process_id: Option<i64>,
    ) -> Result<()> {
        self.hit(Breakpoint::BeforeExternalBind)?;
        let tx = self.conn.unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE attempts SET thread_id = ?1, turn_id = ?2, process_id = ?3, state = ?4
             WHERE id = ?5",
            params![
                thread_id,
                turn_id,
                process_id,
                AttemptState::Bound.as_str(),
                attempt_id
            ],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound(format!("attempt {attempt_id}")));
        }
        tx.commit()?;
        Ok(())
    }

    /// Persist an intent then bind external IDs, hitting the injectable
    /// breakpoints between the two commits. If a hook aborts after persist, the
    /// intent remains durable and unbound (recoverable), and the error is
    /// surfaced — never swallowed.
    #[allow(clippy::too_many_arguments)] // intent identity + optional external bindings
    pub fn send_intent(
        &self,
        run_id: &str,
        node_id: &str,
        attempt_no: u32,
        intent_json: &str,
        now: i64,
        thread_id: Option<&str>,
        turn_id: Option<&str>,
        process_id: Option<i64>,
    ) -> Result<i64> {
        let id = self.persist_intent(run_id, node_id, attempt_no, intent_json, now)?;
        self.bind_external_ids(id, thread_id, turn_id, process_id)?;
        Ok(id)
    }

    /// Update an attempt's lifecycle state explicitly.
    pub fn set_attempt_state(&self, attempt_id: i64, state: AttemptState) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE attempts SET state = ?1 WHERE id = ?2",
            params![state.as_str(), attempt_id],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound(format!("attempt {attempt_id}")));
        }
        Ok(())
    }

    /// Read an attempt by id.
    pub fn get_attempt(&self, attempt_id: i64) -> Result<Attempt> {
        self.conn
            .query_row(
                "SELECT id, run_id, node_id, attempt_no, intent_json, state, thread_id, turn_id, process_id
                 FROM attempts WHERE id = ?1",
                params![attempt_id],
                |r| {
                    let state_str: String = r.get(5)?;
                    Ok(Attempt {
                        id: r.get(0)?,
                        run_id: r.get(1)?,
                        node_id: r.get(2)?,
                        attempt_no: r.get::<_, i64>(3)? as u32,
                        intent_json: r.get(4)?,
                        state: AttemptState::from_str(&state_str).unwrap_or(AttemptState::Uncertain),
                        thread_id: r.get(6)?,
                        turn_id: r.get(7)?,
                        process_id: r.get(8)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("attempt {attempt_id}")))
    }

    /// List unbound intents for a run (recovery scan after a crash).
    pub fn unbound_intents(&self, run_id: &str) -> Result<Vec<Attempt>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, node_id, attempt_no, intent_json, state, thread_id, turn_id, process_id
             FROM attempts WHERE run_id = ?1 AND state = 'intent_persisted' ORDER BY id",
        )?;
        let rows = stmt.query_map(params![run_id], |r| {
            let state_str: String = r.get(5)?;
            Ok(Attempt {
                id: r.get(0)?,
                run_id: r.get(1)?,
                node_id: r.get(2)?,
                attempt_no: r.get::<_, i64>(3)? as u32,
                intent_json: r.get(4)?,
                state: AttemptState::from_str(&state_str).unwrap_or(AttemptState::Uncertain),
                thread_id: r.get(6)?,
                turn_id: r.get(7)?,
                process_id: r.get(8)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Append a bounded event; returns its monotonic sequence number for replay.
    pub fn append_event(&self, run_id: &str, kind: &str, summary: &str, now: i64) -> Result<i64> {
        Self::bounded("event.summary", summary)?;
        self.hit(Breakpoint::BeforeEventAppend)?;
        self.conn.execute(
            "INSERT INTO events (run_id, kind, summary, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![run_id, kind, summary, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Replay events for a run with sequence strictly greater than `after_seq`.
    pub fn events_after(&self, run_id: &str, after_seq: i64) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, run_id, kind, summary, created_at FROM events
             WHERE run_id = ?1 AND seq > ?2 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![run_id, after_seq], |r| {
            Ok(Event {
                seq: r.get(0)?,
                run_id: r.get(1)?,
                kind: r.get(2)?,
                summary: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Record an artifact reference (path + digest + bounded summary). The blob
    /// itself stays in a file; only the index/reference is stored here.
    #[allow(clippy::too_many_arguments)] // artifact index fields are independently optional
    pub fn record_artifact(
        &self,
        run_id: &str,
        node_id: Option<&str>,
        path: &str,
        media_type: &str,
        digest: Option<&str>,
        size_bytes: Option<i64>,
        schema_ref: Option<&str>,
        summary: Option<&str>,
    ) -> Result<i64> {
        if let Some(s) = summary {
            Self::bounded("artifact.summary", s)?;
        }
        self.conn.execute(
            "INSERT INTO artifacts (run_id, node_id, path, media_type, digest, size_bytes, schema_ref, summary)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![run_id, node_id, path, media_type, digest, size_bytes, schema_ref, summary],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Update a run's lifecycle state.
    pub fn set_run_state(&self, run_id: &str, state: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE runs SET state = ?1 WHERE id = ?2",
            params![state, run_id],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound(format!("run {run_id}")));
        }
        Ok(())
    }

    /// Read back a run's fixed identity and current state.
    pub fn get_run(&self, run_id: &str) -> Result<(RunSpec, String)> {
        self.conn
            .query_row(
                "SELECT id, workflow_digest, bundle_digest, input_summary, repo_identity,
                        start_head, config_summary, state FROM runs WHERE id = ?1",
                params![run_id],
                |r| {
                    Ok((
                        RunSpec {
                            id: r.get(0)?,
                            workflow_digest: r.get(1)?,
                            bundle_digest: r.get(2)?,
                            input_summary: r.get(3)?,
                            repo_identity: r.get(4)?,
                            start_head: r.get(5)?,
                            config_summary: r.get(6)?,
                        },
                        r.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("run {run_id}")))
    }
}
