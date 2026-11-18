//! `superagent` product CLI entry point. Thin dispatch over the library; the
//! deterministic guarantees live in the crates, not here. Stable exit codes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{json, Value};
use superagent_adapter::{Adapter, AdapterConfig};
use superagent_cli::cancel::{cancel_exec_session, cancel_turn_session};
use superagent_cli::plan::{ctx_from_workflow, dry_run_plan, to_json};
use superagent_cli::report::{build_report, report_input_from_store, Change};
use superagent_cli::resume::{expected_identity, resume_run, ResumeLease};
use superagent_cli::run::{execute_run, exit_code_for_report, RunOptions};
use superagent_cli::status::status_json;
use superagent_cli::{exit, CliError, Result, CLI_VERSION};
use superagent_contracts::{Workflow, SCHEMA_VERSION};
use superagent_executor::{sha256_hex, EngineSpec};
use superagent_orchestrator::{NodeState, Policy};
use superagent_recovery::{PriorNode, ResumeContext};
use superagent_storage::{RunSpec, Store};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn positional(args: &[String]) -> Option<String> {
    args.iter().find(|a| !a.starts_with("--")).cloned()
}

fn load_workflow(path: &str) -> Result<Workflow> {
    let text = std::fs::read_to_string(path)?;
    let wf: Workflow = serde_json::from_str(&text)?;
    Ok(wf)
}

fn open_store(args: &[String]) -> Result<Store> {
    match flag(args, "--store") {
        Some(p) => Ok(Store::open(p)?),
        None => Err(CliError::Usage("--store <path> required".into())),
    }
}

fn cmd_run(args: &[String]) -> Result<i32> {
    let wf_path = positional(args).ok_or_else(|| CliError::Usage("run <workflow.json>".into()))?;
    let wf = load_workflow(&wf_path)?;
    let ctx = ctx_from_workflow(&wf);
    if has(args, "--dry-run") {
        let plan = dry_run_plan(&wf, &ctx)?;
        println!("{}", serde_json::to_string_pretty(&to_json(&plan))?);
        return Ok(exit::OK);
    }
    let task = flag(args, "--task").or_else(|| {
        flag(args, "--task-file").map(|p| std::fs::read_to_string(p).unwrap_or_default())
    });
    let engine = flag(args, "--engine")
        .or_else(|| std::env::var("SUPERAGENT_ENGINE").ok())
        .unwrap_or_else(|| "superagent-engine".into());
    let codex_home = flag(args, "--codex-home")
        .or_else(|| std::env::var("SUPERAGENT_CODEX_HOME").ok())
        .map(PathBuf::from);
    let workspace = flag(args, "--workspace")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let run_id = flag(args, "--run-id").unwrap_or_else(|| format!("run-{}", std::process::id()));
    let evidence_dir = flag(args, "--evidence-dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join(".superagent").join("evidence"));
    let wf_digest = sha256_hex(&serde_json::to_vec(&wf)?);
    let spec = RunSpec {
        id: run_id.clone(),
        workflow_digest: wf_digest,
        bundle_digest: flag(args, "--bundle-digest").unwrap_or_default(),
        input_summary: sha256_hex(task.clone().unwrap_or_default().as_bytes()),
        repo_identity: flag(args, "--repo-identity").unwrap_or_default(),
        start_head: flag(args, "--start-head").unwrap_or_default(),
        config_summary: flag(args, "--config-summary").unwrap_or_default(),
    };
    let store = Rc::new(match flag(args, "--store") {
        Some(p) => Store::open(p)?,
        None => Store::open_in_memory()?,
    });
    let opts = RunOptions {
        workflow: wf,
        ctx,
        policy: Policy::default(),
        engine: EngineSpec {
            argv: vec![
                engine,
                "app-server".into(),
                "--listen".into(),
                "stdio://".into(),
            ],
            codex_home,
            cwd: Some(workspace.clone()),
            environment: None,
            run_deadline: Duration::from_secs(600),
            max_artifact_bytes: 20 * 1024 * 1024,
        },
        workspace,
        evidence_dir,
        run_id: run_id.clone(),
        spec,
        now: 0,
        schema_registry: BTreeMap::new(),
        gate_timeout: Duration::from_secs(120),
        output_cap_bytes: 1 << 20,
        task_override: task,
        cancel: superagent_cli::CancelToken::new(),
    };
    let report = execute_run(store, opts)?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({ "runId": run_id, "state": format!("{:?}", report.state), "tokensUsed": report.tokens_used })
        )?
    );
    Ok(exit_code_for_report(&report))
}

fn cmd_status(args: &[String]) -> Result<i32> {
    let run_id = positional(args).ok_or_else(|| CliError::Usage("status <run-id>".into()))?;
    let store = open_store(args)?;
    let v = status_json(&store, &run_id)?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(exit::OK)
}

fn parse_state(s: &str) -> NodeState {
    match s {
        "Succeeded" => NodeState::Succeeded,
        "Failed" => NodeState::Failed,
        "Blocked" => NodeState::Blocked,
        "WaitingInput" => NodeState::WaitingInput,
        "Uncertain" => NodeState::Uncertain,
        "Cancelling" => NodeState::Cancelling,
        "Running" => NodeState::Running,
        "Verifying" => NodeState::Verifying,
        "Ready" => NodeState::Ready,
        _ => NodeState::Pending,
    }
}

fn cmd_resume(args: &[String]) -> Result<i32> {
    let run_id = positional(args).ok_or_else(|| CliError::Usage("resume <run-id>".into()))?;
    let store = open_store(args)?;
    let wf_path = flag(args, "--workflow").ok_or_else(|| CliError::Usage("--workflow".into()))?;
    let wf = load_workflow(&wf_path)?;
    let task = flag(args, "--task").or_else(|| {
        flag(args, "--task-file").map(|p| std::fs::read_to_string(p).unwrap_or_default())
    });
    let expected = expected_identity(
        &wf,
        task.as_deref(),
        &flag(args, "--bundle-digest").unwrap_or_default(),
        &flag(args, "--repo-identity").unwrap_or_default(),
        &flag(args, "--start-head").unwrap_or_default(),
        &flag(args, "--config-summary").unwrap_or_default(),
    )?;
    let prior_map: BTreeMap<String, String> = flag(args, "--prior")
        .map(|p| {
            serde_json::from_str(&std::fs::read_to_string(p).unwrap_or_default())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let artifacts: BTreeMap<String, String> = flag(args, "--artifacts")
        .map(|p| {
            serde_json::from_str(&std::fs::read_to_string(p).unwrap_or_default())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let non_idempotent: BTreeSet<String> = flag(args, "--non-idempotent")
        .map(|s| {
            s.split(',')
                .filter(|x| !x.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let prior = prior_map
        .into_iter()
        .map(|(id, st)| {
            (
                id.clone(),
                PriorNode {
                    state: parse_state(&st),
                    artifact_digest: artifacts.get(&id).cloned(),
                },
            )
        })
        .collect();
    let ctx = ResumeContext {
        prior,
        current_artifacts: artifacts,
        non_idempotent,
    };
    let owner = flag(args, "--owner").unwrap_or_else(|| "cli".into());
    let lease = ResumeLease {
        owner: &owner,
        now: 0,
        ttl_secs: 3600,
    };
    let v = resume_run(&store, &run_id, &lease, &expected, &wf, &ctx)?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(exit::OK)
}

fn cmd_cancel(args: &[String]) -> Result<i32> {
    let run_id = positional(args).ok_or_else(|| CliError::Usage("cancel <run-id>".into()))?;
    let store = open_store(args)?;
    if let (Some(engine), Some(pid)) = (flag(args, "--engine"), flag(args, "--process-id")) {
        let cfg = AdapterConfig {
            argv: vec![
                engine,
                "app-server".into(),
                "--listen".into(),
                "stdio://".into(),
            ],
            run_deadline: Duration::from_secs(30),
            ..Default::default()
        };
        let adapter = Adapter::spawn_default(cfg)?;
        adapter.handshake()?;
        let ev = cancel_exec_session(&adapter, &pid, &store, &run_id, 0)?;
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({ "runId": run_id, "cancelled": true, "cleanup": { "terminated": ev.terminated, "engineClosed": ev.engine_closed } })
            )?
        );
        return Ok(exit::CANCELLED);
    }
    if let (Some(engine), Some(tid)) = (flag(args, "--engine"), flag(args, "--thread-id")) {
        let cfg = AdapterConfig {
            argv: vec![
                engine,
                "app-server".into(),
                "--listen".into(),
                "stdio://".into(),
            ],
            run_deadline: Duration::from_secs(30),
            ..Default::default()
        };
        let adapter = Adapter::spawn_default(cfg)?;
        adapter.handshake()?;
        let ev = cancel_turn_session(&adapter, &tid, &store, &run_id, 0)?;
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({ "runId": run_id, "cancelled": true, "cleanup": { "interrupted": ev.interrupted, "engineClosed": ev.engine_closed } })
            )?
        );
        return Ok(exit::CANCELLED);
    }
    // No live engine handle: record the cancellation intent durably.
    store.set_run_state(&run_id, "cancelled")?;
    store.append_event(
        &run_id,
        "run_cancelled",
        "{\"engineClosed\":false,\"reason\":\"no live engine handle\"}",
        0,
    )?;
    println!("{}", json!({ "runId": run_id, "cancelled": true }));
    Ok(exit::CANCELLED)
}

fn changes_from_file(path: Option<String>) -> Vec<Change> {
    let Some(p) = path else { return vec![] };
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(p).unwrap_or_default()).unwrap_or(json!([]));
    v.as_array()
        .map(|arr| {
            arr.iter()
                .map(|c| Change {
                    path: c
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    digest: c.get("digest").and_then(Value::as_str).map(String::from),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn cmd_report(args: &[String]) -> Result<i32> {
    let run_id = positional(args).ok_or_else(|| CliError::Usage("report <run-id>".into()))?;
    let store = open_store(args)?;
    let events = store.events_after(&run_id, 0)?;
    let evidence: Vec<String> = events
        .iter()
        .map(|e| format!("event:{}:{}", e.seq, e.kind))
        .collect();
    let inp = report_input_from_store(
        &store,
        &run_id,
        CLI_VERSION,
        changes_from_file(flag(args, "--baseline-changes")),
        changes_from_file(flag(args, "--run-changes")),
        evidence,
    )?;
    println!("{}", serde_json::to_string_pretty(&build_report(&inp))?);
    Ok(exit::OK)
}

fn dispatch(args: &[String]) -> Result<i32> {
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let rest = &args[1..];
    match cmd {
        "run" => cmd_run(rest),
        "status" => cmd_status(rest),
        "resume" => cmd_resume(rest),
        "cancel" => cmd_cancel(rest),
        "report" => cmd_report(rest),
        "version" | "--version" => {
            println!("superagent {CLI_VERSION} (workflow schema {SCHEMA_VERSION})");
            Ok(exit::OK)
        }
        _ => Err(CliError::Usage(
            "commands: run|status|resume|cancel|report|version".into(),
        )),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(e.exit_code() as u8)
        }
    }
}
