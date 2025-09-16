//! Shutdown-aware component trait.

use async_trait::async_trait;
use std::time::Duration;

/// Trait for components that can be gracefully shut down.
///
/// Components implementing this trait can be registered with the
/// `ShutdownCoordinator` for orderly shutdown when the process
/// receives a termination signal.
#[async_trait]
pub trait ShutdownAware: Send + Sync {
    /// Returns the component name for logging purposes.
    fn name(&self) -> &str;

    /// Gracefully shut down the component.
    ///
    /// The implementation should:
    /// 1. Stop accepting new work
    /// 2. Complete in-progress work (within the timeout)
    /// 3. Release resources
    ///
    /// # Arguments
    ///
    /// * `timeout` - Maximum time to wait for shutdown to complete.
    ///               If exceeded, the shutdown coordinator will proceed
    ///               to the next component.
    async fn shutdown(&self, timeout: Duration);
}

/// A simple wrapper for any sync shutdown function.
pub struct FnComponent<F> {
    name: String,
    shutdown_fn: F,
}

impl<F> FnComponent<F>
where
    F: Fn() + Send + Sync,
{
    pub fn new(name: impl Into<String>, shutdown_fn: F) -> Self {
        Self {
            name: name.into(),
            shutdown_fn,
        }
    }
}

#[async_trait]
impl<F> ShutdownAware for FnComponent<F>
where
    F: Fn() + Send + Sync,
{
    fn name(&self) -> &str {
        &self.name
    }

    async fn shutdown(&self, _timeout: Duration) {
        (self.shutdown_fn)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_fn_component_shutdown() {
        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();

        let component = FnComponent::new("test", move || {
            called_clone.store(true, Ordering::SeqCst);
        });

        assert_eq!(component.name(), "test");
        component.shutdown(Duration::from_secs(1)).await;
        assert!(called.load(Ordering::SeqCst));
    }
}
