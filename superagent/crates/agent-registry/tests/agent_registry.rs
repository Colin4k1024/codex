//! P5-2 / #96 acceptance tests: two hosts' agents mutually discover and complete
//! one handoff over the cross-host board; identity forgery is rejected;
//! heartbeat liveness, session-level routing credentials, manual placement, and
//! JWKS verification. Model-free / process-free / network-free over a fixture
//! in-memory board transport with real Ed25519/JWKS crypto.

use std::collections::BTreeSet;

use superagent_agent_registry::{
    AgentIdentity, AgentRegistry, EnvelopeKind, Registration, RegistryError,
};

const NOW: i64 = 1_700_000_000;
const HB_TTL: i64 = 300;
const TOKEN_TTL: i64 = 3600;

fn identity(id: &str, host: &str, seed_byte: u8) -> AgentIdentity {
    AgentIdentity::from_seed(id, host, [seed_byte; 32])
}

fn caps(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn register(
    reg: &mut AgentRegistry,
    id: &AgentIdentity,
    capabilities: BTreeSet<String>,
    sess: &str,
) {
    let r = Registration {
        agent_id: id.agent_id.clone(),
        host_id: id.host_id.clone(),
        capabilities,
        endpoint: format!("board://{}", id.agent_id),
        public_key: id.public_key(),
        session_credential: sess.to_string(),
    };
    let tok = id.issue_token(NOW, TOKEN_TTL);
    reg.register(r, &tok, NOW).unwrap();
}

/// Two logical hosts, each with one registered agent.
fn two_hosts() -> (AgentRegistry, AgentIdentity, AgentIdentity) {
    let mut reg = AgentRegistry::new(HB_TTL);
    let a = identity("agent-a", "host-1", 1);
    let b = identity("agent-b", "host-2", 2);
    register(&mut reg, &a, caps(&["build"]), "sess-a");
    register(&mut reg, &b, caps(&["test"]), "sess-b");
    (reg, a, b)
}

#[test]
fn two_hosts_mutual_discovery_and_handoff() {
    let (mut reg, a, b) = two_hosts();

    // Mutual cross-host discovery: each host sees the other's agent.
    let from_1 = reg.discover("host-1", NOW);
    assert_eq!(from_1.len(), 1);
    assert_eq!(from_1[0].agent_id, "agent-b");
    assert_eq!(from_1[0].host_id, "host-2");
    let from_2 = reg.discover("host-2", NOW);
    assert_eq!(from_2.len(), 1);
    assert_eq!(from_2[0].agent_id, "agent-a");
    assert_eq!(reg.catalog(NOW).len(), 2);

    // A -> B handoff over the cross-host board.
    let tok_a = a.issue_token(NOW, TOKEN_TTL);
    let tok_b = b.issue_token(NOW, TOKEN_TTL);
    reg.send_handoff("agent-a", "agent-b", "task-42", &tok_a, "sess-a", NOW)
        .unwrap();
    let inbox_b = reg.recv("agent-b", &tok_b, NOW).unwrap();
    assert_eq!(inbox_b.len(), 1);
    assert_eq!(inbox_b[0].from, "agent-a");
    assert_eq!(inbox_b[0].to, "agent-b");
    assert_eq!(inbox_b[0].from_host, "host-1");
    assert_eq!(inbox_b[0].to_host, "host-2");
    assert_eq!(inbox_b[0].kind, EnvelopeKind::Handoff);
    assert_eq!(inbox_b[0].payload, "task-42");

    // B acknowledges back to A: one handoff completed end to end.
    reg.ack_handoff("agent-b", "agent-a", &tok_b, "sess-b", NOW)
        .unwrap();
    let inbox_a = reg.recv("agent-a", &tok_a, NOW).unwrap();
    assert_eq!(inbox_a.len(), 1);
    assert_eq!(inbox_a[0].kind, EnvelopeKind::HandoffAck);
    assert_eq!(inbox_a[0].from, "agent-b");

    // Audit chain proves registration + routing + receipt.
    let audit = reg.audit();
    assert_eq!(
        audit
            .iter()
            .filter(|e| e["event"] == "agent_registered")
            .count(),
        2
    );
    assert_eq!(
        audit
            .iter()
            .filter(|e| e["event"] == "board_routed")
            .count(),
        2
    );
    assert_eq!(
        audit
            .iter()
            .filter(|e| e["event"] == "handoff_received")
            .count(),
        2
    );
}

#[test]
fn identity_forgery_rejected() {
    let (mut reg, a, b) = two_hosts();
    let tok_b = b.issue_token(NOW, TOKEN_TTL);

    // Wrong signing key claiming agent-a's identity: signature fails against JWKS.
    let impostor = identity("agent-a", "host-1", 9);
    let forged = impostor.issue_token(NOW, TOKEN_TTL);
    assert_eq!(
        reg.send_handoff("agent-a", "agent-b", "evil", &forged, "sess-a", NOW)
            .unwrap_err(),
        RegistryError::InvalidSignature
    );

    // Tampered signature segment.
    let good = a.issue_token(NOW, TOKEN_TTL);
    let mut parts: Vec<String> = good.split('.').map(String::from).collect();
    parts[2] =
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            .to_string();
    let tampered = parts.join(".");
    assert_eq!(
        reg.heartbeat("agent-a", &tampered, NOW).unwrap_err(),
        RegistryError::InvalidSignature
    );

    // Expired token rejected.
    let expired = a.issue_token(NOW - 7200, TOKEN_TTL);
    assert_eq!(
        reg.heartbeat("agent-a", &expired, NOW).unwrap_err(),
        RegistryError::Expired
    );

    // Valid signature but host claim mismatches the registered placement: Forged.
    let evil_host = identity("agent-a", "host-evil", 1);
    let r = Registration {
        agent_id: "agent-a".into(),
        host_id: "host-1".into(),
        capabilities: caps(&["build"]),
        endpoint: "board://agent-a".into(),
        public_key: evil_host.public_key(),
        session_credential: "sess-a".into(),
    };
    assert_eq!(
        reg.register(r, &evil_host.issue_token(NOW, TOKEN_TTL), NOW)
            .unwrap_err(),
        RegistryError::Forged
    );

    // Recipient inbox cannot be drained with another agent's token.
    assert_eq!(
        reg.recv("agent-b", &a.issue_token(NOW, TOKEN_TTL), NOW)
            .unwrap_err(),
        RegistryError::Forged
    );

    // No handoff was ever delivered by the forged attempts.
    assert!(reg.recv("agent-b", &tok_b, NOW).unwrap().is_empty());
}

#[test]
fn heartbeat_liveness_and_stale_exclusion() {
    let (mut reg, a, b) = two_hosts();
    assert_eq!(reg.discover("host-1", NOW).len(), 1);

    // Past the heartbeat TTL with no refresh: agents drop out of discovery.
    let later = NOW + HB_TTL + 100;
    assert!(reg.catalog(later).is_empty());
    assert!(reg.discover("host-1", later).is_empty());

    // A fresh heartbeat restores liveness.
    let tok_b = b.issue_token(later, TOKEN_TTL);
    reg.heartbeat("agent-b", &tok_b, later).unwrap();
    let live = reg.discover("host-1", later);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].agent_id, "agent-b");
    assert_eq!(live[0].last_seen, later);

    // A token whose kid is not in the JWKS is UnknownKid.
    let other = identity("agent-x", "host-2", 7);
    assert_eq!(
        reg.heartbeat("agent-b", &other.issue_token(later, TOKEN_TTL), later)
            .unwrap_err(),
        RegistryError::UnknownKid
    );

    // A registered kid whose sub differs from the target agent hits the
    // heartbeat sub-binding branch: Forged (agent-a's token on agent-b).
    assert_eq!(
        reg.heartbeat("agent-b", &a.issue_token(later, TOKEN_TTL), later)
            .unwrap_err(),
        RegistryError::Forged
    );
}

#[test]
fn reregistration_identity_replacement_rejected() {
    let (mut reg, a, b) = two_hosts();
    let tok_a = a.issue_token(NOW, TOKEN_TTL);
    let tok_b = b.issue_token(NOW, TOKEN_TTL);

    // Attacker re-registers the live agent-a with a DIFFERENT signing key and a
    // self-signed token whose sub/host match: identity replacement must be Forged.
    let attacker = identity("agent-a", "host-1", 9);
    let r = Registration {
        agent_id: "agent-a".into(),
        host_id: "host-1".into(),
        capabilities: caps(&["build"]),
        endpoint: "board://agent-a".into(),
        public_key: attacker.public_key(),
        session_credential: "sess-evil".into(),
    };
    assert_eq!(
        reg.register(r, &attacker.issue_token(NOW, TOKEN_TTL), NOW)
            .unwrap_err(),
        RegistryError::Forged
    );

    // The published JWKS key is unchanged: the real agent still verifies and the
    // attacker's key is still rejected.
    reg.heartbeat("agent-a", &tok_a, NOW).unwrap();
    assert_eq!(
        reg.heartbeat("agent-a", &attacker.issue_token(NOW, TOKEN_TTL), NOW)
            .unwrap_err(),
        RegistryError::InvalidSignature
    );

    // The session credential is unchanged: sess-a routes, sess-evil is denied.
    reg.send_handoff("agent-a", "agent-b", "task", &tok_a, "sess-a", NOW)
        .unwrap();
    assert_eq!(
        reg.send_handoff("agent-a", "agent-b", "task", &tok_a, "sess-evil", NOW)
            .unwrap_err(),
        RegistryError::SessionDenied
    );

    // No forged delivery: agent-b receives only the legitimate handoff.
    let inbox = reg.recv("agent-b", &tok_b, NOW).unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].payload, "task");
    assert_eq!(inbox[0].from, "agent-a");
}

#[test]
fn cross_host_routing_session_credentials() {
    let (mut reg, a, _b) = two_hosts();
    let tok_a = a.issue_token(NOW, TOKEN_TTL);

    // Wrong session-level credential is refused even with a valid identity token.
    assert_eq!(
        reg.send_handoff("agent-a", "agent-b", "task", &tok_a, "sess-wrong", NOW)
            .unwrap_err(),
        RegistryError::SessionDenied
    );

    // Correct credential routes; routing to an unknown peer is refused.
    reg.send_handoff("agent-a", "agent-b", "task", &tok_a, "sess-a", NOW)
        .unwrap();
    assert_eq!(
        reg.send_handoff("agent-a", "ghost", "task", &tok_a, "sess-a", NOW)
            .unwrap_err(),
        RegistryError::UnknownAgent
    );
}

#[test]
fn manual_placement_policy() {
    let (reg, _a, _b) = two_hosts();
    assert_eq!(reg.placement("agent-a").unwrap(), "host-1");
    assert_eq!(reg.placement("agent-b").unwrap(), "host-2");
    assert_eq!(
        reg.placement("ghost").unwrap_err(),
        RegistryError::UnknownAgent
    );
}

#[test]
fn jwks_verification_and_unknown_kid() {
    let (mut reg, a, _b) = two_hosts();

    // JWKS publishes one OKP/Ed25519 key per registered agent.
    let jwks = reg.jwks();
    assert_eq!(jwks.len(), 2);
    assert!(jwks.iter().all(|k| k.kty == "OKP" && k.crv == "Ed25519"));
    assert!(jwks.iter().any(|k| k.kid == "agent-a"));

    // A token whose kid is not in the JWKS is rejected.
    let ghost = identity("ghost", "host-9", 3);
    assert_eq!(
        reg.heartbeat("ghost", &ghost.issue_token(NOW, TOKEN_TTL), NOW)
            .unwrap_err(),
        RegistryError::UnknownKid
    );

    // Malformed token rejected.
    assert_eq!(
        reg.heartbeat("agent-a", "garbage", NOW).unwrap_err(),
        RegistryError::MalformedToken
    );

    // The legitimate agent still verifies against its published key.
    reg.heartbeat("agent-a", &a.issue_token(NOW, TOKEN_TTL), NOW)
        .unwrap();
}
