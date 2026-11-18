use serde_json::json;
use superagent_model_routing::{
    FixtureRun, ModelSpec, RouteRequest, Router, RoutingError, RoutingPolicy, TaskRule, UsageRecord,
};

fn catalog() -> Vec<ModelSpec> {
    vec![
        ModelSpec {
            slug: "fast-cheap".into(),
            tier: "fast".into(),
            input_cost_per_mtok: 1.0,
            output_cost_per_mtok: 2.0,
            latency_ms: 100.0,
        },
        ModelSpec {
            slug: "balanced".into(),
            tier: "balanced".into(),
            input_cost_per_mtok: 5.0,
            output_cost_per_mtok: 10.0,
            latency_ms: 300.0,
        },
        ModelSpec {
            slug: "frontier".into(),
            tier: "frontier".into(),
            input_cost_per_mtok: 20.0,
            output_cost_per_mtok: 60.0,
            latency_ms: 800.0,
        },
        ModelSpec {
            slug: "frontier-alt".into(),
            tier: "frontier".into(),
            input_cost_per_mtok: 25.0,
            output_cost_per_mtok: 70.0,
            latency_ms: 900.0,
        },
    ]
}

fn tier_order() -> Vec<String> {
    vec!["fast".into(), "balanced".into(), "frontier".into()]
}

fn task_routed() -> RoutingPolicy {
    let mut rules = std::collections::BTreeMap::new();
    rules.insert(
        "code_fix".into(),
        TaskRule {
            preferred_tier: "balanced".into(),
            max_tier: "frontier".into(),
        },
    );
    rules.insert(
        "summarize".into(),
        TaskRule {
            preferred_tier: "fast".into(),
            max_tier: "balanced".into(),
        },
    );
    rules.insert(
        "research".into(),
        TaskRule {
            preferred_tier: "frontier".into(),
            max_tier: "frontier".into(),
        },
    );
    RoutingPolicy {
        tier_order: tier_order(),
        rules,
        default_rule: TaskRule {
            preferred_tier: "balanced".into(),
            max_tier: "frontier".into(),
        },
        allowed_models: Default::default(),
        strict: false,
    }
}

fn always(tier: &str) -> RoutingPolicy {
    RoutingPolicy {
        tier_order: tier_order(),
        rules: Default::default(),
        default_rule: TaskRule {
            preferred_tier: tier.into(),
            max_tier: tier.into(),
        },
        allowed_models: Default::default(),
        strict: false,
    }
}

fn req(task: &str, override_slug: Option<&str>) -> RouteRequest {
    RouteRequest {
        task_class: task.into(),
        spawn_override: override_slug.map(|s| s.into()),
    }
}

#[test]
fn configurable_policy_audited() {
    let config = json!({ "catalog": catalog(), "policy": task_routed() });
    let mut r = Router::from_config(&config).unwrap();
    let a = r.route(&req("code_fix", None)).unwrap();
    assert_eq!(a.tier, "balanced");
    assert_eq!(a.slug, "balanced");
    assert!(!a.overridden);
    let b = r.route(&req("summarize", None)).unwrap();
    assert_eq!(b.tier, "fast");
    assert_eq!(r.audit().len(), 2);
    assert_eq!(r.audit()[0]["event"], "route");
    assert_eq!(r.audit()[0]["slug"], "balanced");
    assert_eq!(r.audit()[1]["tier"], "fast");
    assert!(r.audit()[0]["reason"].as_str().unwrap().contains("routed"));
}

#[test]
fn spawn_override_compat_and_ceiling() {
    let mut r = Router::new(catalog(), task_routed()).unwrap();
    // summarize ceiling = balanced; frontier override exceeds it.
    assert!(matches!(
        r.route(&req("summarize", Some("frontier"))),
        Err(RoutingError::OverrideExceedsLimit(_))
    ));
    // balanced override within ceiling is honored.
    let ok = r.route(&req("summarize", Some("balanced"))).unwrap();
    assert!(ok.overridden);
    assert_eq!(ok.slug, "balanced");
    // unknown model rejected.
    assert!(matches!(
        r.route(&req("code_fix", Some("ghost"))),
        Err(RoutingError::UnknownModel(_))
    ));
    // code_fix ceiling = frontier; frontier override honored, cheapest frontier picked when no override.
    let f = r.route(&req("code_fix", Some("frontier"))).unwrap();
    assert!(f.overridden);
    let auto = r.route(&req("research", None)).unwrap();
    assert_eq!(auto.slug, "frontier"); // cheaper than frontier-alt
}

#[test]
fn usage_recorded() {
    let mut r = Router::new(catalog(), task_routed()).unwrap();
    r.route(&req("code_fix", None)).unwrap();
    r.record_usage(UsageRecord {
        task_class: "code_fix".into(),
        slug: "balanced".into(),
        input_tokens: 1_000_000,
        output_tokens: 1_000_000,
        latency_ms: 300.0,
    })
    .unwrap();
    assert_eq!(r.usage().len(), 1);
    let usage_event = r.audit().iter().find(|e| e["event"] == "usage").unwrap();
    assert_eq!(usage_event["inputTokens"], 1_000_000);
    assert_eq!(usage_event["tier"], "balanced");
    // (1e6*5 + 1e6*10)/1e6 = 15.0
    assert!((usage_event["cost"].as_f64().unwrap() - 15.0).abs() < 1e-9);
    assert!((r.total_cost() - 15.0).abs() < 1e-9);
    assert!(matches!(
        r.record_usage(UsageRecord {
            task_class: "x".into(),
            slug: "ghost".into(),
            input_tokens: 1,
            output_tokens: 1,
            latency_ms: 1.0
        }),
        Err(RoutingError::UnknownModel(_))
    ));
}

#[test]
fn cost_latency_comparison_report() {
    let fixture = vec![
        FixtureRun {
            task_class: "code_fix".into(),
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
        },
        FixtureRun {
            task_class: "summarize".into(),
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
        },
        FixtureRun {
            task_class: "research".into(),
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
        },
    ];
    let policies = vec![
        ("always-frontier", always("frontier")),
        ("task-routed", task_routed()),
        ("always-fast", always("fast")),
    ];
    let report = Router::compare_policies(&catalog(), &fixture, &policies).unwrap();
    assert_eq!(report["fixtureRuns"], 3);
    let entries = report["policies"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    let cost = |name: &str| {
        entries.iter().find(|e| e["name"] == name).unwrap()["totalCost"]
            .as_f64()
            .unwrap()
    };
    let lat = |name: &str| {
        entries.iter().find(|e| e["name"] == name).unwrap()["totalLatencyMs"]
            .as_f64()
            .unwrap()
    };
    // always-frontier 3*(20+60)=240; always-fast 3*(1+2)=9; task-routed 15+3+80=98.
    assert!((cost("always-frontier") - 240.0).abs() < 1e-9);
    assert!((cost("always-fast") - 9.0).abs() < 1e-9);
    assert!((cost("task-routed") - 98.0).abs() < 1e-9);
    assert!(cost("task-routed") < cost("always-frontier"));
    assert!(cost("task-routed") > cost("always-fast"));
    assert!(lat("task-routed") < lat("always-frontier"));
    // identical fixture routed under each policy: perRun length matches.
    for e in entries {
        assert_eq!(e["perRun"].as_array().unwrap().len(), 3);
    }
}

#[test]
fn unknown_task_class_and_empty_config() {
    let mut strict = task_routed();
    strict.strict = true;
    let mut r = Router::new(catalog(), strict).unwrap();
    assert!(matches!(
        r.route(&req("mystery", None)),
        Err(RoutingError::UnknownTaskClass(_))
    ));
    // mapped class still routes under strict mode.
    assert!(r.route(&req("code_fix", None)).is_ok());
    assert!(matches!(
        Router::new(vec![], task_routed()),
        Err(RoutingError::EmptyConfig)
    ));
}

#[test]
fn routing_does_not_expand_limits() {
    let mut p = task_routed();
    p.allowed_models = ["fast-cheap".to_string(), "balanced".to_string()]
        .into_iter()
        .collect();
    let mut r = Router::new(catalog(), p).unwrap();
    // research prefers frontier but frontier is outside the admin allowed set:
    // routing falls back to the cheapest allowed model, never the excluded one.
    let d = r.route(&req("research", None)).unwrap();
    assert_ne!(d.slug, "frontier");
    assert_ne!(d.slug, "frontier-alt");
    assert_eq!(d.slug, "fast-cheap");
    // an explicit frontier override is rejected by the admin ceiling.
    assert!(matches!(
        r.route(&req("research", Some("frontier"))),
        Err(RoutingError::OverrideExceedsLimit(_))
    ));
    // an allowed override within ceiling still works.
    let ok = r.route(&req("code_fix", Some("balanced"))).unwrap();
    assert!(ok.overridden);
    assert_eq!(ok.slug, "balanced");
}
