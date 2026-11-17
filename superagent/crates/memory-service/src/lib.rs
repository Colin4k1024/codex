//! SuperAgent memory-service (P3-1 / #87).
//!
//! Product-side SQLite FTS5 hybrid index over rollout / memories / board posts.
//! Chinese tokenization uses the built-in FTS5 `trigram` tokenizer (substring
//! matching works for CJK without an external segmenter). Incremental sync is
//! content-hash based: unchanged sources are skipped, changed sources reindexed.
//! Embeddings vector retrieval is an optional posterior — the interface is
//! reserved here but no model or vector engine is executed in this crate.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_RESULTS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Rollout,
    Memory,
    Board,
}

impl SourceKind {
    fn as_str(self) -> &'static str {
        match self {
            SourceKind::Rollout => "rollout",
            SourceKind::Memory => "memory",
            SourceKind::Board => "board",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub kind: String,
    pub source_id: String,
    pub snippet: String,
    pub score: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOutcome {
    Inserted,
    Reindexed,
    Unchanged,
}

#[derive(Debug)]
pub enum MemoryError {
    Sqlite(rusqlite::Error),
    EmbeddingsUnsupported,
    EmptyQuery,
}

impl std::fmt::Display for MemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryError::Sqlite(e) => write!(f, "sqlite: {e}"),
            MemoryError::EmbeddingsUnsupported => {
                write!(f, "embeddings vector retrieval is reserved (optional posterior), not implemented")
            }
            MemoryError::EmptyQuery => write!(f, "search query must be non-empty"),
        }
    }
}

impl std::error::Error for MemoryError {}

impl From<rusqlite::Error> for MemoryError {
    fn from(e: rusqlite::Error) -> Self {
        MemoryError::Sqlite(e)
    }
}

pub type Result<T> = std::result::Result<T, MemoryError>;

/// Reserved embeddings interface (optional posterior). No implementation in
/// this crate spawns a model or vector engine; the stub fails explicitly.
pub trait VectorIndex {
    fn upsert_embedding(&mut self, kind: SourceKind, source_id: &str, vector: &[f32]) -> Result<()>;
    fn search_vector(&self, query: &[f32], limit: usize) -> Result<Vec<Hit>>;
}

pub struct ReservedVectorIndex;

impl VectorIndex for ReservedVectorIndex {
    fn upsert_embedding(&mut self, _kind: SourceKind, _source_id: &str, _vector: &[f32]) -> Result<()> {
        Err(MemoryError::EmbeddingsUnsupported)
    }
    fn search_vector(&self, _query: &[f32], _limit: usize) -> Result<Vec<Hit>> {
        Err(MemoryError::EmbeddingsUnsupported)
    }
}

const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS sources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    source_id TEXT NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}',
    indexed_at INTEGER NOT NULL,
    UNIQUE(kind, source_id)
);
CREATE VIRTUAL TABLE IF NOT EXISTS mem_fts USING fts5(
    kind, source_id, content, tokenize='trigram'
);
CREATE INDEX IF NOT EXISTS idx_sources_kind ON sources(kind);
";

pub struct MemoryService {
    conn: Connection,
}

fn content_hash(content: &str) -> String {
    let mut h = DefaultHasher::new();
    content.hash(&mut h);
    format!("{:016x}", h.finish())
}

impl MemoryService {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    pub fn from_connection(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let svc = Self { conn };
        svc.migrate()?;
        Ok(svc)
    }

    pub fn migrate(&self) -> Result<()> {
        let v: u32 = self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if v < SCHEMA_VERSION {
            self.conn.execute_batch(SCHEMA_V1)?;
            self.conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<u32> {
        Ok(self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    /// Incremental sync: insert new sources, reindex changed content, skip
    /// unchanged content (hash match). FTS rowid mirrors sources.id.
    pub fn index(&self, kind: SourceKind, source_id: &str, content: &str, metadata: &serde_json::Value) -> Result<SyncOutcome> {
        let hash = content_hash(content);
        let meta = metadata.to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let existing: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, content_hash FROM sources WHERE kind=?1 AND source_id=?2",
                rusqlite::params![kind.as_str(), source_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let tx = self.conn.unchecked_transaction()?;
        let outcome = match existing {
            Some((_id, old_hash)) if old_hash == hash => SyncOutcome::Unchanged,
            Some((id, _)) => {
                tx.execute(
                    "UPDATE sources SET content=?1, content_hash=?2, metadata=?3, indexed_at=?4 WHERE id=?5",
                    rusqlite::params![content, hash, meta, now, id],
                )?;
                tx.execute("DELETE FROM mem_fts WHERE rowid=?1", rusqlite::params![id])?;
                tx.execute(
                    "INSERT INTO mem_fts(rowid, kind, source_id, content) VALUES(?1,?2,?3,?4)",
                    rusqlite::params![id, kind.as_str(), source_id, content],
                )?;
                SyncOutcome::Reindexed
            }
            None => {
                tx.execute(
                    "INSERT INTO sources(kind, source_id, content, content_hash, metadata, indexed_at) VALUES(?1,?2,?3,?4,?5,?6)",
                    rusqlite::params![kind.as_str(), source_id, content, hash, meta, now],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO mem_fts(rowid, kind, source_id, content) VALUES(?1,?2,?3,?4)",
                    rusqlite::params![id, kind.as_str(), source_id, content],
                )?;
                SyncOutcome::Inserted
            }
        };
        tx.commit()?;
        Ok(outcome)
    }

    pub fn remove(&self, kind: SourceKind, source_id: &str) -> Result<bool> {
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM sources WHERE kind=?1 AND source_id=?2",
                rusqlite::params![kind.as_str(), source_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = existing else { return Ok(false) };
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM mem_fts WHERE rowid=?1", rusqlite::params![id])?;
        tx.execute("DELETE FROM sources WHERE id=?1", rusqlite::params![id])?;
        tx.commit()?;
        Ok(true)
    }

    /// Bounded hybrid search via FTS5 MATCH + bm25 ranking. Results cite their
    /// source (kind + source_id) and a snippet. `limit` is clamped to MAX_RESULTS.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>> {
        if query.trim().is_empty() {
            return Err(MemoryError::EmptyQuery);
        }
        let limit = limit.min(MAX_RESULTS).max(1);
        let match_expr = format!("\"{}\"", query.replace('"', "\"\""));
        let mut stmt = self.conn.prepare(
            "SELECT s.kind, s.source_id, snippet(mem_fts, 2, '[', ']', '…', 12), bm25(mem_fts) \
             FROM mem_fts JOIN sources s ON s.id = mem_fts.rowid \
             WHERE mem_fts MATCH ?1 ORDER BY bm25(mem_fts) LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![match_expr, limit as i64], |r| {
            Ok(Hit { kind: r.get(0)?, source_id: r.get(1)?, snippet: r.get(2)?, score: r.get(3)? })
        })?;
        let mut hits = Vec::new();
        for row in rows {
            hits.push(row?);
        }
        Ok(hits)
    }

    pub fn count(&self, kind: SourceKind) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sources WHERE kind=?1",
            rusqlite::params![kind.as_str()],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }
}

trait OptionalExt<T> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for std::result::Result<T, rusqlite::Error> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}