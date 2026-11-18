//! Durable evidence (P2-5 / #82).
//!
//! Every gate pass and every agent-turn result is backed by a concrete artifact
//! written to disk AND registered in the #80 [`Store`] with a sha256 digest and
//! byte size. A [`GateOutcome`] is only `is_proven()` when it carries a non-empty
//! `evidence_ref`; this module produces those references. Evidence write failures
//! surface as errors — they never fabricate a pass.

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use sha2::{Digest, Sha256};
use superagent_storage::Store;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    out.iter().map(|b| format!("{b:02x}")).collect()
}

/// Records durable evidence artifacts for a run.
#[derive(Clone)]
pub struct EvidenceRecorder {
    store: Rc<Store>,
    run_id: String,
    dir: PathBuf,
}

impl EvidenceRecorder {
    pub fn new(store: Rc<Store>, run_id: impl Into<String>, dir: impl Into<PathBuf>) -> Self {
        EvidenceRecorder {
            store,
            run_id: run_id.into(),
            dir: dir.into(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Write `bytes` to `<dir>/<name>`, register the artifact in the store with
    /// digest + size, and return the absolute path used as `evidence_ref`.
    pub fn record(
        &self,
        node_id: Option<&str>,
        name: &str,
        bytes: &[u8],
        media_type: &str,
        schema_ref: Option<&str>,
        summary: Option<&str>,
    ) -> Result<String, String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("mkdir evidence dir: {e}"))?;
        let path = self.dir.join(name);
        fs::write(&path, bytes).map_err(|e| format!("write evidence {}: {e}", path.display()))?;
        let abs = path
            .canonicalize()
            .map_err(|e| format!("canonicalize evidence: {e}"))?;
        let digest = sha256_hex(bytes);
        self.store
            .record_artifact(
                &self.run_id,
                node_id,
                &abs.to_string_lossy(),
                media_type,
                Some(&digest),
                Some(bytes.len() as i64),
                schema_ref,
                summary,
            )
            .map_err(|e| format!("record artifact: {e}"))?;
        Ok(abs.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_stable_and_hex() {
        let d = sha256_hex(b"hello");
        assert_eq!(d.len(), 64);
        assert_eq!(d, sha256_hex(b"hello"));
        assert_ne!(d, sha256_hex(b"world"));
    }

    #[test]
    fn record_writes_file_and_registers_artifact() {
        let store = Rc::new(Store::open_in_memory().unwrap());
        let tmp = tempfile::tempdir().unwrap();
        let rec = EvidenceRecorder::new(store.clone(), "run-1", tmp.path().join("ev"));
        let spec = superagent_storage::RunSpec {
            id: "run-1".into(),
            workflow_digest: "wd".into(),
            bundle_digest: "bd".into(),
            input_summary: "in".into(),
            repo_identity: "repo".into(),
            start_head: "head".into(),
            config_summary: "cfg".into(),
        };
        store.create_run(&spec, 0).unwrap();
        let ref_path = rec
            .record(
                Some("n1"),
                "gate.json",
                b"{\"ok\":true}",
                "application/json",
                None,
                Some("gate"),
            )
            .unwrap();
        assert!(Path::new(&ref_path).exists());
        assert_eq!(fs::read(&ref_path).unwrap(), b"{\"ok\":true}");
    }
}
