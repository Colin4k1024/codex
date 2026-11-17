//! P2-1 (#78) acceptance tests: schema validation (unknown types fail, DAG
//! cycles detected, loop constraints) and dry-run purity (no model/commands).

use std::collections::BTreeMap;

use superagent_contracts::{dry_run, DryRunContext, NodeKind, Workflow};

const VALID: &str = include_str!("fixtures/valid-workflow.json");

fn ctx() -> DryRunContext {
    let mut variables = BTreeMap::new();
    variables.insert("workspace".to_string(), serde_json::json!("/tmp/fixture"));
    DryRunContext {
        variables,
        command_catalog: vec!["team.plan".into(), "team.implement".into()],
        models: vec!["gpt-6-astra".into()],
        programs: vec!["python3".into()],
        schema_registry: vec!["schemas/patch.json".into()],
    }
}

fn wf() -> Workflow {
    serde_json::from_str(VALID).expect("valid fixture must deserialize")
}

#[test]
fn valid_fixture_passes_dry_run() {
    assert!(dry_run(&wf(), &ctx()).is_empty());
}

#[test]
fn unknown_node_type_fails_deserialization() {
    let bad = VALID.replace("\"type\": \"capability\"", "\"type\": \"magic_stub\"");
    let err = serde_json::from_str::<Workflow>(&bad).unwrap_err();
    assert!(err.to_string().contains("unknown variant"), "{err}");
}

#[test]
fn unknown_gate_kind_fails_deserialization() {
    let bad = VALID.replace("\"kind\": \"command\"", "\"kind\": \"vibes\"");
    assert!(serde_json::from_str::<Workflow>(&bad).is_err());
}

#[test]
fn unknown_field_rejected() {
    let bad = VALID.replace("\"schema_version\": 1,", "\"schema_version\": 1, \"surprise\": true,");
    assert!(serde_json::from_str::<Workflow>(&bad).is_err());
}

#[test]
fn dag_cycle_detected() {
    let mut w = wf();
    // plan -> implement -> verify -> fix-loop; make plan depend on fix-loop.
    w.nodes.iter_mut().find(|n| n.id == "plan").unwrap().depends_on.push("fix-loop".into());
    let errs = dry_run(&w, &ctx());
    assert!(errs.iter().any(|e| e.message.contains("DAG cycle detected")), "{errs:?}");
}

#[test]
fn self_dependency_detected() {
    let mut w = wf();
    w.nodes.iter_mut().find(|n| n.id == "plan").unwrap().depends_on.push("plan".into());
    let errs = dry_run(&w, &ctx());
    assert!(errs.iter().any(|e| e.message.contains("depends on itself")), "{errs:?}");
}

#[test]
fn loop_missing_required_fields_fails_deserialization() {
    for field in ["\"max_attempts\": 2,", "\"wall_timeout_secs\": 1800,", "\"gate\": \"tests-pass\""] {
        let bad = VALID.replace(field, "");
        assert!(serde_json::from_str::<Workflow>(&bad).is_err(), "missing {field} must fail");
    }
}

#[test]
fn loop_zero_bounds_fail_dry_run() {
    let mut w = wf();
    if let NodeKind::Loop { max_attempts, wall_timeout_secs, .. } =
        &mut w.nodes.iter_mut().find(|n| n.id == "fix-loop").unwrap().kind
    {
        *max_attempts = 0;
        *wall_timeout_secs = 0;
    }
    let errs = dry_run(&w, &ctx());
    assert!(errs.iter().any(|e| e.message.contains("max_attempts must be >= 1")));
    assert!(errs.iter().any(|e| e.message.contains("wall_timeout_secs must be > 0")));
}

#[test]
fn loop_unknown_gate_fails() {
    let mut w = wf();
    if let NodeKind::Loop { gate, .. } =
        &mut w.nodes.iter_mut().find(|n| n.id == "fix-loop").unwrap().kind
    {
        *gate = "no-such-gate".into();
    }
    let errs = dry_run(&w, &ctx());
    assert!(errs.iter().any(|e| e.message.contains("unknown gate 'no-such-gate'")));
}

#[test]
fn verification_requires_gates() {
    let mut w = wf();
    if let NodeKind::Verification { gates, .. } =
        &mut w.nodes.iter_mut().find(|n| n.id == "verify").unwrap().kind
    {
        gates.clear();
    }
    let errs = dry_run(&w, &ctx());
    assert!(errs.iter().any(|e| e.message.contains("requires at least one gate")));
}

#[test]
fn undeclared_variable_reference_fails() {
    let bad = VALID.replace("${workspace}/tests", "${missing_var}/tests");
    let w: Workflow = serde_json::from_str(&bad).unwrap();
    let errs = dry_run(&w, &ctx());
    assert!(errs.iter().any(|e| e.message.contains("undeclared variable reference '${missing_var}'")), "{errs:?}");
}

#[test]
fn required_variable_without_value_fails() {
    let mut c = ctx();
    c.variables.clear();
    let errs = dry_run(&wf(), &c);
    assert!(errs.iter().any(|e| e.location == "variable:workspace"
        && e.message.contains("required variable has no value")));
}

#[test]
fn variable_type_mismatch_fails() {
    let mut c = ctx();
    c.variables.insert("workspace".into(), serde_json::json!(42));
    let errs = dry_run(&wf(), &c);
    assert!(errs.iter().any(|e| e.message.contains("does not match type")));
}

#[test]
fn unavailable_command_ref_fails() {
    let mut c = ctx();
    c.command_catalog.clear();
    let errs = dry_run(&wf(), &c);
    assert!(errs.iter().any(|e| e.message.contains("not in team command catalog")));
}

#[test]
fn unavailable_program_fails() {
    let mut c = ctx();
    c.programs.clear();
    let errs = dry_run(&wf(), &c);
    assert!(errs.iter().any(|e| e.message.contains("not in available programs catalog")));
}

#[test]
fn unavailable_model_fails() {
    let mut w = wf();
    w.nodes[0].model = Some("nonexistent-model".into());
    let errs = dry_run(&w, &ctx());
    assert!(errs.iter().any(|e| e.message.contains("model 'nonexistent-model' not available")));
}

#[test]
fn unknown_artifact_and_schema_ref_fail() {
    let mut w = wf();
    if let NodeKind::Capability { outputs, .. } = &mut w.nodes[0].kind {
        outputs.push("ghost-artifact".into());
    }
    let errs = dry_run(&w, &ctx());
    assert!(errs.iter().any(|e| e.message.contains("unknown artifact 'ghost-artifact'")));

    let mut c = ctx();
    c.schema_registry.clear();
    let errs = dry_run(&wf(), &c);
    assert!(errs.iter().any(|e| e.message.contains("not in registry")));
}

#[test]
fn duplicate_node_ids_fail() {
    let mut w = wf();
    w.nodes.push(w.nodes[0].clone());
    let errs = dry_run(&w, &ctx());
    assert!(errs.iter().any(|e| e.message.contains("duplicate node id")));
}

#[test]
fn dry_run_is_pure_no_execution_surface() {
    // Structural guard: the contracts crate must contain no process/command
    // execution surface; dry-run can never start a model or command.
    let src = include_str!("../src/lib.rs");
    assert!(!src.contains("std::process"), "process execution found in contracts");
    assert!(!src.contains("Command::new"), "command spawn found in contracts");
    assert!(!src.contains("codex_"), "engine DTO leakage into contracts");
}

#[test]
fn node_schema_enumerates_known_types_only() {
    let schema = serde_json::to_value(schemars::schema_for!(superagent_contracts::Node)).unwrap();
    let text = schema.to_string();
    for t in ["capability", "verification", "loop"] {
        assert!(text.contains(t), "schema must enumerate {t}");
    }
}