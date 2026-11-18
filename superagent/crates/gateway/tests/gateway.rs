//! P4-1 / #90 gateway acceptance: dual-tenant isolation, process-pool crash
//! recovery + leak-free reaping, traceable audit, bounded concurrency, and a
//! real TCP unified entry routing app-server JSON-RPC per tenant. Model-free:
//! the only subprocess is fake_engine.py (real stdio JSON-RPC) + `kill`.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use superagent_gateway::{
    serve_connection, AuditLog, EngineTemplate, GatewayError, GatewayPool, RunManifest,
};

fn fake_engine() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../adapter/tests/fake_engine.py")
        .canonicalize()
        .expect("fake_engine.py present")
}

fn template(dir: &Path) -> EngineTemplate {
    EngineTemplate {
        argv: vec![
            "python3".into(),
            fake_engine().to_string_lossy().into_owned(),
            "--scenario".into(),
            "echo".into(),
            "--state-file".into(),
            dir.join("state.json").to_string_lossy().into_owned(),
        ],
    }
}

fn pool(dir: &Path, max: usize) -> GatewayPool {
    GatewayPool::new(
        &dir.join("tenants"),
        template(dir),
        max,
        AuditLog::new(dir.join("audit.jsonl")).unwrap(),
    )
    .unwrap()
}

fn kill_pid(pid: u32, sig: &str) -> bool {
    Command::new("kill")
        .arg(format!("-{sig}"))
        .arg(pid.to_string())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn wait_dead(pool: &GatewayPool, tenant: &str) {
    for _ in 0..60 {
        if !pool.is_alive(tenant).unwrap() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("engine {tenant} did not die");
}

fn req(id: u64, method: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":{}})
}

// 1. Dual-tenant isolation: each engine reports its own CODEX_HOME; a config/auth
//    marker written in tenant A's home is invisible to tenant B.
#[test]
fn dual_tenant_isolation() {
    let dir = tempfile::tempdir().unwrap();
    let p = pool(dir.path(), 4);
    let pa = p.acquire("tenant-a").unwrap();
    let pb = p.acquire("tenant-b").unwrap();
    assert_ne!(pa, pb);
    let ha = p.codex_home("tenant-a");
    let hb = p.codex_home("tenant-b");
    assert_ne!(ha, hb);

    let ra = p.forward("tenant-a", &req(1, "initialize")).unwrap();
    let rb = p.forward("tenant-b", &req(2, "initialize")).unwrap();
    assert_eq!(ra["result"]["codexHome"], json!(ha.display().to_string()));
    assert_eq!(rb["result"]["codexHome"], json!(hb.display().to_string()));

    // A private auth/config marker in A's home must not appear in B's home.
    std::fs::write(ha.join("auth.json"), "{\"token\":\"tenant-a-secret\"}").unwrap();
    assert!(ha.join("auth.json").exists());
    assert!(
        !hb.join("auth.json").exists(),
        "tenant B must not see A's auth"
    );

    let ea = p.forward("tenant-a", &req(3, "test/env")).unwrap();
    let eb = p.forward("tenant-b", &req(4, "test/env")).unwrap();
    assert_eq!(ea["result"]["codexHome"], json!(ha.display().to_string()));
    assert_eq!(eb["result"]["codexHome"], json!(hb.display().to_string()));
    assert_eq!(ea["result"]["pid"], json!(pa));
    assert_eq!(eb["result"]["pid"], json!(pb));
    p.release_all().unwrap();
}

// 2. Crash recovery: kill a live engine, detect death, restart -> new live pid,
//    respawned engine functional, audit records crash + restart.
#[test]
fn pool_crash_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let p = pool(dir.path(), 2);
    let pid1 = p.acquire("t").unwrap();
    assert!(p.is_alive("t").unwrap());
    assert!(kill_pid(pid1, "9"), "simulate crash");
    wait_dead(&p, "t");
    assert!(!p.is_alive("t").unwrap());

    let pid2 = p.restart("t").unwrap();
    assert_ne!(pid1, pid2, "restart spawns a new engine");
    assert!(p.is_alive("t").unwrap());
    let pong = p.forward("t", &req(1, "test/ping")).unwrap();
    assert_eq!(pong["result"]["pong"], json!(true));

    let audit = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
    assert!(audit.contains("\"event\":\"crash\""));
    assert!(audit.contains("\"event\":\"restart\""));
    p.release_all().unwrap();
}

// 3. No leak: release_all reaps every engine; no orphan process survives.
#[test]
fn pool_release_no_leak() {
    let dir = tempfile::tempdir().unwrap();
    let p = pool(dir.path(), 4);
    let mut pids = vec![];
    for t in ["a", "b", "c"] {
        pids.push(p.acquire(t).unwrap());
    }
    assert_eq!(p.active_count(), 3);
    p.release_all().unwrap();
    assert_eq!(p.active_count(), 0);
    for pid in pids {
        // kill -0 fails (ESRCH) once the process is reaped — no orphan/zombie.
        assert!(!kill_pid(pid, "0"), "pid {pid} leaked");
    }
    assert!(matches!(
        p.is_alive("a"),
        Err(GatewayError::UnknownTenant(_))
    ));
}

// 4. Audit traceable: lifecycle events + run manifest with version/digest/platform/protocol.
#[test]
fn audit_and_manifest_traceable() {
    let dir = tempfile::tempdir().unwrap();
    let p = pool(dir.path(), 2);
    p.acquire("a").unwrap();
    p.restart("a").unwrap();
    p.release("a").unwrap();

    let audit = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
    for ev in ["spawn", "crash", "restart", "release"] {
        assert!(
            audit.contains(&format!("\"event\":\"{ev}\"")),
            "missing {ev}: {audit}"
        );
    }
    assert!(audit.contains("\"tenant\":\"a\""));
    assert!(audit.contains("\"codexHome\""));

    let manifest = RunManifest {
        product_version: "superagent/0.1.0".into(),
        engine_digest: "sha256:deadbeef".into(),
        platform: "macos-arm64".into(),
        protocol_summary: "app-server JSONL stdio (initialize/thread/turn/command)".into(),
    };
    let mp = dir.path().join("manifest.json");
    manifest.write(&mp).unwrap();
    let m: Value = serde_json::from_str(&std::fs::read_to_string(&mp).unwrap()).unwrap();
    for k in [
        "productVersion",
        "engineDigest",
        "platform",
        "protocolSummary",
    ] {
        assert!(!m[k].as_str().unwrap().is_empty(), "manifest {k} empty");
    }
}

// 5. Bounded concurrency: max tenants enforced; release frees a slot.
#[test]
fn concurrency_cap_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let p = pool(dir.path(), 2);
    p.acquire("a").unwrap();
    p.acquire("b").unwrap();
    assert!(matches!(
        p.acquire("c"),
        Err(GatewayError::ConcurrencyLimit(2))
    ));
    p.release("a").unwrap();
    p.acquire("c").unwrap();
    assert_eq!(p.active_count(), 2);
    p.release_all().unwrap();
}

// 6. Real TCP unified entry routes app-server JSON-RPC to the correct tenant
//    engine over a socket; responses carry each tenant's isolated CODEX_HOME.
#[test]
fn unified_entry_tcp_routes_tenants() {
    let dir = tempfile::tempdir().unwrap();
    let p = pool(dir.path(), 4);
    p.acquire("tenant-a").unwrap();
    p.acquire("tenant-b").unwrap();
    let ha = p.codex_home("tenant-a").display().to_string();
    let hb = p.codex_home("tenant-b").display().to_string();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let client = thread::spawn(move || {
        let stream = TcpStream::connect(addr).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut ask = |id: u64, tenant: &str| -> Value {
            let r = json!({"tenantId":tenant,"jsonrpc":"2.0","id":id,"method":"initialize","params":{}});
            writeln!(writer, "{}", serde_json::to_string(&r).unwrap()).unwrap();
            writer.flush().unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            serde_json::from_str(line.trim()).unwrap()
        };
        let ra = ask(1, "tenant-a");
        let rb = ask(2, "tenant-b");
        (ra, rb)
    });

    serve_connection(&listener, &p).unwrap();
    let (ra, rb) = client.join().unwrap();
    assert_eq!(ra["result"]["codexHome"], json!(ha));
    assert_eq!(rb["result"]["codexHome"], json!(hb));
    assert_ne!(ha, hb);
    p.release_all().unwrap();
}
