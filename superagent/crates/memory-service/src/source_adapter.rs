//! Real-file source adapter for the memory-service index (#87 runtime path).
//!
//! Connects the FTS5 index to the genuine product write paths: rollout session
//! files (`*.jsonl`), memory notes (`*.md`/`*.txt`) and board posts
//! (`*.json`/`*.jsonl`). A [`SourceAdapter::sync`] pass indexes new and changed
//! files (content-hash gated by [`crate::MemoryService::index`]) and removes
//! sources whose files disappeared. It is model-free and process-free: it only
//! reads files already on disk and never spawns an engine, model or vector
//! backend, so it can run inline on the product write path without blocking on
//! inference.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{MemoryService, Result, SourceKind, SyncOutcome};

/// Directories holding each source kind. `None` skips that kind.
#[derive(Debug, Clone, Default)]
pub struct SourceRoots {
    pub rollouts: Option<PathBuf>,
    pub memories: Option<PathBuf>,
    pub board: Option<PathBuf>,
}

/// Per-pass tally of incremental sync outcomes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub inserted: usize,
    pub reindexed: usize,
    pub unchanged: usize,
    pub removed: usize,
}

pub struct SourceAdapter;

impl SourceAdapter {
    /// Index every current file under `roots`, then drop indexed sources whose
    /// files no longer exist. Idempotent: an unchanged second pass reports only
    /// `unchanged`.
    pub fn sync(svc: &MemoryService, roots: &SourceRoots) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        let kinds = [
            (
                SourceKind::Rollout,
                roots.rollouts.as_deref(),
                &["jsonl"][..],
            ),
            (
                SourceKind::Memory,
                roots.memories.as_deref(),
                &["md", "txt"][..],
            ),
            (
                SourceKind::Board,
                roots.board.as_deref(),
                &["json", "jsonl"][..],
            ),
        ];
        for (kind, dir, exts) in kinds {
            let Some(dir) = dir else { continue };
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for path in collect_files(dir, exts)? {
                let source_id = relative_id(dir, &path);
                let raw = std::fs::read_to_string(&path)?;
                let text = extract_text(kind, &raw);
                let metadata = serde_json::json!({
                    "path": path.to_string_lossy(),
                    "bytes": raw.len(),
                });
                match svc.index(kind, &source_id, &text, &metadata)? {
                    SyncOutcome::Inserted => report.inserted += 1,
                    SyncOutcome::Reindexed => report.reindexed += 1,
                    SyncOutcome::Unchanged => report.unchanged += 1,
                }
                seen.insert(source_id);
            }
            for id in svc.source_ids(kind)? {
                if !seen.contains(&id) && svc.remove(kind, &id)? {
                    report.removed += 1;
                }
            }
        }
        Ok(report)
    }
}

fn relative_id(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn collect_files(dir: &Path, exts: &[&str]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if exts.iter().any(|want| want.eq_ignore_ascii_case(ext)) {
                    out.push(path);
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Pull searchable prose out of a source file. Rollouts and board posts are
/// JSON/JSONL: harvest message/text/content fields. Memories are plain prose.
/// Falls back to the raw bytes when no structured text is found so a source is
/// never silently dropped from the index.
fn extract_text(kind: SourceKind, raw: &str) -> String {
    match kind {
        SourceKind::Memory => raw.to_string(),
        SourceKind::Rollout | SourceKind::Board => {
            let mut parts: Vec<String> = Vec::new();
            for line in raw.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    harvest(&v, &mut parts);
                }
            }
            if parts.is_empty() {
                if let Ok(v) = serde_json::from_str::<Value>(raw) {
                    harvest(&v, &mut parts);
                }
            }
            if parts.is_empty() {
                raw.to_string()
            } else {
                parts.join("\n")
            }
        }
    }
}

fn harvest(v: &Value, parts: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for key in ["text", "message", "content", "body", "title", "summary"] {
                if let Some(field) = map.get(key) {
                    harvest(field, parts);
                }
            }
            if let Some(payload) = map.get("payload") {
                harvest(payload, parts);
            }
        }
        Value::Array(items) => {
            for item in items {
                harvest(item, parts);
            }
        }
        Value::String(s) if !s.trim().is_empty() => {
            parts.push(s.clone());
        }
        _ => {}
    }
}
