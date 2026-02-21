use commonware_consensus::Reporter;
use std::marker::PhantomData;

/// A reporter that logs consensus activity events.
///
/// Generic over the activity type so it works with any consensus scheme.
/// In production, this would handle finalization callbacks:
/// executing blocks through the STF, committing state changes,
/// and updating chain state. For now, it logs activity.
#[derive(Clone)]
pub struct EvolveReporter<A> {
    _phantom: PhantomData<fn() -> A>,
}

impl<A> EvolveReporter<A> {
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<A> Default for EvolveReporter<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Clone + Send + 'static> Reporter for EvolveReporter<A> {
    type Activity = A;

    async fn report(&mut self, _activity: Self::Activity) {
        // In production, this would:
        // 1. Extract finalization events (Finalization variant)
        // 2. Look up the full block from pending_blocks by digest
        // 3. Execute through STF
        // 4. Commit state changes to storage
        // 5. Update chain height and last_hash
        tracing::debug!("reporter: received consensus activity");
    }
}
