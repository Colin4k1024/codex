//! SuperAgent agent capability-pack marketplace (P5-4 / #98).
//!
//! Agent packs become first-class marketplace entries: a manifest + content
//! digest + publisher signature, one-click install / upgrade / rollback, and
//! run bindings pinned to the digest captured at bind time (an upgrade never
//! changes a running task's pack). Curated and third-party sources are
//! distinguished and the license list travels with the pack. Extraction is
//! safe: path-traversal and symlink entries are rejected before any write
//! (all-or-nothing). Model-free: pure crypto (hmac/sha2) + filesystem policy.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Curated,
    ThirdParty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Regular,
    Symlink,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackFile {
    pub path: String,
    pub content: String,
    pub kind: FileKind,
    #[serde(default)]
    pub symlink_target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackManifest {
    pub name: String,
    pub version: String,
    pub source: Source,
    pub publisher_key_id: String,
    pub licenses: Vec<String>,
    pub files: Vec<PackFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedPack {
    pub manifest: PackManifest,
    pub digest: String,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPack {
    pub name: String,
    pub version: String,
    pub digest: String,
    pub source: Source,
    pub licenses: Vec<String>,
    pub install_dir: String,
}

#[derive(Debug)]
pub enum MarketError {
    UnknownKey(String),
    DigestMismatch,
    BadSignature,
    NotInstalled(String),
    NothingToRollback(String),
    PathTraversal(String),
    SymlinkRejected(String),
    Io(std::io::Error),
}

impl std::fmt::Display for MarketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarketError::UnknownKey(s) => write!(f, "unknown publisher key: {s}"),
            MarketError::DigestMismatch => write!(f, "manifest digest mismatch"),
            MarketError::BadSignature => write!(f, "invalid pack signature"),
            MarketError::NotInstalled(s) => write!(f, "pack not installed: {s}"),
            MarketError::NothingToRollback(s) => write!(f, "no prior version to roll back: {s}"),
            MarketError::PathTraversal(s) => write!(f, "path traversal rejected: {s}"),
            MarketError::SymlinkRejected(s) => write!(f, "symlink entry rejected: {s}"),
            MarketError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for MarketError {}

impl From<std::io::Error> for MarketError {
    fn from(e: std::io::Error) -> Self {
        MarketError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, MarketError>;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn manifest_digest(manifest: &PackManifest) -> String {
    let bytes = serde_json::to_vec(manifest).expect("serialize manifest");
    let mut h = Sha256::new();
    h.update(&bytes);
    hex(&h.finalize())
}

fn sign_bytes(secret: &str, digest: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(digest.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Validate a relative pack path stays inside the install dir: no absolute
/// path, no parent-dir (`..`) component, no empty/root-only path.
fn safe_rel_path(path: &str) -> Result<()> {
    let p = Path::new(path);
    if p.is_absolute() || path.is_empty() {
        return Err(MarketError::PathTraversal(path.into()));
    }
    for comp in p.components() {
        match comp {
            Component::Normal(_) => {}
            _ => return Err(MarketError::PathTraversal(path.into())),
        }
    }
    Ok(())
}

pub struct Marketplace {
    trusted_keys: BTreeMap<String, String>,
    installed: BTreeMap<String, InstalledPack>,
    history: BTreeMap<String, Vec<InstalledPack>>,
    run_bindings: BTreeMap<String, String>,
    audit: Vec<Value>,
}

impl Marketplace {
    pub fn new() -> Self {
        Self {
            trusted_keys: BTreeMap::new(),
            installed: BTreeMap::new(),
            history: BTreeMap::new(),
            run_bindings: BTreeMap::new(),
            audit: Vec::new(),
        }
    }

    pub fn register_key(&mut self, key_id: &str, secret: &str) {
        self.trusted_keys.insert(key_id.into(), secret.into());
    }

    pub fn sign_pack(&self, manifest: PackManifest) -> Result<SignedPack> {
        let secret = self
            .trusted_keys
            .get(&manifest.publisher_key_id)
            .ok_or_else(|| MarketError::UnknownKey(manifest.publisher_key_id.clone()))?;
        let digest = manifest_digest(&manifest);
        let signature = sign_bytes(secret, &digest);
        Ok(SignedPack {
            manifest,
            digest,
            signature,
        })
    }

    pub fn verify_pack(&self, pack: &SignedPack) -> Result<()> {
        let recomputed = manifest_digest(&pack.manifest);
        if recomputed != pack.digest {
            return Err(MarketError::DigestMismatch);
        }
        let secret = self
            .trusted_keys
            .get(&pack.manifest.publisher_key_id)
            .ok_or_else(|| MarketError::UnknownKey(pack.manifest.publisher_key_id.clone()))?;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
        mac.update(pack.digest.as_bytes());
        mac.verify_slice(&pack.signature)
            .map_err(|_| MarketError::BadSignature)?;
        Ok(())
    }

    /// Safe, all-or-nothing extraction: validate every entry first (reject
    /// symlinks and path traversal), then write regular files only.
    fn extract(files: &[PackFile], dir: &Path) -> Result<()> {
        for f in files {
            if f.kind == FileKind::Symlink {
                return Err(MarketError::SymlinkRejected(f.path.clone()));
            }
            safe_rel_path(&f.path)?;
        }
        for f in files {
            let dest = dir.join(&f.path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, &f.content)?;
        }
        Ok(())
    }

    fn install_internal(
        &mut self,
        pack: &SignedPack,
        target_root: &Path,
        upgrade: bool,
    ) -> Result<InstalledPack> {
        self.verify_pack(pack)?;
        let m = &pack.manifest;
        if upgrade && !self.installed.contains_key(&m.name) {
            return Err(MarketError::NotInstalled(m.name.clone()));
        }
        let dir: PathBuf = target_root.join(&m.name).join(&m.version);
        Self::extract(&m.files, &dir)?;
        let rec = InstalledPack {
            name: m.name.clone(),
            version: m.version.clone(),
            digest: pack.digest.clone(),
            source: m.source,
            licenses: m.licenses.clone(),
            install_dir: dir.to_string_lossy().into_owned(),
        };
        if let Some(prev) = self.installed.insert(m.name.clone(), rec.clone()) {
            self.history.entry(m.name.clone()).or_default().push(prev);
        }
        self.audit.push(json!({
            "event": if upgrade { "upgrade" } else { "install" },
            "name": rec.name, "version": rec.version, "digest": rec.digest,
            "source": rec.source, "licenses": rec.licenses,
        }));
        Ok(rec)
    }

    pub fn install(&mut self, pack: &SignedPack, target_root: &Path) -> Result<InstalledPack> {
        self.install_internal(pack, target_root, false)
    }

    pub fn upgrade(&mut self, pack: &SignedPack, target_root: &Path) -> Result<InstalledPack> {
        self.install_internal(pack, target_root, true)
    }

    pub fn rollback(&mut self, name: &str) -> Result<InstalledPack> {
        let prev = self
            .history
            .get_mut(name)
            .and_then(|h| h.pop())
            .ok_or_else(|| MarketError::NothingToRollback(name.into()))?;
        self.installed.insert(name.into(), prev.clone());
        self.audit.push(json!({
            "event": "rollback", "name": prev.name, "version": prev.version, "digest": prev.digest,
        }));
        Ok(prev)
    }

    pub fn current(&self, name: &str) -> Option<&InstalledPack> {
        self.installed.get(name)
    }

    /// Pin a run to the digest of the currently installed pack. Later upgrades
    /// do not change this binding.
    pub fn bind_run(&mut self, run_id: &str, name: &str) -> Result<String> {
        let rec = self
            .installed
            .get(name)
            .ok_or_else(|| MarketError::NotInstalled(name.into()))?;
        self.run_bindings.insert(run_id.into(), rec.digest.clone());
        self.audit.push(
            json!({ "event": "bind_run", "runId": run_id, "name": name, "digest": rec.digest }),
        );
        Ok(rec.digest.clone())
    }

    pub fn resolve_run_binding(&self, run_id: &str) -> Option<&String> {
        self.run_bindings.get(run_id)
    }

    pub fn audit(&self) -> &[Value] {
        &self.audit
    }
}

impl Default for Marketplace {
    fn default() -> Self {
        Self::new()
    }
}
