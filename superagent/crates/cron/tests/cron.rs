use std::sync::Arc;

use superagent_cron::{
    run_interval, Clock, CronDef, CronError, ManualClock, MisfirePolicy, Schedule, Scheduler,
};
use tokio::sync::Mutex;

const T0: i64 = 1_699_999_200 // hour-aligned epoch
;
const HOUR: i64 = 3600;
const DAY: i64 = 86400;

fn hour_of(ts: i64) -> i64 {
    ts.rem_euclid(DAY) / HOUR
}
fn weekday_of(ts: i64) -> i64 {
    (ts.div_euclid(DAY) + 3).rem_euclid(7)
}

fn def(id: &str, tenant: &str, schedule: Schedule, misfire: MisfirePolicy) -> CronDef {
    CronDef {
        id: id.into(),
        tenant: tenant.into(),
        schedule,
        task_class: "meta_task".into(),
        misfire,
        created_at: T0,
    }
}

#[test]
fn schedules_fire_on_time_audited() {
    assert_eq!(Schedule::Hourly.next_fire(T0), T0 + HOUR);
    let d = Schedule::Daily { hour: 9 }.next_fire(T0);
    assert!(d > T0 && d <= T0 + DAY && hour_of(d) == 9);
    let w = Schedule::Weekdays { hour: 9 }.next_fire(T0);
    assert!(hour_of(w) == 9 && weekday_of(w) < 5);
    let wk = Schedule::Weekly {
        weekday: 0,
        hour: 9,
    }
    .next_fire(T0);
    assert!(hour_of(wk) == 9 && weekday_of(wk) == 0 && wk <= T0 + 8 * DAY);

    let mut s = Scheduler::new();
    s.add_def(def("j1", "t1", Schedule::Hourly, MisfirePolicy::CatchUp))
        .unwrap();
    let fires = s.tick(T0 + HOUR);
    assert_eq!(fires.len(), 1);
    assert_eq!(fires[0].fire_time, T0 + HOUR);
    assert_eq!(fires[0].tenant, "t1");
    assert_eq!(s.audit().len(), 1);
    assert_eq!(s.audit()[0]["event"], "fire");
    assert_eq!(s.audit()[0]["fireTime"], T0 + HOUR);
    assert_eq!(s.audit()[0]["taskClass"], "meta_task");
    assert!(s.audit()[0]["idempotencyKey"]
        .as_str()
        .unwrap()
        .contains("t1/j1"));
}

#[test]
fn crud_and_tenant_isolation() {
    let mut s = Scheduler::new();
    s.add_def(def("j1", "t1", Schedule::Hourly, MisfirePolicy::Skip))
        .unwrap();
    s.add_def(def("j1", "t2", Schedule::Hourly, MisfirePolicy::Skip))
        .unwrap();
    assert_eq!(s.list_defs("t1").len(), 1);
    assert_eq!(s.list_defs("t2").len(), 1);
    assert!(s.get_def("t1", "j1").is_ok());
    // t1 cannot reach t2's definition (key is tenant-scoped).
    assert!(matches!(
        s.get_def("t1", "j2"),
        Err(CronError::UnknownDef(_))
    ));
    s.add_def(def("j2", "t2", Schedule::Hourly, MisfirePolicy::Skip))
        .unwrap();
    assert!(matches!(
        s.get_def("t1", "j2"),
        Err(CronError::UnknownDef(_))
    ));
    assert!(matches!(
        s.add_def(def("j1", "t1", Schedule::Hourly, MisfirePolicy::Skip)),
        Err(CronError::DuplicateDef(_))
    ));
    s.update_def(
        "t1",
        def("j1", "t1", Schedule::Daily { hour: 3 }, MisfirePolicy::Skip),
    )
    .unwrap();
    assert_eq!(
        s.get_def("t1", "j1").unwrap().schedule,
        Schedule::Daily { hour: 3 }
    );
    assert!(s.remove_def("t1", "j1").unwrap());
    assert_eq!(s.list_defs("t1").len(), 0);
    assert!(!s.remove_def("t1", "j1").unwrap());
}

#[test]
fn misfire_skip_vs_catchup() {
    let mut skip = Scheduler::new();
    skip.add_def(def("j1", "t1", Schedule::Hourly, MisfirePolicy::Skip))
        .unwrap();
    let f = skip.tick(T0 + 5 * HOUR);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].fire_time, T0 + 5 * HOUR);

    let mut catch = Scheduler::new();
    catch
        .add_def(def("j1", "t1", Schedule::Hourly, MisfirePolicy::CatchUp))
        .unwrap();
    let f = catch.tick(T0 + 5 * HOUR);
    assert_eq!(f.len(), 5);
    assert_eq!(f[0].fire_time, T0 + HOUR);
    assert_eq!(f[4].fire_time, T0 + 5 * HOUR);
}

#[test]
fn no_duplicate_on_concurrent_tick() {
    let mut s = Scheduler::new();
    s.add_def(def("j1", "t1", Schedule::Hourly, MisfirePolicy::CatchUp))
        .unwrap();
    assert_eq!(s.tick(T0 + 2 * HOUR).len(), 2);
    // same now again: nothing re-fires.
    assert_eq!(s.tick(T0 + 2 * HOUR).len(), 0);
    // later now fires only the new slot, never the old ones.
    let f = s.tick(T0 + 3 * HOUR);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].fire_time, T0 + 3 * HOUR);
    assert_eq!(s.fired_keys().len(), 3);
}

#[test]
fn no_duplicate_on_restart() {
    let mut s1 = Scheduler::new();
    s1.add_def(def("j1", "t1", Schedule::Hourly, MisfirePolicy::CatchUp))
        .unwrap();
    assert_eq!(s1.tick(T0 + 2 * HOUR).len(), 2);
    let state = s1.export_state();
    // restart: reload persisted state, re-tick at the same now -> no duplicate.
    let mut s2 = Scheduler::import_state(&state).unwrap();
    assert_eq!(s2.tick(T0 + 2 * HOUR).len(), 0);
    assert_eq!(s2.fired_keys().len(), 2);
    // a genuinely new slot after restart fires exactly once.
    let f = s2.tick(T0 + 3 * HOUR);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].fire_time, T0 + 3 * HOUR);
}

#[tokio::test]
async fn tokio_driver_triggers_and_dedups() {
    let mut s = Scheduler::new();
    s.add_def(def("j1", "t1", Schedule::Hourly, MisfirePolicy::CatchUp))
        .unwrap();
    let sched = Arc::new(Mutex::new(s));
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(T0 + HOUR));
    let collected = Arc::new(Mutex::new(Vec::new()));
    let sink = collected.clone();
    run_interval(
        sched.clone(),
        clock,
        std::time::Duration::from_millis(5),
        3,
        move |f| {
            let sink = sink.clone();
            // block_in_place-free: push synchronously via try_lock (single-threaded runtime).
            sink.try_lock().unwrap().push(f);
        },
    )
    .await;
    let fires = collected.lock().await;
    // first tick fires the due slot; subsequent ticks dedup via idempotency key.
    assert_eq!(fires.len(), 1);
    assert_eq!(fires[0].fire_time, T0 + HOUR);
    let audit = sched.lock().await.audit().to_vec();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["event"], "fire");
}
