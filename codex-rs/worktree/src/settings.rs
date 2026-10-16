//! Resolves managed-worktree settings shared with Desktop.
//! CLI allocation uses the same root without enabling automatic cleanup.

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

const WORKTREE_ROOT: &str = "git-worktree-root";
const AUTO_CLEANUP: &str = "worktree-auto-cleanup-enabled";
const KEEP_COUNT: &str = "worktree-keep-count";

pub const DEFAULT_WORKTREE_KEEP_COUNT: usize = 15;

/// Effective host-local settings for managed worktrees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeSettings {
    pub root: PathBuf,
    pub auto_cleanup_enabled: bool,
    pub keep_count: usize,
}

impl WorktreeSettings {
    /// Shares Desktop's allocation root while leaving CLI cleanup disabled.
    pub fn for_cli(codex_home: &Path, app_config: Option<&HashMap<String, Value>>) -> Result<Self> {
        Ok(Self {
            auto_cleanup_enabled: false,
            ..Self::from_app_config(codex_home, app_config)?
        })
    }

    /// Resolves existing `[app]` values without introducing another config format.
    pub fn from_app_config(
        codex_home: &Path,
        app_config: Option<&HashMap<String, Value>>,
    ) -> Result<Self> {
        let root = match app_config.and_then(|settings| settings.get(WORKTREE_ROOT)) {
            None | Some(Value::Null) => codex_home.join("worktrees"),
            Some(value) => {
                let configured = value
                    .as_str()
                    .context("app.git-worktree-root must be a string")?
                    .trim();
                if configured.is_empty() {
                    codex_home.join("worktrees")
                } else {
                    let path = PathBuf::from(configured);
                    if !path.is_absolute() {
                        bail!("app.git-worktree-root must be an absolute path");
                    }
                    path
                }
            }
        };

        let auto_cleanup_enabled = app_config
            .and_then(|settings| settings.get(AUTO_CLEANUP))
            .map(|value| {
                value
                    .as_bool()
                    .context("app.worktree-auto-cleanup-enabled must be a boolean")
            })
            .transpose()?
            .unwrap_or(true);

        let keep_count = app_config
            .and_then(|settings| settings.get(KEEP_COUNT))
            .map(|value| {
                let count = value
                    .as_u64()
                    .context("app.worktree-keep-count must be a positive integer")?;
                if count == 0 {
                    bail!("app.worktree-keep-count must be a positive integer");
                }
                usize::try_from(count).context("app.worktree-keep-count is too large")
            })
            .transpose()?
            .unwrap_or(DEFAULT_WORKTREE_KEEP_COUNT);

        Ok(Self {
            root: dunce::simplified(&root).to_path_buf(),
            auto_cleanup_enabled,
            keep_count,
        })
    }
}
