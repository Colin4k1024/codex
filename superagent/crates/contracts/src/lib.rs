//! SuperAgent Swarm Flow data contracts (P2-1 / #78).
//!
//! Product-side types only: no codex engine DTOs (those stay isolated in the
//! adapter layer), no process/model execution anywhere in this crate.
//! Unknown node/gate types fail deserialization (`deny_unknown_fields` +
//! internally tagged enums); they can never become success stubs.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "ts/")]
pub struct Workflow {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub variables: BTreeMap<String, VariableSpec>,
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub gates: BTreeMap<String, Gate>,
    #[serde(default)]
    pub artifacts: BTreeMap<String, ArtifactSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "ts/")]
pub struct VariableSpec {
    #[serde(rename = "type")]
    pub var_type: VarType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "ts/")]
pub enum VarType {
    String,
    Number,
    Bool,
    Path,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export, export_to = "ts/")]
pub struct Node {
    pub id: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(flatten)]
    pub kind: NodeKind,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub budget_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export, export_to = "ts/")]
pub enum NodeKind {
    /// Team command reference (resolved against the pack command catalog).
    Capability {
        command_ref: String,
        #[serde(default)]
        inputs: Vec<String>,
        #[serde(default)]
        outputs: Vec<String>,
    },
    /// Executable program; success requires every listed gate to pass.
    Verification {
        argv: Vec<String>,
        gates: Vec<String>,
    },
    /// Retry loop: must carry max_attempts, wall_timeout and an explicit gate.
    Loop {
        body: String,
        #[schemars(range(min = 1))]
        max_attempts: u32,
        #[schemars(range(min = 1))]
        wall_timeout_secs: u64,
        gate: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export, export_to = "ts/")]
pub struct Gate {
    pub id: String,
    #[serde(flatten)]
    pub kind: GateKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export, export_to = "ts/")]
pub enum GateKind {
    Command {
        argv: Vec<String>,
        #[serde(default)]
        expect_exit_code: i32,
    },
    ArtifactSchema {
        artifact: String,
        schema_ref: String,
    },
    Approval {
        prompt: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "ts/")]
pub struct ArtifactSpec {
    pub path: String,
    pub media_type: String,
    #[serde(default)]
    pub schema_ref: Option<String>,
}

/// Availability context for dry-run. Everything is checked against these
/// declared catalogs; nothing is executed or spawned.
#[derive(Debug, Clone, Default)]
pub struct DryRunContext {
    pub variables: BTreeMap<String, serde_json::Value>,
    pub command_catalog: Vec<String>,
    pub models: Vec<String>,
    pub programs: Vec<String>,
    pub schema_registry: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub location: String,
    pub message: String,
}

impl ValidationError {
    fn new(location: impl Into<String>, message: impl Into<String>) -> Self {
        Self { location: location.into(), message: message.into() }
    }
}

/// Deterministic dry-run validation: variables, DAG, tool/model availability,
/// gate definitions. Pure function — never starts a model or command.
pub fn dry_run(wf: &Workflow, ctx: &DryRunContext) -> Vec<ValidationError> {
    let mut errs = Vec::new();
    if wf.schema_version != SCHEMA_VERSION {
        errs.push(ValidationError::new("workflow", format!(
            "unsupported schema_version {} (expected {SCHEMA_VERSION})", wf.schema_version)));
    }
    if wf.id.is_empty() || wf.version.is_empty() {
        errs.push(ValidationError::new("workflow", "id and version must be non-empty"));
    }

    let mut ids: BTreeSet<&str> = BTreeSet::new();
    for node in &wf.nodes {
        if !ids.insert(node.id.as_str()) {
            errs.push(ValidationError::new(format!("node:{}", node.id), "duplicate node id"));
        }
        if node.id.is_empty() {
            errs.push(ValidationError::new("node", "node id must be non-empty"));
        }
    }
    for node in &wf.nodes {
        let loc = format!("node:{}", node.id);
        for dep in &node.depends_on {
            if !ids.contains(dep.as_str()) {
                errs.push(ValidationError::new(&loc, format!("depends_on unknown node '{dep}'")));
            }
            if dep == &node.id {
                errs.push(ValidationError::new(&loc, "node depends on itself"));
            }
        }
        if let Some(model) = &node.model {
            if !ctx.models.iter().any(|m| m == model) {
                errs.push(ValidationError::new(&loc, format!("model '{model}' not available")));
            }
        }
        match &node.kind {
            NodeKind::Capability { command_ref, inputs, outputs } => {
                if !ctx.command_catalog.iter().any(|c| c == command_ref) {
                    errs.push(ValidationError::new(&loc, format!(
                        "command_ref '{command_ref}' not in team command catalog")));
                }
                for art in inputs.iter().chain(outputs.iter()) {
                    if !wf.artifacts.contains_key(art) {
                        errs.push(ValidationError::new(&loc, format!("unknown artifact '{art}'")));
                    }
                }
                collect_vars(command_ref, &wf.variables, &mut errs, &loc);
            }
            NodeKind::Verification { argv, gates } => {
                check_argv(argv, ctx, &mut errs, &loc);
                if gates.is_empty() {
                    errs.push(ValidationError::new(&loc, "verification node requires at least one gate"));
                }
                for g in gates {
                    check_gate_ref(g, wf, ctx, &mut errs, &loc);
                }
                for a in argv {
                    collect_vars(a, &wf.variables, &mut errs, &loc);
                }
            }
            NodeKind::Loop { body, max_attempts, wall_timeout_secs, gate } => {
                if !ids.contains(body.as_str()) {
                    errs.push(ValidationError::new(&loc, format!("loop body unknown node '{body}'")));
                }
                if body == &node.id {
                    errs.push(ValidationError::new(&loc, "loop body cannot be the loop itself"));
                }
                if *max_attempts == 0 {
                    errs.push(ValidationError::new(&loc, "loop max_attempts must be >= 1"));
                }
                if *wall_timeout_secs == 0 {
                    errs.push(ValidationError::new(&loc, "loop wall_timeout_secs must be > 0"));
                }
                check_gate_ref(gate, wf, ctx, &mut errs, &loc);
            }
        }
    }

    // DAG cycle detection (Kahn); only over edges whose endpoints exist.
    let mut indeg: BTreeMap<&str, usize> = ids.iter().map(|id| (*id, 0)).collect();
    let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for node in &wf.nodes {
        for dep in &node.depends_on {
            if ids.contains(dep.as_str()) && dep != &node.id {
                adj.entry(dep.as_str()).or_default().push(node.id.as_str());
                *indeg.get_mut(node.id.as_str()).unwrap() += 1;
            }
        }
    }
    let mut queue: VecDeque<&str> = indeg.iter().filter(|(_, d)| **d == 0).map(|(id, _)| *id).collect();
    let mut seen = 0usize;
    while let Some(id) = queue.pop_front() {
        seen += 1;
        if let Some(next) = adj.get(id) {
            for n in next {
                let d = indeg.get_mut(n).unwrap();
                *d -= 1;
                if *d == 0 {
                    queue.push_back(n);
                }
            }
        }
    }
    if seen != ids.len() {
        let cyclic: Vec<&str> = indeg.iter().filter(|(_, d)| **d > 0).map(|(id, _)| *id).collect();
        errs.push(ValidationError::new("workflow", format!("DAG cycle detected among nodes: {cyclic:?}")));
    }

    for (gid, gate) in &wf.gates {
        let loc = format!("gate:{gid}");
        if gate.id != *gid {
            errs.push(ValidationError::new(&loc, "gate.id must match its map key"));
        }
        match &gate.kind {
            GateKind::Command { argv, .. } => {
                check_argv(argv, ctx, &mut errs, &loc);
                for a in argv {
                    collect_vars(a, &wf.variables, &mut errs, &loc);
                }
            }
            GateKind::ArtifactSchema { artifact, schema_ref } => {
                if !wf.artifacts.contains_key(artifact) {
                    errs.push(ValidationError::new(&loc, format!("unknown artifact '{artifact}'")));
                }
                if !ctx.schema_registry.iter().any(|s| s == schema_ref) {
                    errs.push(ValidationError::new(&loc, format!("schema_ref '{schema_ref}' not in registry")));
                }
            }
            GateKind::Approval { prompt } => {
                if prompt.is_empty() {
                    errs.push(ValidationError::new(&loc, "approval gate prompt must be non-empty"));
                }
            }
        }
    }

    for (aid, art) in &wf.artifacts {
        let loc = format!("artifact:{aid}");
        if art.path.is_empty() || art.media_type.is_empty() {
            errs.push(ValidationError::new(&loc, "path and media_type must be non-empty"));
        }
        if let Some(s) = &art.schema_ref {
            if !ctx.schema_registry.iter().any(|r| r == s) {
                errs.push(ValidationError::new(&loc, format!("schema_ref '{s}' not in registry")));
            }
        }
        collect_vars(&art.path, &wf.variables, &mut errs, &loc);
    }

    for (name, spec) in &wf.variables {
        let provided = ctx.variables.get(name).or(spec.default.as_ref());
        if spec.required && provided.is_none() {
            errs.push(ValidationError::new(format!("variable:{name}"),
                "required variable has no value and no default"));
            continue;
        }
        if let Some(v) = provided {
            let ok = match spec.var_type {
                VarType::String | VarType::Path => v.is_string(),
                VarType::Number => v.is_number(),
                VarType::Bool => v.is_boolean(),
            };
            if !ok {
                errs.push(ValidationError::new(format!("variable:{name}"),
                    format!("value {v} does not match type {:?}", spec.var_type)));
            }
        }
    }
    for name in ctx.variables.keys() {
        if !wf.variables.contains_key(name) {
            errs.push(ValidationError::new(format!("variable:{name}"),
                "provided variable is not declared in workflow"));
        }
    }
    errs
}

fn check_argv(argv: &[String], ctx: &DryRunContext, errs: &mut Vec<ValidationError>, loc: &str) {
    if argv.is_empty() {
        errs.push(ValidationError::new(loc, "argv must be non-empty"));
        return;
    }
    if !ctx.programs.iter().any(|p| p == &argv[0]) {
        errs.push(ValidationError::new(loc, format!(
            "program '{}' not in available programs catalog", argv[0])));
    }
}

fn check_gate_ref(g: &str, wf: &Workflow, ctx: &DryRunContext, errs: &mut Vec<ValidationError>, loc: &str) {
    match wf.gates.get(g) {
        None => errs.push(ValidationError::new(loc, format!("unknown gate '{g}'"))),
        Some(gate) => match &gate.kind {
            GateKind::Command { argv, .. } => check_argv(argv, ctx, errs, &format!("{loc}->gate:{g}")),
            GateKind::ArtifactSchema { artifact, schema_ref } => {
                if !wf.artifacts.contains_key(artifact) {
                    errs.push(ValidationError::new(loc, format!("gate '{g}' references unknown artifact '{artifact}'")));
                }
                if !ctx.schema_registry.iter().any(|s| s == schema_ref) {
                    errs.push(ValidationError::new(loc, format!("gate '{g}' schema_ref '{schema_ref}' not in registry")));
                }
            }
            GateKind::Approval { .. } => {}
        },
    }
}

/// Scan `${name}` references; every referenced variable must be declared.
fn collect_vars(s: &str, declared: &BTreeMap<String, VariableSpec>, errs: &mut Vec<ValidationError>, loc: &str) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'$' && bytes[i + 1] == b'{' {
            if let Some(end) = s[i + 2..].find('}') {
                let name = &s[i + 2..i + 2 + end];
                if !name.is_empty()
                    && name.chars().next().map_or(false, |c| c.is_ascii_alphabetic() || c == '_')
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !declared.contains_key(name)
                {
                    errs.push(ValidationError::new(loc, format!("undeclared variable reference '${{{name}}}'")));
                }
                i += 2 + end + 1;
                continue;
            }
        }
        i += 1;
    }
}