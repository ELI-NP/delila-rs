//! Unified shutdown handling for DELILA components
//!
//! # Design Principles (KISS)
//! - Single function to setup signal handlers with broadcast channel
//! - Returns (sender, receiver) for component use
//! - Components call run(shutdown_rx) as before
//!
//! Handles **both SIGINT (Ctrl+C) and SIGTERM** (TODO 58 M8): `pkill`,
//! `systemctl stop` and the stop scripts send SIGTERM, which previously
//! killed components without flushing — losing up to a BufWriter's worth of
//! recorder data plus the file footer. (Same failure class as the
//! `sigterm_handler_required` memory for CAEN-handle binaries.)

use tokio::signal;
use tokio::sync::broadcast;
use tracing::info;

/// Shutdown signal type (unit type, just signals "shutdown now")
pub type ShutdownSignal = ();

/// Shutdown channel sender
pub type ShutdownSender = broadcast::Sender<ShutdownSignal>;

/// Shutdown channel receiver
pub type ShutdownReceiver = broadcast::Receiver<ShutdownSignal>;

/// Wait for either SIGINT (Ctrl+C) or SIGTERM. Returns a short label naming
/// the signal that fired. On non-unix targets only Ctrl+C is available.
async fn wait_for_termination() -> &'static str {
    #[cfg(unix)]
    {
        let mut sigterm = match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                // Registration can only realistically fail under exotic
                // conditions; degrade to Ctrl+C-only rather than panicking.
                tracing::warn!(error = %e, "Failed to register SIGTERM handler — Ctrl+C only");
                signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
                return "Ctrl+C";
            }
        };
        tokio::select! {
            _ = signal::ctrl_c() => "Ctrl+C",
            _ = sigterm.recv() => "SIGTERM",
        }
    }
    #[cfg(not(unix))]
    {
        signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
        "Ctrl+C"
    }
}

/// Setup shutdown handling with Ctrl+C / SIGTERM
///
/// Creates a broadcast channel and spawns a task that sends on the first
/// termination signal. Returns (sender, receiver) - caller uses receiver for
/// their component, and can clone sender if needed for additional shutdown
/// triggers.
///
/// # Example
/// ```ignore
/// let (_shutdown_tx, shutdown_rx) = setup_shutdown();
/// component.run(shutdown_rx).await?;
/// ```
pub fn setup_shutdown() -> (ShutdownSender, ShutdownReceiver) {
    let (tx, rx) = broadcast::channel::<ShutdownSignal>(1);

    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let which = wait_for_termination().await;
        info!("{which} received, initiating shutdown");
        let _ = tx_clone.send(());
    });

    (tx, rx)
}

/// Setup shutdown with custom message
///
/// Same as `setup_shutdown` but allows custom log message.
pub fn setup_shutdown_with_message(message: &'static str) -> (ShutdownSender, ShutdownReceiver) {
    let (tx, rx) = broadcast::channel::<ShutdownSignal>(1);

    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let _ = wait_for_termination().await;
        println!("\n{}", message);
        let _ = tx_clone.send(());
    });

    (tx, rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_shutdown_channel_creation() {
        let (tx, mut rx) = broadcast::channel::<ShutdownSignal>(1);

        // Sending should work
        tx.send(()).unwrap();

        // Receiving should work
        let result = rx.recv().await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_type_aliases() {
        // Just verify the type aliases compile correctly
        fn _takes_sender(_: ShutdownSender) {}
        fn _takes_receiver(_: ShutdownReceiver) {}
    }
}
