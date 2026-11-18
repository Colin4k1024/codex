//! SuperAgent dev workbench (P5-5 / #99).
//!
//! A low-code workflow canvas whose data model is the P2 workflow schema (#78).
//! The canvas holds nodes / dependency edges / gates / artifacts plus visual
//! layout metadata; exporting strips the layout and generates a `Workflow` JSON
//! that is validated by `contracts::dry_run` (#78) and executable by the #81
//! orchestrator. A minimal Application Space gives a per-tenant resource view.
//!
//! Model-free / process-free: generation + validation are pure functions; the
//! orchestrator execution proof in tests uses an injected deterministic executor.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use superagent_contracts::{
    dry_run, ArtifactSpec, DryRunContext, Gate, Node, NodeKind, ValidationError, VariableSpec,
    Workflow, SCHEMA_VERSION,
};

/// Visual position of a canvas node (editor-only; stripped on export).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

/// A canvas node: a workflow node plus its editor layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasNode {
    pub id: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(flatten)]
    pub kind: NodeKind,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub budget_tokens: Option<u64>,
    pub position: Position,
}

/// The workflow canvas editor model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Canvas {
    pub id: String,
    pub version: String,
    pub nodes: Vec<CanvasNode>,
    #[serde(default)]
    pub gates: BTreeMap<String, Gate>,
    #[serde(default)]
    pub artifacts: BTreeMap<String, ArtifactSpec>,
    #[serde(default)]
    pub variables: BTreeMap<String, VariableSpec>,
}

impl Canvas {
    pub fn new(id: &str, version: &str) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            nodes: Vec::new(),
            gates: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            variables: BTreeMap::new(),
        }
    }

    pub fn add_node(&mut self, node: CanvasNode) {
        self.nodes.push(node);
    }

    pub fn add_gate(&mut self, gate: Gate) {
        self.gates.insert(gate.id.clone(), gate);
    }

    pub fn add_artifact(&mut self, id: &str, spec: ArtifactSpec) {
        self.artifacts.insert(id.into(), spec);
    }

    /// Dependency edges for visualization: (from, to) for each depends_on.
    pub fn edges(&self) -> Vec<(String, String)> {
        let mut edges = Vec::new();
        for n in &self.nodes {
            for dep in &n.depends_on {
                edges.push((dep.clone(), n.id.clone()));
            }
        }
        edges
    }

    /// Generate the workflow, stripping editor-only layout metadata.
    pub fn to_workflow(&self) -> Workflow {
        Workflow {
            schema_version: SCHEMA_VERSION,
            id: self.id.clone(),
            version: self.version.clone(),
            variables: self.variables.clone(),
            nodes: self
                .nodes
                .iter()
                .map(|n| Node {
                    id: n.id.clone(),
                    depends_on: n.depends_on.clone(),
                    kind: n.kind.clone(),
                    model: n.model.clone(),
                    budget_tokens: n.budget_tokens,
                })
                .collect(),
            gates: self.gates.clone(),
            artifacts: self.artifacts.clone(),
        }
    }

    /// Canvas -> workflow JSON (the export consumed by the orchestrator).
    pub fn to_workflow_json(&self) -> Value {
        serde_json::to_value(self.to_workflow()).expect("serialize workflow")
    }

    /// Validate the generated workflow against the #78 contracts dry-run.
    pub fn validate(&self, ctx: &DryRunContext) -> Vec<ValidationError> {
        dry_run(&self.to_workflow(), ctx)
    }
}

/// Minimal per-tenant Application Space resource view.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceView {
    pub tenant: String,
    pub workflows: Vec<String>,
    pub artifacts: Vec<String>,
    pub runs: Vec<String>,
}

#[derive(Debug, Default)]
pub struct AppSpace {
    spaces: BTreeMap<String, ResourceView>,
}

impl AppSpace {
    pub fn new() -> Self {
        Self::default()
    }

    fn entry(&mut self, tenant: &str) -> &mut ResourceView {
        self.spaces
            .entry(tenant.into())
            .or_insert_with(|| ResourceView {
                tenant: tenant.into(),
                ..Default::default()
            })
    }

    pub fn register_workflow(&mut self, tenant: &str, workflow_id: &str, artifacts: &[String]) {
        let view = self.entry(tenant);
        if !view.workflows.iter().any(|w| w == workflow_id) {
            view.workflows.push(workflow_id.into());
        }
        for a in artifacts {
            if !view.artifacts.iter().any(|x| x == a) {
                view.artifacts.push(a.clone());
            }
        }
    }

    pub fn record_run(&mut self, tenant: &str, run_id: &str) {
        let view = self.entry(tenant);
        if !view.runs.iter().any(|r| r == run_id) {
            view.runs.push(run_id.into());
        }
    }

    pub fn view(&self, tenant: &str) -> Option<&ResourceView> {
        self.spaces.get(tenant)
    }

    pub fn tenants(&self) -> Vec<&str> {
        self.spaces.keys().map(String::as_str).collect()
    }
}
