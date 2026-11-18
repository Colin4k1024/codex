//! Per-run transcript log with rotation. Mirrors `spike_client._log_frame`:
//! when retained bytes + next entry would exceed the limit, the oldest
//! segment is dropped (file truncated/reopened) and `rotations` increments,
//! so retained bytes never exceed the limit. A single entry larger than the
//! whole limit is replaced by a bounded truncation notice (never fabricates
//! content). Same mechanism at 100 MiB production and scaled test thresholds.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::AdapterError;

#[derive(Debug)]
pub struct RunLog {
    path: PathBuf,
    file: File,
    limit: usize,
    retained: usize,
    rotations: usize,
}

impl RunLog {
    pub fn open(path: &Path, limit: usize) -> Result<Self, AdapterError> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| AdapterError::Io(format!("run log open: {e}")))?;
        let retained = file
            .metadata()
            .map_err(|e| AdapterError::Io(format!("run log stat: {e}")))?
            .len() as usize;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            limit,
            retained,
            rotations: 0,
        })
    }

    pub fn record(&mut self, direction: &str, line: &str) -> Result<(), AdapterError> {
        let entry = format!("{direction} {line}\n");
        let size = entry.len();
        if self.retained + size > self.limit {
            // Rotate: drop the oldest segment, start a fresh one.
            self.file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&self.path)
                .map_err(|e| AdapterError::Io(format!("run log rotate: {e}")))?;
            self.retained = 0;
            self.rotations += 1;
        }
        let written = if size > self.limit {
            let notice = format!(
                "{direction} <entry truncated: {size} bytes exceeds run-log limit {}>\n",
                self.limit
            );
            self.file
                .write_all(notice.as_bytes())
                .map_err(|e| AdapterError::Io(format!("run log write: {e}")))?;
            notice.len()
        } else {
            self.file
                .write_all(entry.as_bytes())
                .map_err(|e| AdapterError::Io(format!("run log write: {e}")))?;
            size
        };
        self.file
            .flush()
            .map_err(|e| AdapterError::Io(format!("run log flush: {e}")))?;
        self.retained += written;
        Ok(())
    }

    pub fn rotations(&self) -> usize {
        self.rotations
    }

    pub fn retained_bytes(&self) -> usize {
        self.retained
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_drops_oldest_and_bounds_retained() {
        let dir = std::env::temp_dir().join(format!("sa-runlog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("run.log");
        let _ = std::fs::remove_file(&path);
        let limit = 4096;
        let mut log = RunLog::open(&path, limit).unwrap();
        let chunk = "x".repeat(1000);
        for i in 0..20 {
            log.record("recv", &format!("{chunk}#{i}")).unwrap();
            assert!(log.retained_bytes() <= limit);
        }
        assert!(log.rotations() >= 1);
        assert!(std::fs::metadata(&path).unwrap().len() as usize <= limit);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn entry_larger_than_limit_is_truncated_notice() {
        let dir = std::env::temp_dir().join(format!("sa-runlog-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("run.log");
        let _ = std::fs::remove_file(&path);
        let mut log = RunLog::open(&path, 128).unwrap();
        log.record("recv", &"y".repeat(5000)).unwrap();
        assert!(log.retained_bytes() <= 128);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("entry truncated"));
        let _ = std::fs::remove_file(&path);
    }
}
