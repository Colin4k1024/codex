//! SuperAgent model-routing (P5-3 / #97).
//!
//! Product-side task-class -> model-tier routing service. Consumes a
//! model-provider-shaped catalog (slug / tier / cost / latency) plus a
//! configurable routing policy, records every selection and usage event for
//! audit, and produces a same-fixture cost/latency comparison across policies.
//! Spawn-level model overrides are honored but never expand the user/admin
//! ceiling. Model-free: no network, no model turn, no subprocess — pure policy
//! computation over committed fixtures.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpec {
    pub slug: String,
    pub tier: String,
    pub input_cost_per_mtok: f64,
    pub output_cost_per_mtok: f64,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRule {
    pub preferred_tier: String,
    pub max_tier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingPolicy {
    /// Ascending capability/cost order, e.g. ["fast", "balanced", "frontier"].
    pub tier_order: Vec<String>,
    pub rules: BTreeMap<String, TaskRule>,
    pub default_rule: TaskRule,
    /// Admin ceiling on selectable models; empty means the whole catalog.
    pub allowed_models: BTreeSet<String>,
    /// When true, an unmapped task class is rejected instead of using default.
    #[serde(default)]
    pub strict: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRequest {
    pub task_class: String,
    pub spawn_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub slug: String,
    pub tier: String,
    pub overridden: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub task_class: String,
    pub slug: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureRun {
    pub task_class: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug)]
pub enum RoutingError {
    EmptyConfig,
    UnknownModel(String),
    UnknownTaskClass(String),
    OverrideExceedsLimit(String),
    NoModelForTier(String),
    Config(String),
}

impl std::fmt::Display for RoutingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoutingError::EmptyConfig => write!(f, "model catalog is empty"),
            RoutingError::UnknownModel(s) => write!(f, "unknown model slug: {s}"),
            RoutingError::UnknownTaskClass(s) => write!(f, "unmapped task class (strict): {s}"),
            RoutingError::OverrideExceedsLimit(s) => {
                write!(f, "spawn override exceeds user/admin ceiling: {s}")
            }
            RoutingError::NoModelForTier(s) => write!(f, "no selectable model for tier: {s}"),
            RoutingError::Config(s) => write!(f, "invalid routing config: {s}"),
        }
    }
}

impl std::error::Error for RoutingError {}

pub type Result<T> = std::result::Result<T, RoutingError>;

fn run_cost(m: &ModelSpec, input: u64, output: u64) -> f64 {
    (input as f64 * m.input_cost_per_mtok + output as f64 * m.output_cost_per_mtok) / 1_000_000.0
}

pub struct Router {
    catalog: Vec<ModelSpec>,
    policy: RoutingPolicy,
    audit: Vec<Value>,
    usage: Vec<UsageRecord>,
}

impl Router {
    pub fn new(catalog: Vec<ModelSpec>, policy: RoutingPolicy) -> Result<Self> {
        if catalog.is_empty() {
            return Err(RoutingError::EmptyConfig);
        }
        if policy.tier_order.is_empty() {
            return Err(RoutingError::Config("tier_order is empty".into()));
        }
        Ok(Self {
            catalog,
            policy,
            audit: Vec::new(),
            usage: Vec::new(),
        })
    }

    /// Consume a model-provider-shaped config: {catalog:[ModelSpec], policy:{...}}.
    pub fn from_config(v: &Value) -> Result<Self> {
        let catalog: Vec<ModelSpec> = serde_json::from_value(v["catalog"].clone())
            .map_err(|e| RoutingError::Config(format!("catalog: {e}")))?;
        let policy: RoutingPolicy = serde_json::from_value(v["policy"].clone())
            .map_err(|e| RoutingError::Config(format!("policy: {e}")))?;
        Self::new(catalog, policy)
    }

    pub fn audit(&self) -> &[Value] {
        &self.audit
    }

    pub fn usage(&self) -> &[UsageRecord] {
        &self.usage
    }

    fn tier_rank(&self, tier: &str) -> Option<usize> {
        self.policy.tier_order.iter().position(|t| t == tier)
    }

    fn model(&self, slug: &str) -> Option<&ModelSpec> {
        self.catalog.iter().find(|m| m.slug == slug)
    }

    fn allowed(&self, slug: &str) -> bool {
        self.policy.allowed_models.is_empty() || self.policy.allowed_models.contains(slug)
    }

    fn rule_for(&self, task_class: &str) -> Result<&TaskRule> {
        match self.policy.rules.get(task_class) {
            Some(r) => Ok(r),
            None if self.policy.strict => {
                Err(RoutingError::UnknownTaskClass(task_class.to_string()))
            }
            None => Ok(&self.policy.default_rule),
        }
    }

    /// Route a task. A spawn override is honored only within the admin allowed
    /// set AND the task's tier ceiling — routing never expands limits.
    pub fn route(&mut self, req: &RouteRequest) -> Result<RouteDecision> {
        let rule = self.rule_for(&req.task_class)?.clone();
        let max_rank = self
            .tier_rank(&rule.max_tier)
            .ok_or_else(|| RoutingError::NoModelForTier(rule.max_tier.clone()))?;

        let decision = if let Some(slug) = &req.spawn_override {
            let m = self
                .model(slug)
                .ok_or_else(|| RoutingError::UnknownModel(slug.clone()))?;
            if !self.allowed(slug) {
                return Err(RoutingError::OverrideExceedsLimit(format!(
                    "{slug} not in admin allowed set"
                )));
            }
            let rank = self
                .tier_rank(&m.tier)
                .ok_or_else(|| RoutingError::NoModelForTier(m.tier.clone()))?;
            if rank > max_rank {
                return Err(RoutingError::OverrideExceedsLimit(format!(
                    "{slug} tier {} above task ceiling {}",
                    m.tier, rule.max_tier
                )));
            }
            RouteDecision {
                slug: m.slug.clone(),
                tier: m.tier.clone(),
                overridden: true,
                reason: "spawn override within ceiling".to_string(),
            }
        } else {
            let chosen = self
                .catalog
                .iter()
                .filter(|m| self.allowed(&m.slug))
                .filter(|m| self.tier_rank(&m.tier).is_some_and(|r| r <= max_rank))
                .filter(|m| m.tier == rule.preferred_tier)
                .min_by(|a, b| {
                    (a.input_cost_per_mtok + a.output_cost_per_mtok)
                        .total_cmp(&(b.input_cost_per_mtok + b.output_cost_per_mtok))
                })
                .or_else(|| {
                    self.catalog
                        .iter()
                        .filter(|m| self.allowed(&m.slug))
                        .filter(|m| self.tier_rank(&m.tier).is_some_and(|r| r <= max_rank))
                        .min_by(|a, b| {
                            (a.input_cost_per_mtok + a.output_cost_per_mtok)
                                .total_cmp(&(b.input_cost_per_mtok + b.output_cost_per_mtok))
                        })
                })
                .ok_or_else(|| RoutingError::NoModelForTier(rule.preferred_tier.clone()))?;
            RouteDecision {
                slug: chosen.slug.clone(),
                tier: chosen.tier.clone(),
                overridden: false,
                reason: format!("task-class routed to tier {}", chosen.tier),
            }
        };

        self.audit.push(json!({
            "event": "route",
            "taskClass": req.task_class,
            "slug": decision.slug,
            "tier": decision.tier,
            "overridden": decision.overridden,
            "reason": decision.reason,
        }));
        Ok(decision)
    }

    /// Record the actual model selection and token usage for audit/cost.
    pub fn record_usage(&mut self, rec: UsageRecord) -> Result<()> {
        let m = self
            .model(&rec.slug)
            .ok_or_else(|| RoutingError::UnknownModel(rec.slug.clone()))?;
        let cost = run_cost(m, rec.input_tokens, rec.output_tokens);
        self.audit.push(json!({
            "event": "usage",
            "taskClass": rec.task_class,
            "slug": rec.slug,
            "tier": m.tier,
            "inputTokens": rec.input_tokens,
            "outputTokens": rec.output_tokens,
            "latencyMs": rec.latency_ms,
            "cost": cost,
        }));
        self.usage.push(rec);
        Ok(())
    }

    pub fn total_cost(&self) -> f64 {
        self.usage
            .iter()
            .filter_map(|u| {
                self.model(&u.slug)
                    .map(|m| run_cost(m, u.input_tokens, u.output_tokens))
            })
            .sum()
    }

    /// Same-fixture cost/latency comparison across named routing policies.
    /// Each policy routes the identical fixture runs; totals are computed from
    /// the catalog cost/latency profile, so the comparison is deterministic.
    pub fn compare_policies(
        catalog: &[ModelSpec],
        fixture: &[FixtureRun],
        policies: &[(&str, RoutingPolicy)],
    ) -> Result<Value> {
        let mut entries = Vec::new();
        for (name, policy) in policies {
            let mut router = Router::new(catalog.to_vec(), policy.clone())?;
            let mut total_cost = 0.0;
            let mut total_latency = 0.0;
            let mut per_run = Vec::new();
            for run in fixture {
                let decision = router.route(&RouteRequest {
                    task_class: run.task_class.clone(),
                    spawn_override: None,
                })?;
                let m = router
                    .model(&decision.slug)
                    .ok_or_else(|| RoutingError::UnknownModel(decision.slug.clone()))?;
                let cost = run_cost(m, run.input_tokens, run.output_tokens);
                total_cost += cost;
                total_latency += m.latency_ms;
                per_run.push(json!({
                    "taskClass": run.task_class,
                    "slug": decision.slug,
                    "tier": decision.tier,
                    "cost": cost,
                    "latencyMs": m.latency_ms,
                }));
            }
            entries.push(json!({
                "name": name,
                "totalCost": total_cost,
                "totalLatencyMs": total_latency,
                "perRun": per_run,
            }));
        }
        Ok(json!({ "fixtureRuns": fixture.len(), "policies": entries }))
    }
}
