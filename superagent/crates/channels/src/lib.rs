//! SuperAgent channel ingress/egress (P4-3 / #92).
//!
//! Model-free and process-free: inbound Lark/Slack/webhook payloads are
//! normalized into a task driven through the meta-agent session; privileged
//! actions surface an outbound approval card that shows the RAW command,
//! working directory, permission scope, reason and allowed choices (never
//! hiding the real command). Channel identities map to IAM users (#91).
//! A dropped connection marks the run "执行状态待确认" (execution status
//! pending confirmation), never "已取消" (cancelled).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use superagent_iam::{IamError, IamServer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    Lark,
    Slack,
    Webhook,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Lark => "lark",
            Channel::Slack => "slack",
            Channel::Webhook => "webhook",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelError {
    MalformedPayload(String),
    UnknownIdentity(String),
    UnknownTask(String),
    UnknownCard(String),
    NotRunning(String),
    AlreadyDecided(String),
    Iam(String),
}

impl fmt::Display for ChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChannelError::MalformedPayload(s) => write!(f, "malformed payload: {s}"),
            ChannelError::UnknownIdentity(s) => write!(f, "unmapped channel identity: {s}"),
            ChannelError::UnknownTask(s) => write!(f, "unknown task: {s}"),
            ChannelError::UnknownCard(s) => write!(f, "unknown approval card: {s}"),
            ChannelError::NotRunning(s) => write!(f, "task not running: {s}"),
            ChannelError::AlreadyDecided(s) => write!(f, "card already decided: {s}"),
            ChannelError::Iam(s) => write!(f, "iam: {s}"),
        }
    }
}

impl std::error::Error for ChannelError {}

impl From<IamError> for ChannelError {
    fn from(e: IamError) -> Self {
        match e {
            IamError::UnknownIdentity(s) => ChannelError::UnknownIdentity(s),
            other => ChannelError::Iam(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, ChannelError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundMessage {
    pub channel: Channel,
    pub external_id: String,
    pub text: String,
    pub thread_id: String,
}

fn field<'a>(v: &'a Value, ptr: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for key in ptr {
        cur = cur.get(key)?;
    }
    cur.as_str()
}

/// Generic webhook payload: `{sender, text, thread?}`.
pub fn parse_webhook(v: &Value) -> Result<InboundMessage> {
    let external_id = field(v, &["sender"])
        .ok_or_else(|| ChannelError::MalformedPayload("webhook.sender".into()))?;
    let text =
        field(v, &["text"]).ok_or_else(|| ChannelError::MalformedPayload("webhook.text".into()))?;
    let thread_id = field(v, &["thread"]).unwrap_or("default");
    Ok(InboundMessage {
        channel: Channel::Webhook,
        external_id: external_id.into(),
        text: text.into(),
        thread_id: thread_id.into(),
    })
}

/// Lark `im.message.receive_v1` event:
/// `{event:{sender:{sender_id:{open_id}}, message:{chat_id, content:"{\"text\":..}"}}}`.
pub fn parse_lark(v: &Value) -> Result<InboundMessage> {
    let external_id = field(v, &["event", "sender", "sender_id", "open_id"])
        .ok_or_else(|| ChannelError::MalformedPayload("lark.open_id".into()))?;
    let thread_id = field(v, &["event", "message", "chat_id"])
        .ok_or_else(|| ChannelError::MalformedPayload("lark.chat_id".into()))?;
    let content = field(v, &["event", "message", "content"])
        .ok_or_else(|| ChannelError::MalformedPayload("lark.content".into()))?;
    let content_json: Value = serde_json::from_str(content)
        .map_err(|_| ChannelError::MalformedPayload("lark.content not json".into()))?;
    let text = field(&content_json, &["text"])
        .ok_or_else(|| ChannelError::MalformedPayload("lark.content.text".into()))?;
    Ok(InboundMessage {
        channel: Channel::Lark,
        external_id: external_id.into(),
        text: text.into(),
        thread_id: thread_id.into(),
    })
}

/// Slack Events API `message` callback:
/// `{event:{user, text, channel, ts, thread_ts?}}`.
pub fn parse_slack(v: &Value) -> Result<InboundMessage> {
    let external_id = field(v, &["event", "user"])
        .ok_or_else(|| ChannelError::MalformedPayload("slack.user".into()))?;
    let text = field(v, &["event", "text"])
        .ok_or_else(|| ChannelError::MalformedPayload("slack.text".into()))?;
    let channel = field(v, &["event", "channel"])
        .ok_or_else(|| ChannelError::MalformedPayload("slack.channel".into()))?;
    let thread_id = field(v, &["event", "thread_ts"])
        .or_else(|| field(v, &["event", "ts"]))
        .unwrap_or(channel);
    Ok(InboundMessage {
        channel: Channel::Slack,
        external_id: external_id.into(),
        text: text.into(),
        thread_id: thread_id.into(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Running,
    PendingConfirmation,
    Completed,
    Denied,
}

impl TaskStatus {
    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::Running => "执行中",
            TaskStatus::PendingConfirmation => "执行状态待确认",
            TaskStatus::Completed => "已完成",
            TaskStatus::Denied => "已拒绝",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub task_id: String,
    pub tenant: String,
    pub user_id: String,
    pub role: String,
    pub channel: Channel,
    pub external_id: String,
    pub session_id: String,
    pub text: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// The RAW command/action, shown verbatim on the card (never hidden).
    pub action: String,
    pub cwd: String,
    pub permission_scope: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Allow,
    Deny,
}

impl Decision {
    fn as_str(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalCard {
    pub card_id: String,
    pub task_id: String,
    pub request: ApprovalRequest,
    pub choices: Vec<Decision>,
    pub decided: Option<Decision>,
}

impl ApprovalCard {
    /// Render the outbound card. The raw command is shown verbatim alongside
    /// the working directory, permission scope, reason and allowed choices.
    pub fn render(&self) -> Value {
        json!({
            "cardId": self.card_id,
            "taskId": self.task_id,
            "action": self.request.action,
            "cwd": self.request.cwd,
            "permissionScope": self.request.permission_scope,
            "reason": self.request.reason,
            "choices": ["allow", "deny"],
            "decided": self.decided.map(Decision::as_str),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub channel: Channel,
    pub external_id: String,
    pub thread_id: String,
    pub text: String,
}

pub struct ChannelRouter {
    iam: IamServer,
    tasks: BTreeMap<String, Task>,
    cards: BTreeMap<String, ApprovalCard>,
    thread_of: BTreeMap<String, String>,
    seq: usize,
    audit: Vec<Value>,
}

impl ChannelRouter {
    pub fn new(secret: &str) -> Self {
        Self {
            iam: IamServer::new(secret),
            tasks: BTreeMap::new(),
            cards: BTreeMap::new(),
            thread_of: BTreeMap::new(),
            seq: 0,
            audit: Vec::new(),
        }
    }

    pub fn iam(&self) -> &IamServer {
        &self.iam
    }

    pub fn add_user(&mut self, user_id: &str, role: &str, tenant: &str) {
        self.iam.add_user(user_id, role, tenant);
    }

    pub fn bind_channel(
        &mut self,
        channel: Channel,
        external_id: &str,
        user_id: &str,
    ) -> Result<()> {
        self.iam
            .bind_channel(channel.as_str(), external_id, user_id)
            .map_err(Into::into)
    }

    /// Inbound message -> normalized task bound to an IAM identity/session.
    pub fn ingest(&mut self, channel: Channel, payload: &Value) -> Result<Task> {
        let msg = match channel {
            Channel::Lark => parse_lark(payload)?,
            Channel::Slack => parse_slack(payload)?,
            Channel::Webhook => parse_webhook(payload)?,
        };
        let (user_id, role, tenant) = {
            let user = self
                .iam
                .map_channel_identity(channel.as_str(), &msg.external_id)?;
            (user.user_id.clone(), user.role.clone(), user.tenant.clone())
        };
        self.seq += 1;
        let task_id = format!("task-{}", self.seq);
        let session_id = format!("{tenant}:{}", msg.thread_id);
        let task = Task {
            task_id: task_id.clone(),
            tenant,
            user_id,
            role,
            channel,
            external_id: msg.external_id.clone(),
            session_id,
            text: msg.text.clone(),
            status: TaskStatus::Running,
        };
        self.thread_of.insert(task_id.clone(), msg.thread_id);
        self.tasks.insert(task_id.clone(), task.clone());
        self.audit.push(json!({
            "event": "ingest",
            "taskId": task_id,
            "channel": channel.as_str(),
            "tenant": task.tenant,
            "userId": task.user_id,
        }));
        Ok(task)
    }

    /// Surface a privileged action as an outbound approval card.
    pub fn request_approval(
        &mut self,
        task_id: &str,
        request: ApprovalRequest,
    ) -> Result<ApprovalCard> {
        let status = self
            .tasks
            .get(task_id)
            .ok_or_else(|| ChannelError::UnknownTask(task_id.into()))?
            .status;
        if status != TaskStatus::Running {
            return Err(ChannelError::NotRunning(task_id.into()));
        }
        self.seq += 1;
        let card_id = format!("card-{}", self.seq);
        let card = ApprovalCard {
            card_id: card_id.clone(),
            task_id: task_id.into(),
            request,
            choices: vec![Decision::Allow, Decision::Deny],
            decided: None,
        };
        self.cards.insert(card_id.clone(), card.clone());
        self.audit.push(json!({
            "event": "approval_requested",
            "cardId": card_id,
            "taskId": task_id,
            "action": card.request.action,
        }));
        Ok(card)
    }

    /// Apply an approval decision and produce the outbound channel reply.
    pub fn decide(&mut self, card_id: &str, decision: Decision) -> Result<OutboundMessage> {
        let (task_id, action) = {
            let card = self
                .cards
                .get_mut(card_id)
                .ok_or_else(|| ChannelError::UnknownCard(card_id.into()))?;
            if card.decided.is_some() {
                return Err(ChannelError::AlreadyDecided(card_id.into()));
            }
            card.decided = Some(decision);
            (card.task_id.clone(), card.request.action.clone())
        };
        let (channel, external_id) = {
            let task = self
                .tasks
                .get_mut(&task_id)
                .ok_or_else(|| ChannelError::UnknownTask(task_id.clone()))?;
            task.status = match decision {
                Decision::Allow => TaskStatus::Completed,
                Decision::Deny => TaskStatus::Denied,
            };
            (task.channel, task.external_id.clone())
        };
        let text = match decision {
            Decision::Allow => format!("已批准并执行：{action}"),
            Decision::Deny => format!("已拒绝：{action}"),
        };
        let thread_id = self.thread_of.get(&task_id).cloned().unwrap_or_default();
        self.audit.push(json!({
            "event": "approval_decided",
            "cardId": card_id,
            "taskId": task_id,
            "decision": decision.as_str(),
        }));
        Ok(OutboundMessage {
            channel,
            external_id,
            thread_id,
            text,
        })
    }

    /// A dropped connection never cancels a running task; the run is left
    /// "执行状态待确认" (execution status pending confirmation).
    pub fn mark_disconnected(&mut self, task_id: &str) -> Result<TaskStatus> {
        let status = {
            let task = self
                .tasks
                .get_mut(task_id)
                .ok_or_else(|| ChannelError::UnknownTask(task_id.into()))?;
            if task.status == TaskStatus::Running {
                task.status = TaskStatus::PendingConfirmation;
            }
            task.status
        };
        self.audit.push(json!({
            "event": "disconnected",
            "taskId": task_id,
            "status": status.label(),
        }));
        Ok(status)
    }

    pub fn task(&self, task_id: &str) -> Option<&Task> {
        self.tasks.get(task_id)
    }

    pub fn card(&self, card_id: &str) -> Option<&ApprovalCard> {
        self.cards.get(card_id)
    }

    pub fn audit(&self) -> &[Value] {
        &self.audit
    }
}
