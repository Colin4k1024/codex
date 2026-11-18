//! SuperAgent cron executor (P4-4 / #93).
//!
//! Gateway-side scheduled triggering of meta-agent tasks. The scheduling core
//! is deterministic and model-free: civil-time fire computation (hourly / daily
//! / weekdays / weekly, mirroring the plugin ScheduledTaskSummary metadata),
//! tenant-isolated definition CRUD, misfire policy (skip / catch-up), and
//! lease/idempotency-key dedup so concurrent ticks and restarts never
//! double-fire. Every fire is audited. A thin tokio interval driver
//! ([`run_interval`]) pumps `tick(now)` on a clock; the core logic is proven
//! with a controlled clock so tests are deterministic (no real-time flake).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

const HOUR: i64 = 3600;
const DAY: i64 = 86400;
const MAX_CATCHUP: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Schedule {
    Hourly,
    Daily { hour: u8 },
    Weekdays { hour: u8 },
    Weekly { weekday: u8, hour: u8 },
}

fn hour_of(ts: i64) -> i64 {
    ts.rem_euclid(DAY) / HOUR
}

/// Monday=0 .. Sunday=6. 1970-01-01 (epoch day 0) was a Thursday (=3).
fn weekday_of(ts: i64) -> i64 {
    (ts.div_euclid(DAY) + 3).rem_euclid(7)
}

impl Schedule {
    /// Smallest hour-aligned fire time strictly greater than `after`.
    pub fn next_fire(&self, after: i64) -> i64 {
        let mut t = (after.div_euclid(HOUR) + 1) * HOUR;
        for _ in 0..(24 * 8 + 1) {
            let h = hour_of(t);
            let wd = weekday_of(t);
            let ok = match self {
                Schedule::Hourly => true,
                Schedule::Daily { hour } => h == *hour as i64,
                Schedule::Weekdays { hour } => h == *hour as i64 && wd < 5,
                Schedule::Weekly { weekday, hour } => h == *hour as i64 && wd == *weekday as i64,
            };
            if ok {
                return t;
            }
            t += HOUR;
        }
        t
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MisfirePolicy {
    Skip,
    CatchUp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronDef {
    pub id: String,
    pub tenant: String,
    pub schedule: Schedule,
    pub task_class: String,
    pub misfire: MisfirePolicy,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fire {
    pub tenant: String,
    pub def_id: String,
    pub fire_time: i64,
    pub task_class: String,
    pub idempotency_key: String,
}

#[derive(Debug)]
pub enum CronError {
    DuplicateDef(String),
    UnknownDef(String),
    CrossTenant(String),
}

impl std::fmt::Display for CronError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CronError::DuplicateDef(s) => write!(f, "duplicate schedule id: {s}"),
            CronError::UnknownDef(s) => write!(f, "unknown schedule: {s}"),
            CronError::CrossTenant(s) => write!(f, "cross-tenant access denied: {s}"),
        }
    }
}

impl std::error::Error for CronError {}

pub type Result<T> = std::result::Result<T, CronError>;

fn key(tenant: &str, id: &str) -> String {
    format!("{tenant}/{id}")
}

#[derive(Default)]
pub struct Scheduler {
    defs: BTreeMap<String, CronDef>,
    next_due: BTreeMap<String, i64>,
    fired: BTreeSet<String>,
    audit: Vec<Value>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_def(&mut self, def: CronDef) -> Result<()> {
        let k = key(&def.tenant, &def.id);
        if self.defs.contains_key(&k) {
            return Err(CronError::DuplicateDef(k));
        }
        self.next_due
            .insert(k.clone(), def.schedule.next_fire(def.created_at));
        self.defs.insert(k, def);
        Ok(())
    }

    pub fn update_def(&mut self, tenant: &str, def: CronDef) -> Result<()> {
        let k = key(tenant, &def.id);
        if !self.defs.contains_key(&k) {
            return Err(CronError::UnknownDef(k));
        }
        self.next_due
            .insert(k.clone(), def.schedule.next_fire(def.created_at));
        self.defs.insert(k, def);
        Ok(())
    }

    pub fn remove_def(&mut self, tenant: &str, id: &str) -> Result<bool> {
        let k = key(tenant, id);
        self.next_due.remove(&k);
        Ok(self.defs.remove(&k).is_some())
    }

    /// Tenant isolation: only the caller's own definitions are listed.
    pub fn list_defs(&self, tenant: &str) -> Vec<&CronDef> {
        self.defs.values().filter(|d| d.tenant == tenant).collect()
    }

    pub fn get_def(&self, tenant: &str, id: &str) -> Result<&CronDef> {
        let k = key(tenant, id);
        match self.defs.get(&k) {
            Some(d) if d.tenant == tenant => Ok(d),
            Some(_) => Err(CronError::CrossTenant(k)),
            None => Err(CronError::UnknownDef(k)),
        }
    }

    pub fn audit(&self) -> &[Value] {
        &self.audit
    }

    pub fn fired_keys(&self) -> &BTreeSet<String> {
        &self.fired
    }

    /// Process all due fires up to `now`. Idempotency keys (tenant/id/fire_time)
    /// guarantee a given fire happens at most once across concurrent ticks and
    /// restarts. Misfire Skip fires only the latest missed slot; CatchUp fires
    /// each missed slot (bounded by MAX_CATCHUP).
    pub fn tick(&mut self, now: i64) -> Vec<Fire> {
        let mut fires = Vec::new();
        let keys: Vec<String> = self.defs.keys().cloned().collect();
        for k in keys {
            let def = self.defs[&k].clone();
            let mut due = self.next_due[&k];
            let mut due_times = Vec::new();
            while due <= now && due_times.len() < MAX_CATCHUP {
                due_times.push(due);
                due = def.schedule.next_fire(due);
            }
            if due_times.is_empty() {
                continue;
            }
            let selected: Vec<i64> = match def.misfire {
                MisfirePolicy::Skip => vec![*due_times.last().expect("non-empty")],
                MisfirePolicy::CatchUp => due_times,
            };
            for ft in selected {
                let idem = format!("{k}:{ft}");
                if self.fired.contains(&idem) {
                    continue;
                }
                self.fired.insert(idem.clone());
                let fire = Fire {
                    tenant: def.tenant.clone(),
                    def_id: def.id.clone(),
                    fire_time: ft,
                    task_class: def.task_class.clone(),
                    idempotency_key: idem.clone(),
                };
                self.audit.push(json!({
                    "event": "fire",
                    "tenant": fire.tenant,
                    "defId": fire.def_id,
                    "fireTime": fire.fire_time,
                    "taskClass": fire.task_class,
                    "idempotencyKey": fire.idempotency_key,
                    "misfire": def.misfire,
                }));
                fires.push(fire);
            }
            self.next_due.insert(k, def.schedule.next_fire(now));
        }
        fires
    }

    /// Persisted state for restart recovery (definitions + next_due + fired keys).
    pub fn export_state(&self) -> Value {
        json!({
            "defs": self.defs.values().collect::<Vec<_>>(),
            "nextDue": self.next_due,
            "fired": self.fired.iter().collect::<Vec<_>>(),
        })
    }

    pub fn import_state(v: &Value) -> Result<Self> {
        let defs: Vec<CronDef> = serde_json::from_value(v["defs"].clone())
            .map_err(|e| CronError::UnknownDef(format!("defs: {e}")))?;
        let next_due: BTreeMap<String, i64> = serde_json::from_value(v["nextDue"].clone())
            .map_err(|e| CronError::UnknownDef(format!("nextDue: {e}")))?;
        let fired: Vec<String> = serde_json::from_value(v["fired"].clone())
            .map_err(|e| CronError::UnknownDef(format!("fired: {e}")))?;
        let mut s = Self::new();
        for d in defs {
            s.defs.insert(key(&d.tenant, &d.id), d);
        }
        s.next_due = next_due;
        s.fired = fired.into_iter().collect();
        Ok(s)
    }
}

/// Clock abstraction so the tokio driver can be tested with a controlled clock.
pub trait Clock: Send + Sync {
    fn now(&self) -> i64;
}

pub struct ManualClock(AtomicI64);

impl ManualClock {
    pub fn new(start: i64) -> Self {
        Self(AtomicI64::new(start))
    }
    pub fn set(&self, ts: i64) {
        self.0.store(ts, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// tokio interval driver: pumps `tick(clock.now())` every `interval` for `ticks`
/// iterations, invoking `on_fire` for each triggered task. This is the runtime
/// binding that turns the deterministic core into a live scheduled executor.
pub async fn run_interval<F>(
    sched: Arc<Mutex<Scheduler>>,
    clock: Arc<dyn Clock>,
    interval: std::time::Duration,
    ticks: usize,
    mut on_fire: F,
) where
    F: FnMut(Fire),
{
    for _ in 0..ticks {
        let now = clock.now();
        let fires = {
            let mut s = sched.lock().await;
            s.tick(now)
        };
        for f in fires {
            on_fire(f);
        }
        tokio::time::sleep(interval).await;
    }
}
