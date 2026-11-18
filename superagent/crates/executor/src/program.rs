//! Real child-process execution for verification gates (P2-5 / #82).
//!
//! The gate program is spawned as an argv array (NO shell) via
//! `std::process::Command`; the ACTUAL process exit status is captured from
//! `ExitStatus` (code + signal). A model's self-reported exit code is never
//! consulted here — this module has no model input at all.
//!
//! Bounds actually enforced: cwd confinement, wall timeout (kill + reap),
//! per-stream output cap (memory-bounded capture; totals still metered), and
//! a bounded rotating run log (the #79 [`RunLog`] mechanism).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use superagent_adapter::runlog::RunLog;

/// What to run and the bounds to enforce.
#[derive(Debug, Clone)]
pub struct ProgramSpec {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub timeout: Duration,
    /// Per-stream captured-output cap (bytes). Totals are metered regardless.
    pub output_cap_bytes: usize,
}

/// The REAL outcome of one child process run.
#[derive(Debug, Clone)]
pub struct ProgramRun {
    /// Actual exit code from `ExitStatus::code()`; None on signal death or
    /// spawn failure. Never model-supplied.
    pub exit_code: Option<i32>,
    /// Actual terminating signal (unix), if any.
    pub signal: Option<i32>,
    pub timed_out: bool,
    /// Bounded captured stdout (lossy UTF-8, at most `output_cap_bytes`).
    pub stdout: String,
    /// Bounded captured stderr.
    pub stderr: String,
    pub stdout_total_bytes: usize,
    pub stderr_total_bytes: usize,
    pub output_truncated: bool,
    pub elapsed: Duration,
    /// Spawn failure reason (program missing, argv empty, ...). When set, no
    /// process ran and `exit_code` is None — never fabricated as 0.
    pub spawn_error: Option<String>,
}

fn capture_bounded(
    mut pipe: impl Read + Send + 'static,
    cap: usize,
) -> std::thread::JoinHandle<(Vec<u8>, usize)> {
    std::thread::spawn(move || {
        let mut kept: Vec<u8> = Vec::new();
        let mut total = 0usize;
        let mut buf = [0u8; 8192];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    total += n;
                    if kept.len() < cap {
                        let take = (cap - kept.len()).min(n);
                        kept.extend_from_slice(&buf[..take]);
                    }
                }
            }
        }
        (kept, total)
    })
}

fn bounded_text(bytes: &[u8], cap: usize) -> String {
    let mut s = String::from_utf8_lossy(bytes).into_owned();
    if s.len() > cap {
        s.truncate(cap);
    }
    s
}

/// Run the program and capture its REAL exit status. On timeout the child is
/// killed and reaped; `timed_out` is set and the exit status reflects the
/// kill (never a fabricated success code).
pub fn run_program(spec: &ProgramSpec) -> ProgramRun {
    let start = Instant::now();
    let mut run = ProgramRun {
        exit_code: None,
        signal: None,
        timed_out: false,
        stdout: String::new(),
        stderr: String::new(),
        stdout_total_bytes: 0,
        stderr_total_bytes: 0,
        output_truncated: false,
        elapsed: Duration::ZERO,
        spawn_error: None,
    };
    if spec.argv.is_empty() {
        run.spawn_error = Some("argv must be non-empty".into());
        run.elapsed = start.elapsed();
        return run;
    }
    let mut cmd = Command::new(&spec.argv[0]);
    cmd.args(&spec.argv[1..])
        .current_dir(&spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            run.spawn_error = Some(format!("spawn '{}': {e}", spec.argv[0]));
            run.elapsed = start.elapsed();
            return run;
        }
    };
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let out_handle = capture_bounded(stdout, spec.output_cap_bytes);
    let err_handle = capture_bounded(stderr, spec.output_cap_bytes);

    let deadline = start + spec.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    run.timed_out = true;
                    let _ = child.kill();
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                run.spawn_error = Some(format!("wait '{}': {e}", spec.argv[0]));
                break None;
            }
        }
    };
    if let Some(status) = status {
        run.exit_code = status.code();
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            run.signal = status.signal();
        }
    }
    if let Ok((bytes, total)) = out_handle.join() {
        run.stdout_total_bytes = total;
        run.stdout = bounded_text(&bytes, spec.output_cap_bytes);
    }
    if let Ok((bytes, total)) = err_handle.join() {
        run.stderr_total_bytes = total;
        run.stderr = bounded_text(&bytes, spec.output_cap_bytes);
    }
    run.output_truncated =
        run.stdout_total_bytes > run.stdout.len() || run.stderr_total_bytes > run.stderr.len();
    run.elapsed = start.elapsed();
    run
}

/// Write a program run into a bounded rotating run log (the #79 mechanism).
/// Returns (rotations, retained_bytes). Log write failures surface as errors —
/// they never fabricate evidence.
pub fn log_program_run(
    log_path: &Path,
    log_limit_bytes: usize,
    argv: &[String],
    run: &ProgramRun,
) -> Result<(usize, usize), superagent_adapter::AdapterError> {
    let mut log = RunLog::open(log_path, log_limit_bytes)?;
    log.record("meta", &format!("argv={argv:?}"))?;
    for line in run.stdout.lines() {
        log.record("stdout", line)?;
    }
    for line in run.stderr.lines() {
        log.record("stderr", line)?;
    }
    log.record(
        "exit",
        &format!(
            "code={:?} signal={:?} timedOut={} truncated={} elapsedMs={}",
            run.exit_code,
            run.signal,
            run.timed_out,
            run.output_truncated,
            run.elapsed.as_millis()
        ),
    )?;
    Ok((log.rotations(), log.retained_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(script: &str, timeout_ms: u64, cap: usize) -> ProgramSpec {
        ProgramSpec {
            argv: vec!["python3".into(), "-c".into(), script.into()],
            cwd: std::env::temp_dir(),
            timeout: Duration::from_millis(timeout_ms),
            output_cap_bytes: cap,
        }
    }

    #[test]
    fn captures_real_nonzero_exit_code() {
        let run = run_program(&spec("import sys; sys.exit(3)", 10_000, 4096));
        assert_eq!(run.exit_code, Some(3));
        assert!(!run.timed_out);
        assert!(run.spawn_error.is_none());
    }

    #[test]
    fn captures_real_zero_exit_code_and_output() {
        let run = run_program(&spec("print('gate-ok')", 10_000, 4096));
        assert_eq!(run.exit_code, Some(0));
        assert!(run.stdout.contains("gate-ok"));
    }

    #[test]
    fn timeout_kills_and_reports_no_fabricated_code() {
        let run = run_program(&spec("import time; time.sleep(30)", 300, 4096));
        assert!(run.timed_out);
        assert_ne!(run.exit_code, Some(0));
        assert!(run.elapsed < Duration::from_secs(5));
    }

    #[test]
    fn output_cap_bounds_capture_but_meters_totals() {
        let run = run_program(&spec("print('x' * 100000)", 10_000, 4096));
        assert_eq!(run.exit_code, Some(0));
        assert!(run.stdout.len() <= 4096);
        assert!(run.stdout_total_bytes >= 100000);
        assert!(run.output_truncated);
    }

    #[test]
    fn spawn_failure_is_explicit_not_exit_zero() {
        let s = ProgramSpec {
            argv: vec!["/nonexistent/p2-5-program".into()],
            cwd: std::env::temp_dir(),
            timeout: Duration::from_secs(1),
            output_cap_bytes: 4096,
        };
        let run = run_program(&s);
        assert!(run.spawn_error.is_some());
        assert_eq!(run.exit_code, None);
    }

    #[test]
    fn empty_argv_is_explicit_error() {
        let s = ProgramSpec {
            argv: vec![],
            cwd: std::env::temp_dir(),
            timeout: Duration::from_secs(1),
            output_cap_bytes: 4096,
        };
        assert!(run_program(&s).spawn_error.is_some());
    }
}
