//! SuperAgent IAM/RBAC (P4-2 / #91).
//!
//! Gateway-owned identity and access management, independent of the engine's
//! execution-layer sandbox/permissions (the two layers do not replace each
//! other). HS256 JWT API authentication, user/role/policy tables, resource-level
//! authorization (tenant / thread / run / artifact) with a hard tenant boundary,
//! channel-identity -> IAM-user mapping, and credential hygiene: audit records
//! never carry tokens or secrets, and a leak scanner verifies redaction.
//! Model-free: pure crypto (hmac/sha2) + policy computation, no network/model.

use std::collections::{BTreeMap, BTreeSet};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Tenant,
    Thread,
    Run,
    Artifact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Read,
    Write,
    Delete,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRef {
    pub kind: ResourceKind,
    pub tenant: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub user_id: String,
    pub role: String,
    pub tenant: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    pub tenant: String,
    pub exp: i64,
}

#[derive(Debug)]
pub enum IamError {
    UnknownUser(String),
    UnknownIdentity(String),
    MalformedToken,
    InvalidSignature,
    Expired,
    CrossTenant(String),
    Forbidden(String),
}

impl std::fmt::Display for IamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IamError::UnknownUser(s) => write!(f, "unknown user: {s}"),
            IamError::UnknownIdentity(s) => write!(f, "unmapped channel identity: {s}"),
            IamError::MalformedToken => write!(f, "malformed JWT"),
            IamError::InvalidSignature => write!(f, "invalid JWT signature"),
            IamError::Expired => write!(f, "JWT expired"),
            IamError::CrossTenant(s) => write!(f, "cross-tenant access denied: {s}"),
            IamError::Forbidden(s) => write!(f, "role lacks permission: {s}"),
        }
    }
}

impl std::error::Error for IamError {}

pub type Result<T> = std::result::Result<T, IamError>;

fn b64_encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn b64_decode(s: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| IamError::MalformedToken)
}

fn sign(secret: &str, signing_input: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac accepts any key len");
    mac.update(signing_input.as_bytes());
    b64_encode(&mac.finalize().into_bytes())
}

/// Replace any occurrence of the given secrets with a redaction marker.
pub fn redact(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for s in secrets {
        if !s.is_empty() {
            out = out.replace(s, "[REDACTED]");
        }
    }
    out
}

/// Return the secrets that leak into `text` (empty means clean).
pub fn scan_for_secrets(text: &str, secrets: &[&str]) -> Vec<String> {
    secrets
        .iter()
        .filter(|s| !s.is_empty() && text.contains(*s))
        .map(|s| s.to_string())
        .collect()
}

pub struct IamServer {
    secret: String,
    users: BTreeMap<String, User>,
    bindings: BTreeMap<String, String>,
    policies: BTreeMap<String, BTreeSet<(ResourceKind, Action)>>,
    audit: Vec<Value>,
}

impl IamServer {
    pub fn new(secret: &str) -> Self {
        let mut policies: BTreeMap<String, BTreeSet<(ResourceKind, Action)>> = BTreeMap::new();
        let all_kinds = [
            ResourceKind::Tenant,
            ResourceKind::Thread,
            ResourceKind::Run,
            ResourceKind::Artifact,
        ];
        let all_actions = [Action::Read, Action::Write, Action::Delete, Action::Admin];
        let mut admin = BTreeSet::new();
        for k in all_kinds {
            for a in all_actions {
                admin.insert((k, a));
            }
        }
        policies.insert("admin".into(), admin);
        let mut member = BTreeSet::new();
        for k in [
            ResourceKind::Thread,
            ResourceKind::Run,
            ResourceKind::Artifact,
        ] {
            member.insert((k, Action::Read));
            member.insert((k, Action::Write));
        }
        member.insert((ResourceKind::Tenant, Action::Read));
        policies.insert("member".into(), member);
        let mut viewer = BTreeSet::new();
        for k in all_kinds {
            viewer.insert((k, Action::Read));
        }
        policies.insert("viewer".into(), viewer);
        Self {
            secret: secret.to_string(),
            users: BTreeMap::new(),
            bindings: BTreeMap::new(),
            policies,
            audit: Vec::new(),
        }
    }

    pub fn add_user(&mut self, user_id: &str, role: &str, tenant: &str) {
        self.users.insert(
            user_id.into(),
            User {
                user_id: user_id.into(),
                role: role.into(),
                tenant: tenant.into(),
            },
        );
    }

    pub fn bind_channel(&mut self, channel: &str, external_id: &str, user_id: &str) -> Result<()> {
        if !self.users.contains_key(user_id) {
            return Err(IamError::UnknownUser(user_id.into()));
        }
        self.bindings
            .insert(format!("{channel}:{external_id}"), user_id.into());
        Ok(())
    }

    pub fn map_channel_identity(&self, channel: &str, external_id: &str) -> Result<&User> {
        let k = format!("{channel}:{external_id}");
        let uid = self
            .bindings
            .get(&k)
            .ok_or_else(|| IamError::UnknownIdentity(k.clone()))?;
        self.users
            .get(uid)
            .ok_or_else(|| IamError::UnknownUser(uid.clone()))
    }

    pub fn issue_token(&self, user_id: &str, now: i64, ttl: i64) -> Result<String> {
        let u = self
            .users
            .get(user_id)
            .ok_or_else(|| IamError::UnknownUser(user_id.into()))?;
        let claims = Claims {
            sub: u.user_id.clone(),
            role: u.role.clone(),
            tenant: u.tenant.clone(),
            exp: now + ttl,
        };
        let header = b64_encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = b64_encode(&serde_json::to_vec(&claims).expect("serialize claims"));
        let signing_input = format!("{header}.{payload}");
        let sig = sign(&self.secret, &signing_input);
        Ok(format!("{signing_input}.{sig}"))
    }

    pub fn authenticate(&self, token: &str, now: i64) -> Result<Claims> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(IamError::MalformedToken);
        }
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let mut mac = HmacSha256::new_from_slice(self.secret.as_bytes()).expect("hmac key");
        mac.update(signing_input.as_bytes());
        let sig_bytes = b64_decode(parts[2])?;
        mac.verify_slice(&sig_bytes)
            .map_err(|_| IamError::InvalidSignature)?;
        let payload = b64_decode(parts[1])?;
        let claims: Claims =
            serde_json::from_slice(&payload).map_err(|_| IamError::MalformedToken)?;
        if claims.exp <= now {
            return Err(IamError::Expired);
        }
        Ok(claims)
    }

    /// Resource-level authorization. The tenant boundary is hard: a claim never
    /// authorizes a resource in another tenant, regardless of role. Within the
    /// tenant, the role policy must grant (kind, action).
    pub fn authorize(
        &mut self,
        claims: &Claims,
        resource: &ResourceRef,
        action: Action,
    ) -> Result<()> {
        let (decision, reason) = if claims.tenant != resource.tenant {
            ("deny", "cross-tenant")
        } else {
            match self.policies.get(&claims.role) {
                Some(set) if set.contains(&(resource.kind, action)) => ("allow", "policy-grant"),
                Some(_) => ("deny", "role-lacks-permission"),
                None => ("deny", "unknown-role"),
            }
        };
        self.audit.push(json!({
            "event": "authz",
            "sub": claims.sub,
            "role": claims.role,
            "tenant": claims.tenant,
            "resourceKind": resource.kind,
            "resourceTenant": resource.tenant,
            "resourceId": resource.id,
            "action": action,
            "decision": decision,
            "reason": reason,
        }));
        match decision {
            "allow" => Ok(()),
            _ if claims.tenant != resource.tenant => {
                Err(IamError::CrossTenant(resource.id.clone()))
            }
            _ => Err(IamError::Forbidden(format!(
                "{} {:?} {:?}",
                claims.role, resource.kind, action
            ))),
        }
    }

    pub fn audit(&self) -> &[Value] {
        &self.audit
    }

    /// Render the audit trail as a diagnostic string with secrets redacted.
    pub fn redacted_audit(&self, secrets: &[&str]) -> String {
        let raw = serde_json::to_string(&self.audit).expect("serialize audit");
        redact(&raw, secrets)
    }
}
