//! P2-2 (#79) acceptance tests: model-free fault injection against a fake
//! stdio engine (`fake_engine.py`). No real engine, no model, no network.
//!
//! Six acceptance fault cases:
//!   slow consumer        -> slow_consumer_retains_all_events_and_terminal
//!   oversize line        -> oversize_frame_faults_with_explicit_error
//!   big log (scaled)     -> big_log_rotation_scaled_threshold
//!   unknown event        -> unknown_event_and_malformed_frame_keep_pump_alive
//!   unknown server req   -> typed_approval_never_auto_approves (-32601)
//!   engine crash mid-turn-> engine_crash_mid_turn_explicit_terminal_and_reconcile
//! Plus: resource limits, per-type timeouts, typed approvals, deferred
//! approval pump, stderr ring, exit cleanup, no-silent-terminal-drop.
//!
//! Scaled thresholds (64 KiB frame / 256 KiB log / 4 KiB artifact) are injected
//! via AdapterConfig — the SAME mechanism enforced at the documented
//! production defaults (16 MiB / 100 MiB / 20 MiB), mirroring p0_6b.py.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use superagent_adapter::{
    classify, reconcile_thread, Adapter, AdapterConfig, AdapterError, AdapterEvent,
    ApprovalHandler, Decision, DecliningApprovalHandler, Disposition, EnvironmentConfig,
    ServerRequest, TimeoutPolicy, UiCoalescer, UiEvent, DEFAULT_MAX_ARTIFACT_BYTES,
    DEFAULT_MAX_FRAME_BYTES, DEFAULT_MAX_LOG_BYTES, DEFAULT_STDERR_RING_LINES,
};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn engine_path() -> String {
    format!("{}/tests/fake_engine.py", env!("CARGO_MANIFEST_DIR"))
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "sa-p2-2-{tag}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn cfg(scenario: &str) -> AdapterConfig {
    AdapterConfig {
        argv: vec![
            "python3".into(),
            engine_path(),
            "--scenario".into(),
            scenario.into(),
        ],
        run_deadline: Duration::from_secs(30),
        close_grace: Duration::from_secs(2),
        ..Default::default()
    }
}

fn spawn(scenario: &str) -> Adapter {
    Adapter::spawn_default(cfg(scenario)).expect("spawn fake engine")
}

fn poll_until(mut f: impl FnMut() -> bool, timeout: Duration) -> bool {
    let dl = Instant::now() + timeout;
    while Instant::now() < dl {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    f()
}

fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn note_method(n: &Value) -> &str {
    n.get("method").and_then(Value::as_str).unwrap_or("")
}

// ---------------------------------------------------------------- task 1/2

#[test]
fn spawn_argv_env_and_pipe_wiring() {
    let dir = tmp("wiring");
    let home = dir.join("codex-home");
    let mut c = cfg("echo");
    c.codex_home = Some(home.clone());
    let a = Adapter::spawn_default(c.clone()).unwrap();
    // argv array used verbatim (no shell); fake engine reports its own argv.
    assert_eq!(a.spawn_argv(), &c.argv[..]);
    let env = a.request("test/env", json!({}), None).unwrap();
    assert_eq!(env["codexHome"], json!(home.to_str().unwrap()));
    assert_eq!(env["argv"], json!(c.argv[1..]));
    a.handshake().unwrap();
    let pid = a.pid();
    a.close();
    assert_eq!(a.stats().exit.unwrap().code, Some(0));
    assert!(
        poll_until(|| !alive(pid), Duration::from_secs(5)),
        "child must be reaped"
    );
}

#[test]
fn request_correlation_three_way_pump() {
    let a = spawn("echo");
    a.handshake().unwrap();
    let ts = a.request("thread/start", json!({}), None).unwrap();
    let tid = ts["thread"]["id"].as_str().unwrap().to_string();
    // Model-task terminal wait uses the run deadline (task 4).
    let result = a
        .request("turn/start", json!({"threadId": tid}), None)
        .unwrap();
    assert_eq!(result["turn"]["status"], "completed");
    let started = a
        .wait_notification(|n| note_method(n) == "turn/started", a.deadline_remaining())
        .unwrap();
    assert_eq!(started["params"]["threadId"], json!(tid));
    let completed = a
        .wait_notification(
            |n| note_method(n) == "turn/completed",
            a.deadline_remaining(),
        )
        .unwrap();
    assert_eq!(completed["params"]["threadId"], json!(tid));
    let deltas: Vec<Value> = a
        .drain_events()
        .into_iter()
        .filter_map(|ev| match ev {
            AdapterEvent::Notification(n) if note_method(&n) == "item/agentMessage/delta" => {
                Some(n)
            }
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 3);
    // Third way: server request -> typed decision -> engine observes reply.
    let sr = a
        .request(
            "test/emitServerRequest",
            json!({"method": "item/commandExecution/requestApproval", "requestId": "srv-corr"}),
            None,
        )
        .unwrap();
    assert_eq!(sr["reply"]["result"]["decision"], "decline");
    assert_eq!(sr["reply"]["id"], "srv-corr");
    // Unknown client method propagates the engine's JSON-RPC error.
    match a.request("no/suchMethod", json!({}), None) {
        Err(AdapterError::Rpc { code, .. }) => assert_eq!(code, -32601),
        other => panic!("expected Rpc -32601, got {other:?}"),
    }
    a.close();
}

// ------------------------------------- P5-1 (#95): environment wiring

#[test]
fn environment_add_registers_and_info_is_cached_without_leaking_token() {
    let dir = tmp("environment");
    let state_file = dir.join("state.json");
    let log_path = dir.join("engine.jsonl");
    let remote_cwd = dir.join("remote-ws");
    let remote_root = dir.join("remote-extra");
    let secret = "p5-1-super-secret-bearer-token";
    let mut c = cfg("echo");
    c.run_log_path = Some(log_path.clone());
    c.environment = Some(EnvironmentConfig {
        id: "env-1".into(),
        exec_server_url: "ws://127.0.0.1:6553".into(),
        auth_bearer_token: Some(secret.into()),
        cwd: Some(remote_cwd.clone()),
        workspace_roots: vec![remote_root.clone()],
    });
    // The config Debug view must redact the token.
    let debug = format!("{c:?}");
    assert!(
        !debug.contains(secret),
        "Debug leaked bearer token: {debug}"
    );
    assert!(debug.contains("<redacted>"));
    c.argv.push("--state-file".into());
    c.argv.push(state_file.to_string_lossy().into_owned());
    let a = Adapter::spawn_default(c).unwrap();
    a.handshake().unwrap();
    // Registration precedes any thread; info discovers deterministic caps.
    a.prepare_environment().unwrap();
    let info = a.environment_info().unwrap();
    assert_eq!(info["shell"]["name"], "zsh");
    assert_eq!(info["shell"]["path"], "/bin/zsh");
    assert_eq!(info["cwd"], "file:///tmp/fake-exec-env");
    // Cached read-only access: a second call emits no additional request.
    assert_eq!(a.cached_environment_info(), Some(info.clone()));
    assert_eq!(a.environment_info().unwrap(), info);
    // thread/start.environments entry comes from the protocol layer.
    let env_params = a.environment_thread_params(&dir).unwrap();
    assert_eq!(env_params["environmentId"], "env-1");
    assert_eq!(env_params["cwd"], remote_cwd.to_string_lossy().as_ref());
    assert_eq!(
        env_params["runtimeWorkspaceRoots"],
        json!([remote_root.to_string_lossy()])
    );
    a.request(
        "thread/start",
        json!({"cwd": dir, "environments": [env_params]}),
        None,
    )
    .unwrap();
    a.close();

    // The bearer token must never be persisted in the transcript log.
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(!log.contains(secret), "bearer token persisted in run log");
    assert!(log.contains("\"authBearerToken\":\"<redacted>\""));
    // Exactly one add and one info request (the repeat read was cached).
    let state: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    // Wire order: handshake -> register -> discover -> thread/start.
    assert_eq!(
        state["callLog"],
        json!([
            "initialize",
            "environment/add",
            "environment/info",
            "thread/start"
        ])
    );
    assert_eq!(state["environmentAdds"].as_array().unwrap().len(), 1);
    assert_eq!(state["environmentAdds"][0]["environmentId"], "env-1");
    assert_eq!(state["environmentAdds"][0]["hasBearerToken"], true);
    assert_eq!(state["environmentInfos"].as_array().unwrap().len(), 1);
    // Experimental API is opted into for environment methods.
    assert_eq!(state["initializeCapabilities"]["experimentalApi"], true);
    // The engine observed the environments entry on thread/start.
    assert_eq!(
        state["threadStarts"][0]["environments"][0]["environmentId"],
        "env-1"
    );
    assert_eq!(
        state["threadStarts"][0]["environments"][0]["runtimeWorkspaceRoots"],
        json!([remote_root.to_string_lossy()])
    );
}

#[test]
fn no_environment_keeps_existing_wire_shape() {
    let dir = tmp("no-environment");
    let state_file = dir.join("state.json");
    let log_path = dir.join("engine.jsonl");
    let mut c = cfg("echo");
    c.run_log_path = Some(log_path.clone());
    c.argv.push("--state-file".into());
    c.argv.push(state_file.to_string_lossy().into_owned());
    let a = Adapter::spawn_default(c).unwrap();
    a.handshake().unwrap();
    // No-op: no environment/add is emitted and no info can be discovered.
    a.prepare_environment().unwrap();
    assert_eq!(a.cached_environment_info(), None);
    assert!(a.environment_thread_params(&dir).is_none());
    match a.environment_info() {
        Err(AdapterError::InvalidConfig(_)) => {}
        other => panic!("environment_info without config must fail explicitly: {other:?}"),
    }
    a.request("thread/start", json!({"cwd": dir}), None)
        .unwrap();
    a.close();

    let state: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    // Only the pre-existing calls happened; prepare_environment emitted nothing.
    assert_eq!(state["callLog"], json!(["initialize", "thread/start"]));
    assert!(state["environmentAdds"].as_array().unwrap().is_empty());
    assert!(state["environmentInfos"].as_array().unwrap().is_empty());
    // thread/start carried exactly the pre-P5-1 params: no `environments` key.
    assert_eq!(
        state["threadStarts"][0],
        json!({"cwd": dir.to_string_lossy()})
    );
    assert!(state["threadStarts"][0].get("environments").is_none());
    // initialize capabilities unchanged (no experimental opt-in).
    assert_eq!(state["initializeCapabilities"], json!({}));
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(!log.contains("environment/add"));
    assert!(!log.contains("environment/info"));
    assert!(!log.contains("experimentalApi"));
}

#[test]
fn environment_config_validation_only_rejects_empty_fields() {
    assert!(AdapterConfig::default().environment.is_none());
    let valid_env = EnvironmentConfig {
        id: "env-1".into(),
        exec_server_url: "ws://127.0.0.1:6553".into(),
        auth_bearer_token: Some("token".into()),
        cwd: Some("/remote".into()),
        workspace_roots: vec!["/remote/extra".into()],
    };
    // Non-emptiness is the only client-side check (transport security is the
    // app-server's responsibility), so this config must pass validation.
    let mut ok = cfg("echo");
    ok.environment = Some(valid_env.clone());
    assert!(ok.validate().is_ok());

    for bad in [
        EnvironmentConfig {
            id: "  ".into(),
            ..valid_env.clone()
        },
        EnvironmentConfig {
            exec_server_url: "".into(),
            ..valid_env.clone()
        },
        EnvironmentConfig {
            auth_bearer_token: Some("".into()),
            ..valid_env.clone()
        },
    ] {
        let mut c = cfg("echo");
        c.environment = Some(bad);
        assert!(
            matches!(c.validate(), Err(AdapterError::InvalidConfig(_))),
            "empty environment field must be rejected"
        );
    }
}

#[test]
fn environment_add_failure_is_an_explicit_error() {
    let dir = tmp("environment-fail");
    let state_file = dir.join("state.json");
    let mut c = cfg("environment_fail");
    c.environment = Some(EnvironmentConfig {
        id: "env-down".into(),
        exec_server_url: "ws://127.0.0.1:1".into(),
        auth_bearer_token: None,
        cwd: None,
        workspace_roots: vec![],
    });
    c.argv.push("--state-file".into());
    c.argv.push(state_file.to_string_lossy().into_owned());
    let a = Adapter::spawn_default(c).unwrap();
    a.handshake().unwrap();
    match a.prepare_environment() {
        Err(AdapterError::Rpc { code, .. }) => assert_eq!(code, -32600),
        other => panic!("expected explicit RPC error, got {other:?}"),
    }
    // No thread was ever started (no silent local continuation).
    let state: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert_eq!(state["callLog"], json!(["initialize", "environment/add"]));
    assert!(state["threads"].as_object().is_none_or(|m| m.is_empty()));
    a.close();
}

// ------------------------------------------------------- task 2: approvals

struct DeferHandler;
impl ApprovalHandler for DeferHandler {
    fn handle(&self, r: &ServerRequest) -> Decision {
        if r.method == "item/commandExecution/requestApproval" {
            Decision::Defer
        } else {
            DecliningApprovalHandler.handle(r)
        }
    }
}

#[test]
fn typed_approval_never_auto_approves() {
    let a = spawn("echo");
    a.handshake().unwrap();
    let cases = [
        (
            "item/commandExecution/requestApproval",
            json!({"decision":"decline"}),
        ),
        ("execCommandApproval", json!({"decision":"deny"})),
        (
            "item/fileChange/requestApproval",
            json!({"decision":"decline"}),
        ),
        ("applyPatchApproval", json!({"decision":"deny"})),
        (
            "item/permissions/requestApproval",
            json!({"decision":"deny"}),
        ),
        ("mcpServer/elicitation/request", json!({"action":"decline"})),
        ("item/tool/requestUserInput", json!({"answers":{}})),
    ];
    for (i, (method, expected)) in cases.iter().enumerate() {
        let r = a
            .request(
                "test/emitServerRequest",
                json!({"method": method, "requestId": format!("srv-{i}")}),
                None,
            )
            .unwrap();
        assert_eq!(r["reply"]["result"], *expected, "{method}");
    }
    // Unknown server request: -32601, NEVER auto-approved.
    let r = a
        .request(
            "test/emitServerRequest",
            json!({"method": "evil/grantEverything", "requestId": "srv-evil"}),
            None,
        )
        .unwrap();
    assert_eq!(r["reply"]["error"]["code"], -32601);
    assert!(r["reply"]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("never auto-approved"));
    // Audit: no decision ever granted anything (error replies are the
    // -32601 block, whose message legitimately says "never auto-approved").
    for d in &a.stats().server_request_decisions {
        if let Some(result) = d.payload.get("result") {
            if let Some(dec) = result.get("decision").and_then(Value::as_str) {
                assert!(matches!(dec, "decline" | "deny"), "grant leaked: {dec}");
            }
            if let Some(action) = result.get("action").and_then(Value::as_str) {
                assert_eq!(action, "decline", "grant leaked: {action}");
            }
            if let Some(answers) = result.get("answers") {
                assert_eq!(answers, &json!({}), "user-input answers must be empty");
            }
        }
    }
    a.close();
}

#[test]
fn deferred_approval_pump_keeps_consuming() {
    let a = Adapter::spawn(cfg("approval_hold"), Arc::new(DeferHandler)).unwrap();
    a.handshake().unwrap();
    let tid_a = a.request("thread/start", json!({}), None).unwrap()["thread"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let rid_a = a
        .send_request(
            "turn/start",
            json!({"threadId": tid_a, "holdApproval": true}),
        )
        .unwrap();
    // waiting_input: approval wait bounded by the run deadline (task 4).
    let pending = a
        .wait_pending_server_request(a.deadline_remaining())
        .unwrap();
    assert_eq!(pending.method, "item/commandExecution/requestApproval");
    assert_eq!(a.stats().pending_server_requests, 1);
    // Concurrent turn on the SAME connection completes while approval held.
    let tid_b = a.request("thread/start", json!({}), None).unwrap()["thread"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    a.request("turn/start", json!({"threadId": tid_b}), None)
        .unwrap();
    let completed_b = a
        .wait_notification(
            |n| note_method(n) == "turn/completed" && n["params"]["threadId"] == json!(tid_b),
            a.deadline_remaining(),
        )
        .unwrap();
    assert_eq!(completed_b["params"]["threadId"], json!(tid_b));
    assert_eq!(a.stats().pending_server_requests, 1, "approval still held");
    // Explicit decline; engine observes the decision and completes turn A.
    a.respond_server_request(&pending.id, Decision::Result(json!({"decision":"decline"})))
        .unwrap();
    let completed_a = a
        .wait_notification(
            |n| note_method(n) == "turn/completed" && n["params"]["threadId"] == json!(tid_a),
            a.deadline_remaining(),
        )
        .unwrap();
    assert_eq!(completed_a["params"]["approvalDecision"], "decline");
    a.wait_response(rid_a, Duration::from_secs(10)).unwrap();
    assert_eq!(a.stats().pending_server_requests, 0);
    a.close();
}

// ------------------------------------------------------- task 4: timeouts

#[test]
fn per_type_timeout_defaults_match_p0_6_contract() {
    let p = TimeoutPolicy::default();
    let expect = [
        ("initialize", 30),
        ("environment/add", 60),
        ("environment/info", 60),
        ("thread/start", 60),
        ("thread/read", 60),
        ("thread/list", 60),
        ("thread/resume", 60),
        ("turn/start", 30),
        ("turn/interrupt", 30),
        ("command/exec", 90),
        ("command/exec/terminate", 30),
    ];
    for (m, s) in expect {
        assert_eq!(p.by_method[m], Duration::from_secs(s), "{m}");
    }
    assert_eq!(p.fallback, Duration::from_secs(60));
    assert_eq!(p.effective("unknown/method", None), Duration::from_secs(60));
    assert_eq!(
        p.effective("command/exec", Some(Duration::from_secs(2))),
        Duration::from_secs(2)
    );
}

#[test]
fn command_exec_timeout_then_terminate_reaps_child() {
    let dir = tmp("exec-timeout");
    let pid_file = dir.join("child.pid");
    let a = spawn("echo");
    a.handshake().unwrap();
    // command/exec waits for program exit; explicit 500ms timeout must fire
    // while `sleep 20` is still running (p0_6b.py parity, scaled).
    let rid = a
        .send_request(
            "command/exec",
            json!({"command": ["sleep", "20"], "processId": "p22-probe",
                   "pidFile": pid_file.to_str().unwrap()}),
        )
        .unwrap();
    let t0 = Instant::now();
    let err = a
        .wait_response(rid, Duration::from_millis(500))
        .unwrap_err();
    let elapsed = t0.elapsed();
    assert!(matches!(err, AdapterError::Timeout { .. }), "{err:?}");
    assert!(
        elapsed < Duration::from_secs(3),
        "timeout must be prompt: {elapsed:?}"
    );
    assert!(poll_until(|| pid_file.exists(), Duration::from_secs(5)));
    let child_pid: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        alive(child_pid),
        "program must still be running after client timeout"
    );
    // Recovery: terminate reaps the program.
    let term = a
        .request(
            "command/exec/terminate",
            json!({"processId": "p22-probe"}),
            None,
        )
        .unwrap();
    assert_eq!(term["terminated"], true);
    assert!(
        poll_until(|| !alive(child_pid), Duration::from_secs(5)),
        "child must be reaped"
    );
    // The late exec response reports the real (non-zero) exit — never success.
    let late = a.wait_response(rid, Duration::from_secs(5)).unwrap();
    assert_ne!(late["result"]["exitCode"], json!(0));
    a.close();
}

#[test]
fn global_deadline_clamps_long_request() {
    let mut c = cfg("echo");
    c.run_deadline = Duration::from_millis(400);
    let a = Adapter::spawn_default(c).unwrap();
    let t0 = Instant::now();
    let err = a
        .request(
            "test/hang",
            json!({"seconds": 5}),
            Some(Duration::from_secs(60)),
        )
        .unwrap_err();
    assert!(
        matches!(
            err,
            AdapterError::DeadlineExceeded { .. } | AdapterError::Timeout { .. }
        ),
        "{err:?}"
    );
    assert!(
        t0.elapsed() < Duration::from_secs(3),
        "deadline must clamp the 60s timeout"
    );
    a.close();
}

#[test]
fn notification_wait_clamped_by_run_deadline() {
    let mut c = cfg("echo");
    c.run_deadline = Duration::from_millis(500);
    let a = Adapter::spawn_default(c).unwrap();
    let t0 = Instant::now();
    let err = a
        .wait_notification(
            |n| note_method(n) == "never/happens",
            Duration::from_secs(60),
        )
        .unwrap_err();
    assert!(
        matches!(err, AdapterError::DeadlineExceeded { .. }),
        "{err:?}"
    );
    assert!(t0.elapsed() < Duration::from_secs(3));
    a.close();
}

#[test]
fn approval_waiting_input_bounded_by_deadline() {
    let mut c = cfg("echo");
    c.run_deadline = Duration::from_millis(500);
    let a = Adapter::spawn_default(c).unwrap();
    let t0 = Instant::now();
    let err = a
        .wait_pending_server_request(Duration::from_secs(60))
        .unwrap_err();
    assert!(
        matches!(err, AdapterError::DeadlineExceeded { .. }),
        "{err:?}"
    );
    assert!(t0.elapsed() < Duration::from_secs(3));
    a.close();
}

// ------------------------------------------------ task 3: resource limits

#[test]
fn oversize_frame_faults_with_explicit_error() {
    let mut c = cfg("echo");
    c.max_frame_bytes = 64 * 1024; // SCALED; production default 16 MiB.
    let a = Adapter::spawn_default(c).unwrap();
    a.handshake().unwrap();
    // 200 KB single frame > 64 KiB cap: explicit error, never fabricated success.
    let err = a
        .request("test/triggerOversize", json!({"bytes": 200_000}), None)
        .unwrap_err();
    assert!(
        matches!(err, AdapterError::FrameLimitExceeded { .. }),
        "{err:?}"
    );
    let stats = a.stats();
    assert!(!stats.frame_violations.is_empty());
    assert!(stats.max_frame_seen >= 200_000);
    assert!(stats.max_frame_seen > stats.frame_limit_bytes);
    assert!(a
        .drain_events()
        .iter()
        .any(|ev| matches!(ev, AdapterEvent::FrameViolation { .. })));
    // Transport stays faulted: further requests fail explicitly.
    let err2 = a.request("test/ping", json!({}), None).unwrap_err();
    assert!(
        matches!(err2, AdapterError::FrameLimitExceeded { .. }),
        "{err2:?}"
    );
    a.close();
}

#[test]
fn big_log_rotation_scaled_threshold() {
    // Production defaults are the documented trace-calibrated values.
    assert_eq!(DEFAULT_MAX_FRAME_BYTES, 16 * 1024 * 1024);
    assert_eq!(DEFAULT_MAX_LOG_BYTES, 100 * 1024 * 1024);
    assert_eq!(DEFAULT_MAX_ARTIFACT_BYTES, 20 * 1024 * 1024);
    assert_eq!(DEFAULT_STDERR_RING_LINES, 2000);

    let dir = tmp("biglog");
    let log_path = dir.join("run.log");
    let limit = 256 * 1024; // SCALED; production default 100 MiB.
    let mut c = cfg("echo");
    c.run_log_path = Some(log_path.clone());
    c.max_log_bytes = limit;
    let a = Adapter::spawn_default(c).unwrap();
    a.handshake().unwrap();
    // ~1.2 MiB of traffic through the SAME rotation mechanism.
    let r = a
        .request(
            "test/triggerBigLog",
            json!({"chunks": 40, "chunkBytes": 30000}),
            None,
        )
        .unwrap();
    assert_eq!(r["emitted"], 40);
    let stats = a.stats();
    assert!(
        stats.run_log_rotations >= 1,
        "rotations: {}",
        stats.run_log_rotations
    );
    assert!(stats.run_log_retained_bytes <= limit);
    let on_disk = std::fs::metadata(&log_path).unwrap().len() as usize;
    assert!(on_disk <= limit, "retained {on_disk} > limit {limit}");
    a.close();
}

#[test]
fn artifact_limit_explicit_error() {
    let dir = tmp("artifact");
    let mut c = cfg("echo");
    c.max_artifact_bytes = 4096; // SCALED; production default 20 MiB.
    let a = Adapter::spawn_default(c).unwrap();
    let big = dir.join("big.bin");
    let err = a.write_artifact(&big, &vec![7u8; 5000]).unwrap_err();
    assert!(
        matches!(
            err,
            AdapterError::ArtifactLimitExceeded {
                limit: 4096,
                observed: 5000
            }
        ),
        "{err:?}"
    );
    assert!(!big.exists(), "oversize artifact must not be written");
    let ok = dir.join("ok.bin");
    a.write_artifact(&ok, &vec![7u8; 3000]).unwrap();
    assert_eq!(std::fs::read(&ok).unwrap().len(), 3000);
    a.close();
}

// ------------------------------------------- fault cases: unknown / crash

#[test]
fn unknown_event_and_malformed_frame_keep_pump_alive() {
    let a = spawn("echo");
    a.handshake().unwrap();
    a.request("test/triggerUnknown", json!({}), None).unwrap();
    let events = a.drain_events();
    assert!(events.iter().any(|ev| matches!(ev,
        AdapterEvent::Notification(n) if note_method(n) == "totally/unknown")));
    assert!(events.iter().any(|ev| matches!(ev,
        AdapterEvent::Notification(n) if n.get("_raw").and_then(Value::as_str) == Some("this is not json"))));
    // Pump survives both; correlation still works.
    assert_eq!(
        a.request("test/ping", json!({}), None).unwrap()["pong"],
        true
    );
    a.close();
}

#[test]
fn slow_consumer_retains_all_events_and_terminal() {
    let a = spawn("slow");
    a.handshake().unwrap();
    let tid = a.request("thread/start", json!({}), None).unwrap()["thread"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    a.request(
        "turn/start",
        json!({"threadId": tid, "deltaCount": 5000}),
        None,
    )
    .unwrap();
    // Simulate a slow upper layer: nothing drained for 300ms while the engine
    // floods deltas. The queue must retain EVERYTHING (no silent drop).
    std::thread::sleep(Duration::from_millis(300));
    let events = a.drain_events();
    let deltas = events
        .iter()
        .filter(|ev| matches!(ev, AdapterEvent::Notification(n) if note_method(n) == "item/agentMessage/delta"))
        .count();
    assert_eq!(deltas, 5000, "overload must not drop deltas");
    assert!(
        events.iter().any(|ev| ev.is_terminal()),
        "terminal state must survive overload"
    );
    // Feed the coalescer (synthetic clock): deltas batch, terminal immediate.
    let mut ui = UiCoalescer::new(50).unwrap();
    let mut emitted = Vec::new();
    for ev in events {
        emitted.extend(ui.push(ev, 0));
    }
    emitted.extend(ui.flush());
    assert_eq!(ui.stats.dropped, 0);
    assert_eq!(ui.stats.deltas_seen, 5000);
    assert_eq!(ui.stats.deltas_emitted, 5000);
    assert!(ui.stats.terminals_emitted >= 1);
    assert!(emitted
        .iter()
        .any(|e| matches!(e, UiEvent::Immediate(ev) if ev.is_terminal())));
    a.close();
}

#[test]
fn engine_crash_mid_turn_explicit_terminal_and_reconcile() {
    let dir = tmp("crash");
    let state_file = dir.join("state.json");
    let mut c = cfg("crash");
    c.argv.push("--state-file".into());
    c.argv.push(state_file.to_str().unwrap().into());
    let a = Adapter::spawn_default(c).unwrap();
    a.handshake().unwrap();
    let tid = a.request("thread/start", json!({}), None).unwrap()["thread"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let rid = a
        .send_request("turn/start", json!({"threadId": tid}))
        .unwrap();
    a.wait_notification(
        |n| note_method(n) == "turn/started",
        Duration::from_secs(10),
    )
    .unwrap();
    // Engine dies mid-turn (exit 9). Collect the explicit terminal signals.
    let mut saw_exit = false;
    let mut saw_turn_terminal = false;
    let dl = Instant::now() + Duration::from_secs(10);
    while Instant::now() < dl {
        match a.wait_event(Duration::from_secs(2)) {
            Ok(AdapterEvent::EngineExited { code, .. }) => {
                assert_eq!(code, Some(9));
                saw_exit = true;
                break;
            }
            Ok(AdapterEvent::TurnTerminal {
                thread_id, status, ..
            }) => {
                assert_eq!(thread_id, tid);
                assert_eq!(status, "aborted");
                saw_turn_terminal = true;
            }
            Ok(_) => {}
            Err(AdapterError::EngineExited { .. }) => break,
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }
    assert!(saw_exit, "EngineExited terminal signal must be delivered");
    assert!(
        saw_turn_terminal,
        "in-flight turn must get a synthetic terminal (no silent drop)"
    );
    // Pending request fails explicitly — never hangs, never fabricates success.
    match a.wait_response(rid, Duration::from_secs(5)) {
        Err(AdapterError::EngineExited { code, .. }) => assert_eq!(code, Some(9)),
        other => panic!("expected EngineExited, got {other:?}"),
    }
    assert_eq!(a.stats().exit.unwrap().code, Some(9));
    a.close();

    // Reconciliation over FRESH connections (trace D parity, orphan-aware).
    let recon = |status: &str| {
        let mut c = cfg("reconcile");
        c.argv.push("--state-file".into());
        c.argv.push(state_file.to_str().unwrap().into());
        c.argv.push("--turn-status".into());
        c.argv.push(status.into());
        Adapter::spawn_default(c).unwrap()
    };
    let a2 = recon("inProgress");
    a2.handshake().unwrap();
    let r = reconcile_thread(&a2, &tid, false).unwrap();
    assert_eq!(r.last_turn_status.as_deref(), Some("inProgress"));
    assert!(
        matches!(r.disposition, Disposition::Uncertain { .. }),
        "no blind replay"
    );
    a2.close();

    let a3 = recon("failed");
    a3.handshake().unwrap();
    let r = reconcile_thread(&a3, &tid, false).unwrap();
    assert_eq!(r.disposition, Disposition::ReconciledTerminal);
    let r_orphan = reconcile_thread(&a3, &tid, true).unwrap();
    assert!(matches!(
        r_orphan.disposition,
        Disposition::Uncertain { .. }
    ));
    a3.close();
    // Pure classification guard (uncertain handling is explicit).
    assert!(matches!(
        classify(None, false),
        Disposition::Uncertain { .. }
    ));
}

// --------------------------------------------- task 1: stderr / cleanup

#[test]
fn stderr_ring_bounded() {
    let a = spawn("stderr_flood");
    a.handshake().unwrap();
    assert!(poll_until(
        || a.stderr_tail(1).first().map(String::as_str) == Some("stderr-line-2999"),
        Duration::from_secs(10)
    ));
    let tail = a.stderr_tail(5000);
    assert!(
        tail.len() <= DEFAULT_STDERR_RING_LINES,
        "ring must be bounded: {}",
        tail.len()
    );
    assert_eq!(tail.last().unwrap(), "stderr-line-2999");
    assert_ne!(
        tail.first().unwrap(),
        "stderr-line-0",
        "oldest lines dropped"
    );
    a.close();
}

#[test]
fn exit_cleanup_reaps_child() {
    let a = spawn("echo");
    a.handshake().unwrap();
    let pid = a.pid();
    a.close();
    assert_eq!(a.stats().exit.unwrap().code, Some(0));
    assert!(
        poll_until(|| !alive(pid), Duration::from_secs(5)),
        "child must be reaped, not zombie"
    );
}

#[test]
fn close_kills_unresponsive_engine() {
    let mut c = cfg("ignore_eof");
    c.close_grace = Duration::from_millis(300);
    let a = Adapter::spawn_default(c).unwrap();
    a.handshake().unwrap();
    let pid = a.pid();
    let t0 = Instant::now();
    a.close();
    assert!(t0.elapsed() < Duration::from_secs(8), "close must not hang");
    let exit = a.stats().exit.unwrap();
    assert_eq!(
        exit.signal,
        Some(9),
        "unresponsive engine must be SIGKILLed: {exit:?}"
    );
    assert!(poll_until(|| !alive(pid), Duration::from_secs(5)));
}

// ------------------------------------------------------------- purity

#[test]
fn purity_no_core_or_model_dependencies() {
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.contains("serde"));
    for forbidden in [
        "codex",
        "tokio",
        "reqwest",
        "openai",
        "hyper",
        "path = \"../../..",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "forbidden dependency marker: {forbidden}"
        );
    }
    for src in [
        include_str!("../src/lib.rs"),
        include_str!("../src/adapter.rs"),
        include_str!("../src/config.rs"),
        include_str!("../src/approval.rs"),
        include_str!("../src/ui.rs"),
        include_str!("../src/reconcile.rs"),
    ] {
        // Actual dependency/import markers (prose mentions are fine).
        for forbidden in [
            "codex_rs",
            "codex_core",
            "codex_app_server",
            "use codex",
            "extern crate codex",
        ] {
            assert!(!src.contains(forbidden), "engine core leakage: {forbidden}");
        }
    }
}
