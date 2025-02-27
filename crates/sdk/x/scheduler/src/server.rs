use crate::scheduler_account::SchedulerRef;
use evolve_core::Environment;
use evolve_ns::resolve_name;
use evolve_server_core::{BeginBlocker, EndBlocker};

pub struct SchedulerBeginBlocker;
impl<B> BeginBlocker<B> for SchedulerBeginBlocker {
    fn begin_block(_block: &B, env: &mut dyn Environment) {
        must_get_scheduler(env).schedule_begin_block(env).unwrap()
    }
}

pub struct SchedulerEndBlocker;

impl EndBlocker for SchedulerEndBlocker {
    fn end_block(env: &mut dyn Environment) {
        must_get_scheduler(env).schedule_end_block(env).unwrap()
    }
}
fn must_get_scheduler(env: &dyn Environment) -> SchedulerRef {
    let scheduler_id = resolve_name("scheduler".to_string(), env)
        .unwrap()
        .expect("expected scheduler account to be resolved");

    SchedulerRef::from(scheduler_id)
}
