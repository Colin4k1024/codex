//! VerificationExecutor (P2-5 / #82): gates as REAL processes / validations.
//!
//! Command gates capture the ACTUAL process exit code — via the engine
//! `command/exec` RPC when an engine sandbox policy is expressible (the engine
//! enforces the sandbox), or via a direct child ([`crate::program`]) ONLY under
//! an explicit `LocalProcess` authorization. A model's self-reported exit code
//! is never consulted. ArtifactSchema gates validate the artifact against its
//! registered JSON Schema. Approval gates return `NeedsInput` (never
//! auto-approved). An inexpressible sandbox returns [`ExecKind::Blocked`] with
//! no host-shell / externalSandbox fallback.
//!
//! Every gate outcome carries a durable, digest-backed `evidence_ref`; a bare
//! `passed=true` without evidence is never proof (orchestrator `is_proven`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use superagent_contracts::{Gate, GateKind, Node, NodeKind, Workflow};
use superagent_orchestrator::{ExecKind, ExecOutcome, GateOutcome, NodeExecutor};

use crate::evidence::EvidenceRecorder;
use crate::permissions::PermissionsConfig;
use crate::program::{log_program_run, run_program, ProgramSpec};
use crate::{blocked, fatal, gate, uncertain, EngineSpec};

pub struct VerificationExecutor {
    pub wf: Workflow,
    pub engine: EngineSpec,
    pub permissions: PermissionsConfig,
    pub workspace: PathBuf,
    pub evidence: EvidenceRecorder,
    /// schema_ref -> JSON Schema document, for ArtifactSchema gates.
    pub schema_registry: BTreeMap<String, Value>,
    pub gate_timeout: Duration,
    pub output_cap_bytes: usize,
}

impl VerificationExecutor {
    fn needs_input(&self, prompt: &str) -> ExecOutcome {
        ExecOutcome {
            kind: ExecKind::NeedsInput,
            tokens_used: 0,
            elapsed_secs: 0,
            gates: vec![],
            artifacts: vec![],
            detail: format!("approval gate requires operator decision: {prompt}"),
        }
    }

    /// Run a command gate's argv and capture its REAL exit code. Engine sandbox
    /// intents go through `command/exec`; only LocalProcess runs a direct child.
    fn run_command(
        &self,
        gate_id: &str,
        argv: &[String],
        expect: i32,
        node_id: &str,
        attempt: u32,
    ) -> Result<GateOutcome, ExecOutcome> {
        match self.permissions.engine_sandbox_policy() {
            Ok(policy) => self.run_command_exec(gate_id, argv, expect, node_id, attempt, &policy),
            Err(_) => match self.permissions.gate_exec_allowed() {
                Ok(()) => Ok(self.run_direct(gate_id, argv, expect, node_id, attempt)),
                Err(b) => Err(blocked(b.0)),
            },
        }
    }

    fn run_command_exec(
        &self,
        gate_id: &str,
        argv: &[String],
        expect: i32,
        node_id: &str,
        attempt: u32,
        policy: &Value,
    ) -> Result<GateOutcome, ExecOutcome> {
        let log = self
            .evidence
            .dir()
            .join(format!("{node_id}-a{attempt}-{gate_id}-engine.jsonl"));
        let adapter = self
            .engine
            .adapter(Some(log))
            .map_err(|e| fatal(format!("engine spawn failed for gate {gate_id}: {e}")))?;
        let result = (|| -> Result<GateOutcome, ExecOutcome> {
            adapter
                .handshake()
                .map_err(|e| fatal(format!("handshake failed for gate {gate_id}: {e}")))?;
            let params = json!({
                "command": argv,
                "cwd": self.workspace,
                "timeoutMs": self.gate_timeout.as_millis() as i64,
                "outputBytesCap": self.output_cap_bytes,
                "sandboxPolicy": policy,
            });
            let resp = adapter
                .request("command/exec", params, Some(self.gate_timeout))
                .map_err(|e| {
                    uncertain(format!(
                        "command/exec gate {gate_id} faulted ({e}); outcome not provable"
                    ))
                })?;
            // REAL exit code from the engine's child process — never model-supplied.
            let exit_code = resp
                .get("exitCode")
                .and_then(Value::as_i64)
                .map(|c| c as i32);
            let stdout = resp.get("stdout").and_then(Value::as_str).unwrap_or("");
            let stderr = resp.get("stderr").and_then(Value::as_str).unwrap_or("");
            let passed = exit_code == Some(expect);
            let report = json!({
                "gateId": gate_id, "path": "command/exec", "argv": argv,
                "expectExitCode": expect, "actualExitCode": exit_code,
                "passed": passed, "stdout": stdout, "stderr": stderr,
                "sandboxPolicy": policy,
            });
            let bytes = serde_json::to_vec_pretty(&report).unwrap_or_default();
            let evidence_ref = self
                .evidence
                .record(
                    Some(node_id),
                    &format!("{node_id}-{gate_id}-gate.json"),
                    &bytes,
                    "application/json",
                    None,
                    Some(&format!("gate {gate_id} exit={exit_code:?}")),
                )
                .map_err(|e| fatal(format!("gate evidence write failed: {e}")))?;
            Ok(gate(gate_id, passed, Some(evidence_ref), exit_code))
        })();
        adapter.close();
        result
    }

    fn run_direct(
        &self,
        gate_id: &str,
        argv: &[String],
        expect: i32,
        node_id: &str,
        attempt: u32,
    ) -> GateOutcome {
        let spec = ProgramSpec {
            argv: argv.to_vec(),
            cwd: self.workspace.clone(),
            timeout: self.gate_timeout,
            output_cap_bytes: self.output_cap_bytes,
        };
        let run = run_program(&spec);
        let _ = std::fs::create_dir_all(self.evidence.dir());
        let log_path = self
            .evidence
            .dir()
            .join(format!("{node_id}-a{attempt}-{gate_id}.runlog"));
        let log_ok = log_program_run(&log_path, 100 * 1024 * 1024, argv, &run).is_ok();
        let passed =
            run.spawn_error.is_none() && !run.timed_out && run.exit_code == Some(expect) && log_ok;
        let evidence_ref = if log_ok {
            Some(log_path.to_string_lossy().into_owned())
        } else {
            None
        };
        gate(gate_id, passed, evidence_ref, run.exit_code)
    }

    fn eval_artifact_schema(
        &self,
        gate_id: &str,
        artifact: &str,
        schema_ref: &str,
        node_id: &str,
    ) -> GateOutcome {
        let mut errors: Vec<String> = Vec::new();
        let mut valid = false;
        let art_path = self
            .wf
            .artifacts
            .get(artifact)
            .map(|a| self.workspace.join(&a.path));
        let schema = self.schema_registry.get(schema_ref);
        match (art_path, schema) {
            (Some(path), Some(schema)) => match std::fs::read(&path) {
                Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(instance) => match jsonschema::validator_for(schema) {
                        Ok(validator) => {
                            let errs: Vec<String> = validator
                                .iter_errors(&instance)
                                .map(|e| e.to_string())
                                .collect();
                            if errs.is_empty() {
                                valid = true;
                            } else {
                                errors = errs;
                            }
                        }
                        Err(e) => errors.push(format!("invalid schema '{schema_ref}': {e}")),
                    },
                    Err(e) => errors.push(format!("artifact '{artifact}' is not valid JSON: {e}")),
                },
                Err(e) => errors.push(format!("cannot read artifact '{}': {e}", path.display())),
            },
            (None, _) => errors.push(format!("unknown artifact '{artifact}'")),
            (_, None) => errors.push(format!("schema_ref '{schema_ref}' not in registry")),
        }
        let report = json!({
            "gateId": gate_id, "path": "artifactSchema", "artifact": artifact,
            "schemaRef": schema_ref, "valid": valid, "errors": errors,
        });
        let bytes = serde_json::to_vec_pretty(&report).unwrap_or_default();
        let evidence_ref = self
            .evidence
            .record(
                Some(node_id),
                &format!("{node_id}-{gate_id}-schema.json"),
                &bytes,
                "application/json",
                Some(schema_ref),
                Some(&format!("schema gate {gate_id} valid={valid}")),
            )
            .ok();
        gate(gate_id, valid, evidence_ref, None)
    }

    fn eval_gate(
        &self,
        gate_id: &str,
        node_id: &str,
        attempt: u32,
    ) -> Result<GateOutcome, ExecOutcome> {
        let g: &Gate = match self.wf.gates.get(gate_id) {
            Some(g) => g,
            None => return Ok(gate(gate_id, false, None, None)),
        };
        match &g.kind {
            GateKind::Command {
                argv,
                expect_exit_code,
            } => self.run_command(gate_id, argv, *expect_exit_code, node_id, attempt),
            GateKind::ArtifactSchema {
                artifact,
                schema_ref,
            } => Ok(self.eval_artifact_schema(gate_id, artifact, schema_ref, node_id)),
            GateKind::Approval { prompt } => Err(self.needs_input(prompt)),
        }
    }

    fn run_verification(
        &self,
        node: &Node,
        argv: &[String],
        gates: &[String],
        attempt: u32,
    ) -> ExecOutcome {
        let start = Instant::now();
        // The verification program itself must run and exit 0 (real exit code).
        let prog =
            match self.run_command(&format!("{}:program", node.id), argv, 0, &node.id, attempt) {
                Ok(g) => g,
                Err(blocked_outcome) => return blocked_outcome,
            };
        if !prog.passed {
            let prog_exit = prog.exit_code;
            return ExecOutcome {
                kind: ExecKind::RetryableFailure,
                tokens_used: 0,
                elapsed_secs: start.elapsed().as_secs(),
                gates: vec![prog],
                artifacts: vec![],
                detail: format!("verification program failed (exit={prog_exit:?})"),
            };
        }
        let mut outcomes = vec![prog];
        for gid in gates {
            match self.eval_gate(gid, &node.id, attempt) {
                Ok(g) => outcomes.push(g),
                Err(needs_input) => return needs_input,
            }
        }
        let mut o = ExecOutcome::completed(outcomes, 0, start.elapsed().as_secs());
        o.detail = format!("verification gates evaluated for {}", node.id);
        o
    }

    fn run_loop(&self, node: &Node, attempt: u32) -> ExecOutcome {
        let (body, max_attempts, wall_timeout_secs, gate_id) = match &node.kind {
            NodeKind::Loop {
                body,
                max_attempts,
                wall_timeout_secs,
                gate,
            } => (body, *max_attempts, *wall_timeout_secs, gate),
            _ => return fatal("run_loop requires a loop node"),
        };
        let start = Instant::now();
        let wall = Duration::from_secs(wall_timeout_secs);
        let mut last: Option<GateOutcome> = None;
        for i in 1..=max_attempts {
            if start.elapsed() >= wall {
                break;
            }
            match self.eval_gate(gate_id, &node.id, attempt) {
                Ok(g) => {
                    let proven = g.is_proven();
                    last = Some(g);
                    if proven {
                        break;
                    }
                }
                Err(needs_input) => return needs_input,
            }
            let _ = i;
            let _ = body;
        }
        let outcomes = last.into_iter().collect::<Vec<_>>();
        let mut o = ExecOutcome::completed(outcomes, 0, start.elapsed().as_secs());
        o.detail = format!("loop body '{body}' gate '{gate_id}' bounded by {max_attempts} attempts / {wall_timeout_secs}s");
        o
    }
}

impl NodeExecutor for VerificationExecutor {
    fn execute(&mut self, node: &Node, attempt: u32) -> ExecOutcome {
        match &node.kind {
            NodeKind::Verification { argv, gates } => {
                self.run_verification(node, argv, gates, attempt)
            }
            NodeKind::Loop { .. } => self.run_loop(node, attempt),
            NodeKind::Capability { .. } => {
                fatal("VerificationExecutor only executes verification/loop nodes")
            }
        }
    }
}
