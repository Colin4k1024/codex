//! `superagent run --dry-run` / plan: validate the workflow and compute the
//! serial schedule WITHOUT starting any model or command. Pure planning over
//! contracts `dry_run` + orchestrator `topo_order`.

use serde_json::{json, Value};
use superagent_contracts::{dry_run, DryRunContext, GateKind, NodeKind, Workflow};
use superagent_orchestrator::{topo_order, PlanError};

#[derive(Debug, Clone)]
pub struct DryRunReport {
    pub order: Vec<String>,
    pub nodes: Vec<Value>,
}

/// Validate + schedule. Never spawns an engine, never runs a command, never
/// calls a model. A validation error or DAG cycle is returned before any
/// execution could occur.
pub fn dry_run_plan(wf: &Workflow, ctx: &DryRunContext) -> Result<DryRunReport, PlanError> {
    let errs = dry_run(wf, ctx);
    if !errs.is_empty() {
        let msg = errs
            .iter()
            .map(|e| format!("{}: {}", e.location, e.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(PlanError(msg));
    }
    let order = topo_order(wf)?;
    let nodes = order
        .iter()
        .map(|id| {
            let n = wf.nodes.iter().find(|n| &n.id == id).expect("in order");
            serde_json::to_value(n).unwrap_or_else(|_| json!({ "id": id }))
        })
        .collect();
    Ok(DryRunReport { order, nodes })
}

/// JSON output explicitly attests that no model/command was started.
pub fn to_json(r: &DryRunReport) -> Value {
    json!({
        "dryRun": true,
        "startedModel": false,
        "startedCommand": false,
        "validated": true,
        "order": r.order,
        "nodes": r.nodes,
    })
}

/// Derive an availability context from the workflow itself so a self-consistent
/// workflow validates. Catalogs are collected from declared nodes/gates; this
/// never executes anything.
pub fn ctx_from_workflow(wf: &Workflow) -> DryRunContext {
    let mut ctx = DryRunContext::default();
    let mut program = |argv: &[String]| {
        if let Some(p) = argv.first() {
            if !ctx.programs.contains(p) {
                ctx.programs.push(p.clone());
            }
        }
    };
    for node in &wf.nodes {
        if let Some(m) = &node.model {
            if !ctx.models.contains(m) {
                ctx.models.push(m.clone());
            }
        }
        match &node.kind {
            NodeKind::Capability { command_ref, .. } => {
                if !ctx.command_catalog.contains(command_ref) {
                    ctx.command_catalog.push(command_ref.clone());
                }
            }
            NodeKind::Verification { argv, .. } => program(argv),
            NodeKind::Loop { .. } => {}
        }
    }
    for gate in wf.gates.values() {
        match &gate.kind {
            GateKind::Command { argv, .. } => program(argv),
            GateKind::ArtifactSchema { schema_ref, .. } => {
                if !ctx.schema_registry.contains(schema_ref) {
                    ctx.schema_registry.push(schema_ref.clone());
                }
            }
            GateKind::Approval { .. } => {}
        }
    }
    ctx
}
