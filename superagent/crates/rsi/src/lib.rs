//! SuperAgent RSI closed loop v1 (P3-3 / #89).
//!
//! Self-evolution is a CONTROLLED loop, never automatic prompt/skill/tool
//! mutation. Refined memory output (#87/#88 memories/write two-stage refine)
//! becomes a traceable candidate; a fixed fixture is evaluated deterministically
//! (same model, same fixture, per-case trace); a HUMAN approval gates promotion
//! into a new capability-pack version whose digest is pinned via the #98
//! marketplace (a run bound to the old digest is unaffected by the upgrade);
//! rollback restores the prior version. Unapproved experience NEVER enters the
//! prompt, and any artifact attempting privilege escalation is rejected.
//!
//! Model-free / process-free / network-free: the fixture evaluator is a
//! deterministic lookup over recorded prior/expected answers; promotion reuses
//! superagent-marketplace (sha256 digest + HMAC signature + safe extraction).

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use superagent_marketplace::{
    FileKind, InstalledPack, MarketError, Marketplace, PackFile, PackManifest, Source,
};

/// Output of the memories/write two-stage refine, with traceable provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinedMemory {
    pub source_session: String,
    pub source_id: String,
}

/// A candidate effect. Only `AnswerRule` is benign learning; `Elevate` and
/// `DeveloperInstructions` are privileged and rejected as escalation attempts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    AnswerRule { key: String, value: String },
    Elevate { scope: String },
    DeveloperInstructions { text: String },
}

impl Effect {
    fn is_privileged(&self) -> bool {
        !matches!(self, Effect::AnswerRule { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateStatus {
    Pending,
    Evaluated,
    Approved,
    Rejected,
    Promoted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub candidate_id: String,
    pub source_session: String,
    pub source_id: String,
    pub effects: Vec<Effect>,
    pub status: CandidateStatus,
    pub eval: Option<EvalReport>,
    pub approver: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureCase {
    pub key: String,
    /// The un-evolved agent's recorded answer (a known failure when != expected).
    pub prior: String,
    pub expected: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    pub fixture_id: String,
    pub cases: Vec<FixtureCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalReport {
    pub fixture_id: String,
    pub model: String,
    pub total: usize,
    pub baseline_pass: usize,
    pub candidate_pass: usize,
    pub delta: i64,
    pub per_case: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Approve,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RsiError {
    UnknownCandidate(String),
    EmptyEffects,
    NotEvaluated(String),
    NotApproved(String),
    AlreadyPromoted(String),
    PrivilegeEscalation(String),
    Market(String),
}

impl fmt::Display for RsiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RsiError::UnknownCandidate(s) => write!(f, "unknown candidate: {s}"),
            RsiError::EmptyEffects => write!(f, "candidate has no effects"),
            RsiError::NotEvaluated(s) => write!(f, "candidate not evaluated: {s}"),
            RsiError::NotApproved(s) => write!(f, "candidate not approved: {s}"),
            RsiError::AlreadyPromoted(s) => write!(f, "candidate already promoted: {s}"),
            RsiError::PrivilegeEscalation(s) => write!(f, "privilege escalation rejected: {s}"),
            RsiError::Market(s) => write!(f, "marketplace: {s}"),
        }
    }
}

impl std::error::Error for RsiError {}

impl From<MarketError> for RsiError {
    fn from(e: MarketError) -> Self {
        RsiError::Market(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, RsiError>;

pub struct RsiEngine {
    market: Marketplace,
    key_id: String,
    candidates: BTreeMap<String, Candidate>,
    seq: usize,
    audit: Vec<Value>,
}

impl RsiEngine {
    pub fn new(key_id: &str, secret: &str) -> Self {
        let mut market = Marketplace::new();
        market.register_key(key_id, secret);
        Self {
            market,
            key_id: key_id.into(),
            candidates: BTreeMap::new(),
            seq: 0,
            audit: Vec::new(),
        }
    }

    /// Collect a refined-memory产出 into the candidate library (traceable source).
    pub fn collect(&mut self, refined: RefinedMemory, effects: Vec<Effect>) -> Result<Candidate> {
        if effects.is_empty() {
            return Err(RsiError::EmptyEffects);
        }
        self.seq += 1;
        let candidate_id = format!("cand-{}", self.seq);
        let cand = Candidate {
            candidate_id: candidate_id.clone(),
            source_session: refined.source_session,
            source_id: refined.source_id,
            effects,
            status: CandidateStatus::Pending,
            eval: None,
            approver: None,
        };
        self.candidates.insert(candidate_id.clone(), cand.clone());
        self.audit.push(json!({
            "event": "collect",
            "candidateId": candidate_id,
            "sourceSession": cand.source_session,
            "sourceId": cand.source_id,
        }));
        Ok(cand)
    }

    /// Deterministic fixed-fixture evaluation (same model, same fixture).
    /// A privileged effect rejects the candidate outright.
    pub fn evaluate(
        &mut self,
        candidate_id: &str,
        fixture: &Fixture,
        model: &str,
    ) -> Result<EvalReport> {
        let effects = self
            .candidates
            .get(candidate_id)
            .ok_or_else(|| RsiError::UnknownCandidate(candidate_id.into()))?
            .effects
            .clone();
        if effects.iter().any(Effect::is_privileged) {
            if let Some(c) = self.candidates.get_mut(candidate_id) {
                c.status = CandidateStatus::Rejected;
            }
            self.audit.push(json!({
                "event": "rejected",
                "candidateId": candidate_id,
                "reason": "privilege_escalation",
            }));
            return Err(RsiError::PrivilegeEscalation(candidate_id.into()));
        }
        let rules: BTreeMap<String, String> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::AnswerRule { key, value } => Some((key.clone(), value.clone())),
                _ => None,
            })
            .collect();
        let mut baseline_pass = 0usize;
        let mut candidate_pass = 0usize;
        let mut per_case = Vec::new();
        for case in &fixture.cases {
            let candidate_answer = rules
                .get(&case.key)
                .cloned()
                .unwrap_or_else(|| case.prior.clone());
            let b = case.prior == case.expected;
            let c = candidate_answer == case.expected;
            baseline_pass += usize::from(b);
            candidate_pass += usize::from(c);
            per_case.push(json!({
                "key": case.key,
                "prior": case.prior,
                "expected": case.expected,
                "baselineAnswer": case.prior,
                "candidateAnswer": candidate_answer,
                "baselinePass": b,
                "candidatePass": c,
            }));
        }
        let report = EvalReport {
            fixture_id: fixture.fixture_id.clone(),
            model: model.into(),
            total: fixture.cases.len(),
            baseline_pass,
            candidate_pass,
            delta: candidate_pass as i64 - baseline_pass as i64,
            per_case,
        };
        if let Some(c) = self.candidates.get_mut(candidate_id) {
            c.status = CandidateStatus::Evaluated;
            c.eval = Some(report.clone());
        }
        self.audit.push(json!({
            "event": "evaluate",
            "candidateId": candidate_id,
            "fixtureId": report.fixture_id,
            "model": report.model,
            "baselinePass": report.baseline_pass,
            "candidatePass": report.candidate_pass,
            "delta": report.delta,
        }));
        Ok(report)
    }

    /// Human approval gate. Promotion is NEVER automatic.
    pub fn approve(
        &mut self,
        candidate_id: &str,
        approver: &str,
        decision: Decision,
    ) -> Result<CandidateStatus> {
        let status = self
            .candidates
            .get(candidate_id)
            .ok_or_else(|| RsiError::UnknownCandidate(candidate_id.into()))?
            .status;
        if status != CandidateStatus::Evaluated {
            return Err(RsiError::NotEvaluated(candidate_id.into()));
        }
        let new_status = match decision {
            Decision::Approve => CandidateStatus::Approved,
            Decision::Reject => CandidateStatus::Rejected,
        };
        if let Some(c) = self.candidates.get_mut(candidate_id) {
            c.status = new_status;
            c.approver = Some(approver.into());
        }
        self.audit.push(json!({
            "event": "approve",
            "candidateId": candidate_id,
            "approver": approver,
            "decision": match decision { Decision::Approve => "approve", Decision::Reject => "reject" },
            "status": format!("{new_status:?}"),
        }));
        Ok(new_status)
    }

    /// Promote an APPROVED candidate into a new capability-pack version. The
    /// pack digest is fixed by the marketplace; a run bound to the old digest
    /// is unaffected. Privileged effects are rejected again as a defense.
    pub fn promote(
        &mut self,
        candidate_id: &str,
        pack_name: &str,
        version: &str,
        target_root: &Path,
    ) -> Result<InstalledPack> {
        let (status, effects) = {
            let c = self
                .candidates
                .get(candidate_id)
                .ok_or_else(|| RsiError::UnknownCandidate(candidate_id.into()))?;
            (c.status, c.effects.clone())
        };
        if status == CandidateStatus::Promoted {
            return Err(RsiError::AlreadyPromoted(candidate_id.into()));
        }
        if status != CandidateStatus::Approved {
            return Err(RsiError::NotApproved(candidate_id.into()));
        }
        if effects.iter().any(Effect::is_privileged) {
            return Err(RsiError::PrivilegeEscalation(candidate_id.into()));
        }
        let rules: BTreeMap<String, String> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::AnswerRule { key, value } => Some((key.clone(), value.clone())),
                _ => None,
            })
            .collect();
        let manifest = PackManifest {
            name: pack_name.into(),
            version: version.into(),
            source: Source::Curated,
            publisher_key_id: self.key_id.clone(),
            licenses: vec!["Apache-2.0".into()],
            files: vec![PackFile {
                path: "experience.json".into(),
                content: serde_json::to_string(&rules).expect("serialize rules"),
                kind: FileKind::Regular,
                symlink_target: None,
            }],
        };
        let signed = self.market.sign_pack(manifest)?;
        let installed = if self.market.current(pack_name).is_some() {
            self.market.upgrade(&signed, target_root)?
        } else {
            self.market.install(&signed, target_root)?
        };
        if let Some(c) = self.candidates.get_mut(candidate_id) {
            c.status = CandidateStatus::Promoted;
        }
        self.audit.push(json!({
            "event": "promote",
            "candidateId": candidate_id,
            "pack": pack_name,
            "version": installed.version,
            "digest": installed.digest,
        }));
        Ok(installed)
    }

    pub fn rollback(&mut self, pack_name: &str) -> Result<InstalledPack> {
        let rec = self.market.rollback(pack_name)?;
        self.audit.push(json!({
            "event": "rollback",
            "pack": pack_name,
            "version": rec.version,
            "digest": rec.digest,
        }));
        Ok(rec)
    }

    /// Pin a run to the currently installed pack digest (survives later promote).
    pub fn bind_run(&mut self, run_id: &str, pack_name: &str) -> Result<String> {
        Ok(self.market.bind_run(run_id, pack_name)?)
    }

    pub fn resolve_run_binding(&self, run_id: &str) -> Option<&String> {
        self.market.resolve_run_binding(run_id)
    }

    pub fn current_pack(&self, pack_name: &str) -> Option<&InstalledPack> {
        self.market.current(pack_name)
    }

    /// Build the prompt experience context from PROMOTED candidates only.
    /// Pending/Evaluated/Approved-but-unpromoted/Rejected experience is excluded,
    /// and privileged effects never appear.
    pub fn prompt_context(&self) -> String {
        let mut rules: BTreeMap<String, String> = BTreeMap::new();
        for c in self.candidates.values() {
            if c.status != CandidateStatus::Promoted {
                continue;
            }
            for e in &c.effects {
                if let Effect::AnswerRule { key, value } = e {
                    rules.insert(key.clone(), value.clone());
                }
            }
        }
        rules
            .iter()
            .map(|(k, v)| format!("{k} => {v}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn candidate(&self, candidate_id: &str) -> Option<&Candidate> {
        self.candidates.get(candidate_id)
    }

    pub fn audit(&self) -> &[Value] {
        &self.audit
    }
}
