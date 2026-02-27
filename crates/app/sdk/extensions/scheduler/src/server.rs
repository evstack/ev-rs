use crate::scheduler_account::SchedulerRef;
use evolve_core::{AccountId, Environment, EnvironmentQuery};
use evolve_stf_traits::{BeginBlocker, EndBlocker};

#[derive(Clone)]
pub struct SchedulerBeginBlocker {
    scheduler_id: AccountId,
}

impl SchedulerBeginBlocker {
    pub const fn new(scheduler_id: AccountId) -> Self {
        Self { scheduler_id }
    }
}
impl<B> BeginBlocker<B> for SchedulerBeginBlocker {
    fn begin_block(&self, _block: &B, env: &mut dyn Environment) {
        let _ = must_get_scheduler(self.scheduler_id, env).schedule_begin_block(env);
    }
}

#[derive(Clone)]
pub struct SchedulerEndBlocker {
    scheduler_id: AccountId,
}

impl SchedulerEndBlocker {
    pub const fn new(scheduler_id: AccountId) -> Self {
        Self { scheduler_id }
    }
}

impl EndBlocker for SchedulerEndBlocker {
    fn end_block(&self, env: &mut dyn Environment) {
        let _ = must_get_scheduler(self.scheduler_id, env).schedule_end_block(env);
    }
}
fn must_get_scheduler(scheduler_id: AccountId, _env: &mut dyn EnvironmentQuery) -> SchedulerRef {
    SchedulerRef::from(scheduler_id)
}
