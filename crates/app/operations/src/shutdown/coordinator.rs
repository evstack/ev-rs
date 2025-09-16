//! Shutdown coordinator for multi-component shutdown orchestration.

use crate::shutdown::components::ShutdownAware;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Coordinates graceful shutdown across multiple components.
///
/// Components are shut down in LIFO (last-in-first-out) order,
/// which typically means:
/// - Public-facing services (RPC) shut down first
/// - Core services shut down last
///
/// This ensures that no new requests are accepted while
/// internal processing completes.
pub struct ShutdownCoordinator {
    components: Mutex<Vec<Arc<dyn ShutdownAware>>>,
    default_timeout: Duration,
}

impl ShutdownCoordinator {
    /// Create a new shutdown coordinator with the given default timeout.
    pub fn new(default_timeout: Duration) -> Self {
        Self {
            components: Mutex::new(Vec::new()),
            default_timeout,
        }
    }

    /// Register a component for shutdown.
    ///
    /// Components are shut down in reverse order of registration (LIFO).
    pub async fn register(&self, component: Arc<dyn ShutdownAware>) {
        let mut components = self.components.lock().await;
        log::debug!("Registered component for shutdown: {}", component.name());
        components.push(component);
    }

    /// Perform graceful shutdown of all registered components.
    ///
    /// Components are shut down in reverse order of registration.
    /// Each component gets the default timeout to complete shutdown.
    pub async fn shutdown(&self) {
        let components = {
            let mut guard = self.components.lock().await;
            std::mem::take(&mut *guard)
        };

        if components.is_empty() {
            log::info!("No components registered for shutdown");
            return;
        }

        log::info!(
            "Beginning graceful shutdown of {} components",
            components.len()
        );

        // Shutdown in reverse order (LIFO)
        for component in components.into_iter().rev() {
            let name = component.name().to_string();
            log::info!("Shutting down component: {}", name);

            let shutdown_future = component.shutdown(self.default_timeout);
            match tokio::time::timeout(self.default_timeout, shutdown_future).await {
                Ok(()) => {
                    log::info!("Component '{}' shut down successfully", name);
                }
                Err(_) => {
                    log::warn!(
                        "Component '{}' shutdown timed out after {:?}",
                        name,
                        self.default_timeout
                    );
                }
            }
        }

        log::info!("Graceful shutdown complete");
    }

    /// Get the number of registered components.
    pub async fn component_count(&self) -> usize {
        self.components.lock().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shutdown::components::FnComponent;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_lifo_shutdown_order() {
        let order = Arc::new(AtomicUsize::new(0));
        let coordinator = ShutdownCoordinator::new(Duration::from_secs(1));

        // Register components
        for i in 0..3 {
            let order_clone = order.clone();
            let expected_order = 2 - i; // LIFO: last registered = first shutdown

            let component = Arc::new(FnComponent::new(format!("component-{}", i), move || {
                let actual = order_clone.fetch_add(1, Ordering::SeqCst);
                assert_eq!(
                    actual, expected_order,
                    "Component {} shutdown out of order",
                    i
                );
            }));

            coordinator.register(component).await;
        }

        assert_eq!(coordinator.component_count().await, 3);

        coordinator.shutdown().await;

        assert_eq!(order.load(Ordering::SeqCst), 3);
        assert_eq!(coordinator.component_count().await, 0);
    }

    #[tokio::test]
    async fn test_empty_shutdown() {
        let coordinator = ShutdownCoordinator::new(Duration::from_secs(1));
        coordinator.shutdown().await;
        // Should not panic
    }

    #[tokio::test]
    async fn test_timeout_handling() {
        use async_trait::async_trait;

        struct SlowComponent;

        #[async_trait]
        impl ShutdownAware for SlowComponent {
            fn name(&self) -> &str {
                "slow"
            }

            async fn shutdown(&self, _timeout: Duration) {
                // Sleep longer than the timeout
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }

        let coordinator = ShutdownCoordinator::new(Duration::from_millis(10));
        coordinator.register(Arc::new(SlowComponent)).await;

        // Should complete despite the slow component
        let start = std::time::Instant::now();
        coordinator.shutdown().await;
        let elapsed = start.elapsed();

        // Should have timed out quickly, not waited 10 seconds
        assert!(elapsed < Duration::from_secs(1));
    }
}
