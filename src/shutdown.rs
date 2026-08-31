//! The process-wide shutdown signal.
//!
//! One token, cancelled by the first termination signal to arrive, held by whatever is running
//! at the time. The reload supervisor passes a child of it to each generation of the runtime,
//! so a rebuild stops the previous listener the same way a `SIGTERM` does.

use tokio_util::sync::CancellationToken;

/// Spawn the signal listener and return the token it cancels.
///
/// `SIGTERM` matters as much as `SIGINT` here: it is what a container runtime and a Kubernetes
/// pod eviction send, and dropping it would mean every graceful shutdown ends as a kill.
#[must_use]
pub fn install_shutdown() -> CancellationToken {
    let token = CancellationToken::new();

    let signalled = token.clone();
    tokio::spawn(async move {
        wait_for_signal().await;
        info!("Shutdown signal received, stopping");
        signalled.cancel();
    });

    token
}

#[cfg(unix)]
async fn wait_for_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    match signal(SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                () = wait_for_ctrl_c() => {},
                _ = terminate.recv() => {},
            }
        }
        Err(e) => {
            // Losing SIGTERM is worth a line in the log, but it is not worth refusing to
            // start: Ctrl-C alone still stops an interactive run.
            warn!("Failed to install the SIGTERM handler, listening for Ctrl-C only: {e}");
            wait_for_ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() {
    wait_for_ctrl_c().await;
}

async fn wait_for_ctrl_c() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        error!("Failed to listen for Ctrl-C, shutdown must be forced: {e}");
        // Never resolve: cancelling the token here would shut the service down over a
        // registration failure that has nothing to do with the operator's intent.
        std::future::pending::<()>().await;
    }
}
