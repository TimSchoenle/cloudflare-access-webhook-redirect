//! The listener.
//!
//! One [`Server`] per generation of the runtime. Routing is `crate::routes` and the signal it
//! stops on is [`shutdown`](crate::shutdown); binding and draining are here.

use crate::Result;
use crate::data::WebHookData;
use crate::routes::{health_check, redirect};
use crate::telemetry;
use actix_web::middleware::Condition;
use actix_web::{App, HttpServer, web};
use derive_new::new;
use tokio_util::sync::CancellationToken;
use tracing_actix_web::TracingLogger;

/// One generation of the listener, bound to `server.host` and `server.port`.
///
/// A reload constructs a new one, so the address is fixed for the life of a listener.
#[derive(new)]
pub struct Server {
    host: String,
    port: u16,
}

impl Server {
    /// Serves until `shutdown` is cancelled.
    ///
    /// Cancellation reaches it from a termination signal, or from the reload supervisor
    /// replacing this generation. Returns only once the listener has stopped and in-flight
    /// requests have drained, which is what lets the next generation bind the same address.
    ///
    /// # Errors
    /// Returns [`Error::IoError`](crate::error::Error::IoError) if the address is already taken or
    /// the process may not bind it, and if the running listener stops with an error of its own.
    pub async fn run_until_stopped(
        &self,
        web_hook_data: WebHookData,
        shutdown: CancellationToken,
    ) -> Result<()> {
        info!(
            "Starting server on {}:{} with allowed paths {:#?}",
            self.host,
            self.port,
            web_hook_data.allowed_paths().allowed_paths()
        );

        let web_hook_data = web::Data::new(web_hook_data);
        // Resolved once, outside the factory: what it reports is a property of the client
        // installed at boot, while the factory runs once per worker thread. `Condition` rather
        // than an `Option`, which actix cannot wrap — the middleware value exists either way and
        // is simply never invoked while the condition is false.
        let sentry = telemetry::actix_middleware();
        let reporting = sentry.is_some();
        let sentry = sentry.unwrap_or_default();
        let server = HttpServer::new(move || {
            App::new()
                .wrap(TracingLogger::default())
                // Registered last and therefore outermost, which is the load-bearing half: the
                // per-request hub has to be bound before anything else runs, or breadcrumbs from
                // concurrently served requests all land on the main hub and every issue arrives
                // with a trail belonging to whoever else was in flight.
                .wrap(Condition::new(reporting, sentry.clone()))
                .app_data(web_hook_data.clone())
                .configure(health_check::get_config)
                .configure(redirect::get_config)
        })
        // Signals are handled by `crate::shutdown`, which the supervisor also stops us
        // through. Leaving actix's own handler installed would race it: a `SIGTERM` would
        // stop the listener out from under a supervisor that then rebuilt it.
        .disable_signals()
        .bind((self.host.clone(), self.port))?
        .run();

        let handle = server.handle();
        let stopper = tokio::spawn(async move {
            shutdown.cancelled().await;
            handle.stop(true).await;
        });

        let result = server.await;
        // The server can also stop on its own; the listener has no reason to outlive it.
        stopper.abort();

        result?;

        Ok(())
    }
}
