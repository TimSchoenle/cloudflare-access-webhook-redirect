//! The binary.
//!
//! What cannot live in the library: the two process-global installs, the report a failed boot
//! prints before any logging exists, and the supervisor that re-runs `serve` from a fresh load
//! whenever a watched file changes. The proxy itself is
//! [`cloudflare_access_webhook_redirect`].

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
        // Nothing is installed yet, so this is stderr rather than a log line. It is also the one
        // moment the report is worth the most: a boot that fails on a missing or doubly-supplied
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

/// Builds one generation of the proxy from `config` and serves it.
///
/// Everything a configuration change replaces is constructed here: the HTTP client, the
/// compiled path patterns and the parsed credentials. Returns once `shutdown` is cancelled
/// and the listener has drained.
async fn serve(config: Arc<Config>, shutdown: CancellationToken) -> Result<()> {
    let web_hook_data = WebHookData::new(
        reqwest::Client::new(),
        config.webhook().target_base().clone(),
        config.webhook().paths().clone().try_into()?,
        config.cloudflare().client_id(),
        config.cloudflare().client_secret(),
    )?;

    Server::new(config.server().host().clone(), *config.server().port())
        .run_until_stopped(web_hook_data, shutdown)
        .await
}

/// Logs which layer supplied each configuration key, once, at boot.
///
/// The report names the files and variables each key was read from and carries no configuration
/// value, which is what makes it safe to log at all.
fn log_config_sources() {
    match cloudflare_access_webhook_redirect::config::explain() {
        // At `debug` because it is a dozen lines an operator only wants when something is wrong.
        Ok(explanation) => debug!("configuration sources:\n{explanation}"),
        // Swallowed: a report that cannot be assembled is not worth failing a boot the
        // configuration itself survived.
        Err(error) => debug!("could not explain the configuration sources: {error}"),
    }
}
