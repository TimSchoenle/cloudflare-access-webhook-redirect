use std::sync::Arc;

use cloudflare_access_webhook_redirect::Result;
use cloudflare_access_webhook_redirect::config::Config;
use cloudflare_access_webhook_redirect::data::WebHookData;
use cloudflare_access_webhook_redirect::error::Error;
use cloudflare_access_webhook_redirect::server::Server;
use cloudflare_access_webhook_redirect::shutdown::install_shutdown;
use cloudflare_access_webhook_redirect::telemetry;
use tokio_util::sync::CancellationToken;

#[macro_use]
extern crate tracing;

#[tokio::main]
async fn main() -> Result<()> {
    let boot = match cloudflare_access_webhook_redirect::config::load_watched::<Config>() {
        Ok(boot) => boot,
        // Nothing is installed yet, so this is stderr rather than a log line — and it is the one
        // moment the report is worth the most. A boot that fails on a missing or doubly-supplied
        // key says which key; the explanation says which files and variables were read looking
        // for it, which is the half that turns a failed deploy into a fixed mount path.
        Err(error) => {
            if let Ok(explanation) = cloudflare_access_webhook_redirect::config::explain() {
                eprintln!("{explanation}");
            }
            return Err(error.into());
        }
    };

    // The subscriber and the Sentry client are process-global and installed once, which is why
    // `telemetry.*` is the one block a configuration reload cannot apply. The guard is bound for
    // the rest of `main`: dropping it closes the client, and with it the flush that gets the last
    // events of a terminating process out.
    let telemetry_guard = telemetry::init(boot.value.telemetry())?;

    // After the subscriber exists, or the line goes nowhere — and "is Sentry actually on in this
    // pod" is the first question an operator asks.
    if telemetry_guard.reporting() {
        let sentry = boot.value.telemetry().sentry();
        info!(
            traces_sample_rate = sentry.traces_sample_rate(),
            http_transactions = sentry.http_transactions(),
            send_default_pii = sentry.send_default_pii(),
            "Sentry reporting enabled"
        );
    }

    log_config_sources();

    let shutdown = install_shutdown();

    // Rebuilds the proxy whenever the files the configuration came from change, so a rotated
    // Cloudflare Access service token is picked up without a restart. A reload that cannot be
    // loaded leaves the running service exactly as it is.
    terrace_config::reload::run(
        (boot.value, boot.sources),
        &shutdown,
        || {
            cloudflare_access_webhook_redirect::config::load_watched::<Config>()
                .map(|loaded| (loaded.value, loaded.sources))
                .map_err(Error::from)
        },
        serve,
    )
    .await
}

/// Build and run everything a configuration change rebuilds: the HTTP client, the compiled path
/// patterns, the injected credentials and the listener.
///
/// Returns once `shutdown` is cancelled and the listener has drained.
async fn serve(config: Arc<Config>, shutdown: CancellationToken) -> Result<()> {
    let web_hook_data = WebHookData::new(
        reqwest::Client::new(),
        config.webhook().target_base().clone(),
        config.webhook().paths().clone().try_into()?,
        config.cloudflare().client_id().clone(),
        config.cloudflare().client_secret().clone(),
    )?;

    Server::new(config.server().host().to_string(), *config.server().port())
        .run_until_stopped(web_hook_data, shutdown)
        .await
}

/// Log which layer supplied each configuration key, once, at boot.
///
/// At `debug` because it is a dozen lines an operator only wants when something is wrong, and
/// never at a lower bar than the log they are already reading. It carries no configuration
/// value — only the names of the files and variables — so it is safe at any level, and a report
/// that cannot be assembled is not worth failing a boot the configuration itself survived.
fn log_config_sources() {
    match cloudflare_access_webhook_redirect::config::explain() {
        Ok(explanation) => debug!("configuration sources:\n{explanation}"),
        Err(error) => debug!("could not explain the configuration sources: {error}"),
    }
}
