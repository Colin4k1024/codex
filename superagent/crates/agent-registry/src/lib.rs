//! P5-2 / #96: agent capability registration / catalog / heartbeat service,
//! cross-host message routing over a board transport with session-level
//! credentials, Ed25519/JWKS identity verification, and manual cross-machine
//! placement (minimal: designate a host/environment at registration).
//!
//! Acceptance: two hosts' agents mutually discover and complete one handoff;
//! identity forgery is rejected. Proven deterministically over a fixture
//! in-memory board transport with two logical hosts and real Ed25519 crypto.
//! Model-free / process-free / network-free. The Ed25519/JWKS scheme mirrors
//! codex-agent-identity (which is workspace-tied and cannot be a path dep of a
//! standalone crate); the board transport is a fixture, not two physical hosts.

use std::collections::{BTreeMap, BTreeSet};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const ISSUER: &str = "superagent-agent-registry";
const AUDIENCE: &str = "superagent-board";

/// Errors surfaced by the registry / identity / routing boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    MalformedToken,
    UnknownKid,
    InvalidSignature,
    Expired,
    /// Token subject/host does not match the claimed agent identity.
    Forged,
    UnknownAgent,
    /// Agent has no live heartbeat within the TTL.
    StaleAgent,
    /// Session-level credential does not match the registered agent.
    SessionDenied,
}

/// JWT claims carried by an agent identity token (EdDSA / Ed25519).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Claims {
    pub iss: String,
    pub aud: String,
    pub sub: String,
    pub host: String,
    pub iat: i64,
    pub exp: i64,
}

/// A single JSON Web Key (OKP / Ed25519) published in the registry JWKS.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Jwk {
    pub kid: String,
    pub kty: String,
    pub crv: String,
    pub x: String,
}

/// An agent's durable signing identity. Each agent holds its own private key;
/// the registry only ever sees the public key (JWKS).
pub struct AgentIdentity {
    pub agent_id: String,
    pub host_id: String,
    signing: SigningKey,
}

impl AgentIdentity {
    /// Deterministic key from a 32-byte seed (fixture-friendly, no OsRng).
    pub fn from_seed(agent_id: &str, host_id: &str, seed: [u8; 32]) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            host_id: host_id.to_string(),
            signing: SigningKey::from_bytes(&seed),
        }
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }

    pub fn public_jwk(&self) -> Jwk {
        Jwk {
            kid: self.agent_id.clone(),
            kty: "OKP".to_string(),
            crv: "Ed25519".to_string(),
            x: URL_SAFE_NO_PAD.encode(self.public_key()),
        }
    }

    /// Issue an EdDSA JWT bound to this agent_id + host_id.
    pub fn issue_token(&self, now: i64, ttl: i64) -> String {
        let header = json!({"alg":"EdDSA","typ":"JWT","kid":self.agent_id});
        let claims = Claims {
            iss: ISSUER.to_string(),
            aud: AUDIENCE.to_string(),
            sub: self.agent_id.clone(),
            host: self.host_id.clone(),
            iat: now,
            exp: now + ttl,
        };
        let h = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
        let c = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        let signing_input = format!("{h}.{c}");
        let sig: Signature = self.signing.sign(signing_input.as_bytes());
        let s = URL_SAFE_NO_PAD.encode(sig.to_bytes());
        format!("{signing_input}.{s}")
    }
}

/// Registration request: capability inventory + endpoint + manual host placement.
#[derive(Clone, Debug)]
pub struct Registration {
    pub agent_id: String,
    pub host_id: String,
    pub capabilities: BTreeSet<String>,
    pub endpoint: String,
    pub public_key: [u8; 32],
    pub session_credential: String,
}

/// A registered agent record as exposed by the catalog / discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRecord {
    pub agent_id: String,
    pub host_id: String,
    pub capabilities: BTreeSet<String>,
    pub endpoint: String,
    pub last_seen: i64,
}

/// A cross-host board envelope routed between agents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub from: String,
    pub to: String,
    pub from_host: String,
    pub to_host: String,
    pub kind: EnvelopeKind,
    pub payload: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvelopeKind {
    Handoff,
    HandoffAck,
}

/// Routing intent: envelope kind plus payload, grouped to keep the routing
/// boundary within a sane argument count.
struct Message<'a> {
    kind: EnvelopeKind,
    payload: &'a str,
}

/// The registration / discovery / routing service over a fixture board.
pub struct AgentRegistry {
    agents: BTreeMap<String, AgentRecord>,
    jwks: BTreeMap<String, VerifyingKey>,
    sessions: BTreeMap<String, String>,
    board: BTreeMap<String, Vec<Envelope>>,
    heartbeat_ttl: i64,
    audit: Vec<Value>,
}

impl AgentRegistry {
    pub fn new(heartbeat_ttl: i64) -> Self {
        Self {
            agents: BTreeMap::new(),
            jwks: BTreeMap::new(),
            sessions: BTreeMap::new(),
            board: BTreeMap::new(),
            heartbeat_ttl,
            audit: Vec::new(),
        }
    }

    pub fn audit(&self) -> &[Value] {
        &self.audit
    }

    pub fn jwks(&self) -> Vec<Jwk> {
        self.jwks
            .iter()
            .map(|(kid, key)| Jwk {
                kid: kid.clone(),
                kty: "OKP".to_string(),
                crv: "Ed25519".to_string(),
                x: URL_SAFE_NO_PAD.encode(key.to_bytes()),
            })
            .collect()
    }

    /// Verify an EdDSA JWT against the JWKS: signature, expiry, issuer/audience.
    fn verify(&self, token: &str, now: i64) -> Result<Claims, RegistryError> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(RegistryError::MalformedToken);
        }
        let header_bytes = URL_SAFE_NO_PAD
            .decode(parts[0])
            .map_err(|_| RegistryError::MalformedToken)?;
        let header: Value =
            serde_json::from_slice(&header_bytes).map_err(|_| RegistryError::MalformedToken)?;
        let kid = header["kid"]
            .as_str()
            .ok_or(RegistryError::MalformedToken)?;
        let key = self.jwks.get(kid).ok_or(RegistryError::UnknownKid)?;
        let sig_bytes = URL_SAFE_NO_PAD
            .decode(parts[2])
            .map_err(|_| RegistryError::MalformedToken)?;
        let sig_arr: [u8; 64] = sig_bytes
            .try_into()
            .map_err(|_| RegistryError::MalformedToken)?;
        let sig = Signature::from_bytes(&sig_arr);
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        key.verify(signing_input.as_bytes(), &sig)
            .map_err(|_| RegistryError::InvalidSignature)?;
        let claim_bytes = URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| RegistryError::MalformedToken)?;
        let claims: Claims =
            serde_json::from_slice(&claim_bytes).map_err(|_| RegistryError::MalformedToken)?;
        if claims.iss != ISSUER || claims.aud != AUDIENCE {
            return Err(RegistryError::Forged);
        }
        if claims.exp <= now {
            return Err(RegistryError::Expired);
        }
        Ok(claims)
    }

    /// Register an agent: verify its identity token, publish its public key to
    /// the JWKS, record capabilities/endpoint and manual host placement.
    pub fn register(
        &mut self,
        reg: Registration,
        token: &str,
        now: i64,
    ) -> Result<(), RegistryError> {
        let key = VerifyingKey::from_bytes(&reg.public_key)
            .map_err(|_| RegistryError::InvalidSignature)?;
        // A failed re-registration must not revoke an already-published identity,
        // so only roll back the JWKS key when this registration newly added it.
        let newly = !self.jwks.contains_key(&reg.agent_id);
        self.jwks.insert(reg.agent_id.clone(), key);
        let verified = self.verify(token, now).and_then(|claims| {
            if claims.sub != reg.agent_id || claims.host != reg.host_id {
                Err(RegistryError::Forged)
            } else {
                Ok(claims)
            }
        });
        if let Err(e) = verified {
            if newly {
                self.jwks.remove(&reg.agent_id);
            }
            return Err(e);
        }
        self.agents.insert(
            reg.agent_id.clone(),
            AgentRecord {
                agent_id: reg.agent_id.clone(),
                host_id: reg.host_id.clone(),
                capabilities: reg.capabilities,
                endpoint: reg.endpoint,
                last_seen: now,
            },
        );
        self.sessions
            .insert(reg.agent_id.clone(), reg.session_credential);
        self.board.entry(reg.agent_id.clone()).or_default();
        self.audit.push(json!({
            "event": "agent_registered",
            "agent": reg.agent_id,
            "host": reg.host_id,
        }));
        Ok(())
    }

    /// Refresh liveness. Requires a valid identity token for the agent.
    pub fn heartbeat(
        &mut self,
        agent_id: &str,
        token: &str,
        now: i64,
    ) -> Result<(), RegistryError> {
        let claims = self.verify(token, now)?;
        if claims.sub != agent_id {
            return Err(RegistryError::Forged);
        }
        let rec = self
            .agents
            .get_mut(agent_id)
            .ok_or(RegistryError::UnknownAgent)?;
        rec.last_seen = now;
        self.audit
            .push(json!({"event":"heartbeat","agent":agent_id}));
        Ok(())
    }

    fn live(&self, rec: &AgentRecord, now: i64) -> bool {
        now - rec.last_seen <= self.heartbeat_ttl
    }

    /// Full catalog of live agents (capability discovery feed).
    pub fn catalog(&self, now: i64) -> Vec<AgentRecord> {
        self.agents
            .values()
            .filter(|r| self.live(r, now))
            .cloned()
            .collect()
    }

    /// Discover live agents on OTHER hosts — cross-host mutual discovery.
    pub fn discover(&self, host_id: &str, now: i64) -> Vec<AgentRecord> {
        self.agents
            .values()
            .filter(|r| r.host_id != host_id && self.live(r, now))
            .cloned()
            .collect()
    }

    /// Manual placement policy: the host/environment an agent is designated to.
    pub fn placement(&self, agent_id: &str) -> Result<String, RegistryError> {
        self.agents
            .get(agent_id)
            .map(|r| r.host_id.clone())
            .ok_or(RegistryError::UnknownAgent)
    }

    fn route(
        &mut self,
        from: &str,
        to: &str,
        msg: Message<'_>,
        token: &str,
        session: &str,
        now: i64,
    ) -> Result<(), RegistryError> {
        let claims = self.verify(token, now)?;
        if claims.sub != from {
            return Err(RegistryError::Forged);
        }
        let sender = self.agents.get(from).ok_or(RegistryError::UnknownAgent)?;
        if !self.live(sender, now) {
            return Err(RegistryError::StaleAgent);
        }
        // Session-level credential bound to the sending agent.
        if self.sessions.get(from).map(String::as_str) != Some(session) {
            return Err(RegistryError::SessionDenied);
        }
        let recipient = self.agents.get(to).ok_or(RegistryError::UnknownAgent)?;
        if !self.live(recipient, now) {
            return Err(RegistryError::StaleAgent);
        }
        let env = Envelope {
            from: from.to_string(),
            to: to.to_string(),
            from_host: sender.host_id.clone(),
            to_host: recipient.host_id.clone(),
            kind: msg.kind,
            payload: msg.payload.to_string(),
        };
        self.board.entry(to.to_string()).or_default().push(env);
        self.audit.push(json!({
            "event": "board_routed",
            "from": from,
            "to": to,
            "kind": format!("{:?}", msg.kind),
        }));
        Ok(())
    }

    /// Send a handoff to a discovered peer over the cross-host board.
    pub fn send_handoff(
        &mut self,
        from: &str,
        to: &str,
        payload: &str,
        token: &str,
        session: &str,
        now: i64,
    ) -> Result<(), RegistryError> {
        self.route(
            from,
            to,
            Message {
                kind: EnvelopeKind::Handoff,
                payload,
            },
            token,
            session,
            now,
        )
    }

    /// Acknowledge a received handoff back to the originator.
    pub fn ack_handoff(
        &mut self,
        from: &str,
        to: &str,
        token: &str,
        session: &str,
        now: i64,
    ) -> Result<(), RegistryError> {
        self.route(
            from,
            to,
            Message {
                kind: EnvelopeKind::HandoffAck,
                payload: "ack",
            },
            token,
            session,
            now,
        )
    }

    /// Drain the recipient inbox. Requires the recipient's valid identity token.
    pub fn recv(
        &mut self,
        agent_id: &str,
        token: &str,
        now: i64,
    ) -> Result<Vec<Envelope>, RegistryError> {
        let claims = self.verify(token, now)?;
        if claims.sub != agent_id {
            return Err(RegistryError::Forged);
        }
        if !self.agents.contains_key(agent_id) {
            return Err(RegistryError::UnknownAgent);
        }
        let inbox = self
            .board
            .get_mut(agent_id)
            .ok_or(RegistryError::UnknownAgent)?;
        let drained = std::mem::take(inbox);
        if !drained.is_empty() {
            self.audit
                .push(json!({"event":"handoff_received","agent":agent_id,"count":drained.len()}));
        }
        Ok(drained)
    }
}
