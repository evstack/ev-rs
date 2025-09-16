//! Signal handling for graceful shutdown.

use tokio::sync::watch;

/// Signal handler that listens for SIGTERM and SIGINT.
///
/// Sends a notification when a shutdown signal is received,
/// allowing components to begin graceful shutdown.
pub struct SignalHandler {
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl SignalHandler {
    /// Create a new signal handler.
    pub fn new() -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            shutdown_tx,
            shutdown_rx,
        }
    }

    /// Get a receiver that will be notified when shutdown is requested.
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.shutdown_rx.clone()
    }

    /// Start listening for shutdown signals.
    ///
    /// This spawns a background task that waits for SIGTERM or SIGINT.
    /// When received, all subscribers will be notified.
    pub fn start(&self) {
        let tx = self.shutdown_tx.clone();

        tokio::spawn(async move {
            let result = wait_for_signal().await;
            log::info!("Received shutdown signal: {}", result);
            let _ = tx.send(true);
        });
    }

    /// Manually trigger shutdown (useful for testing or programmatic shutdown).
    pub fn trigger(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Check if shutdown has been requested.
    pub fn is_shutdown_requested(&self) -> bool {
        *self.shutdown_rx.borrow()
    }
}

impl Default for SignalHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Wait for a shutdown signal (SIGTERM or SIGINT).
///
/// Returns the name of the signal that was received.
#[cfg(unix)]
async fn wait_for_signal() -> &'static str {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm = signal(SignalKind::terminate()).expect("Failed to register SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("Failed to register SIGINT handler");

    tokio::select! {
        _ = sigterm.recv() => "SIGTERM",
        _ = sigint.recv() => "SIGINT",
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() -> &'static str {
    use tokio::signal::ctrl_c;
    ctrl_c().await.expect("Failed to register Ctrl+C handler");
    "Ctrl+C"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_manual_trigger() {
        let handler = SignalHandler::new();
        let mut rx = handler.subscribe();

        assert!(!handler.is_shutdown_requested());

        handler.trigger();

        // Wait for the notification
        rx.changed().await.unwrap();
        assert!(handler.is_shutdown_requested());
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let handler = SignalHandler::new();
        let mut rx1 = handler.subscribe();
        let mut rx2 = handler.subscribe();

        handler.trigger();

        rx1.changed().await.unwrap();
        rx2.changed().await.unwrap();

        assert!(*rx1.borrow());
        assert!(*rx2.borrow());
    }
}
