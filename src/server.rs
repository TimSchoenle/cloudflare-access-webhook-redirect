use crate::Result;
use crate::data::WebHookData;
use crate::routes::{health_check, redirect};
use actix_web::{App, HttpServer, web};
use derive_new::new;
use tokio_util::sync::CancellationToken;
use tracing_actix_web::TracingLogger;

#[derive(new)]
pub struct Server {
    host: String,
    port: u16,
}

impl Server {
    /// Serve until `shutdown` is cancelled — by a termination signal, or by the reload
    /// supervisor because the configuration changed and this generation is being replaced.
    ///
    /// Returns only once the listener has stopped and in-flight requests have drained, which
    /// is what lets the next generation bind the same address.
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
        let server = HttpServer::new(move || {
            App::new()
                .wrap(TracingLogger::default())
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
