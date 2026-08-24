//! The binary.
//!
//! What cannot live in the library: the two process-global installs, the report a failed boot
//! prints before any logging exists, and the supervisor that re-runs `serve` from a fresh load
//! whenever a watched file changes. The proxy itself is
//! [`cloudflare_access_webhook_redirect`].

use std::sync::Arc;

use cloudflare_access_webhook_redirect::Result;
use cloudflare_access_webhook_redirect::config::{Config, TelemetryConfig};
use cloudflare_access_webhook_redirect::data::WebHookData;
use cloudflare_access_webhook_redirect::error::Error;
use cloudflare_access_webhook_redirect::server::Server;
use cloudflare_access_webhook_redirect::shutdown::install_shutdown;
use secrecy::ExposeSecret;
use sentry::ClientInitGuard;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, filter};

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

    // Both are process-global and installed once, which is why `telemetry.*` is the one block
    // a configuration reload cannot apply.
    setup_tracing(boot.value.telemetry());
    let _sentry = setup_sentry(boot.value.telemetry());

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

/// Installs the global subscriber at `telemetry.log_level`.
///
/// Once per process. A second call would panic, which is why a reload never reaches it.
fn setup_tracing(telemetry: &TelemetryConfig) {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_filter(filter::LevelFilter::from_level(*telemetry.log_level())),
        )
        .init();
}

/// Initialises Sentry, or returns `None` when `telemetry.sentry_dsn` is unset.
///
/// The guard has to outlive everything that can report: dropping it flushes the queue, and a
/// process that exits without it loses the events for the failure that ended it.
fn setup_sentry(telemetry: &TelemetryConfig) -> Option<ClientInitGuard> {
    let Some(dsn) = telemetry.sentry_dsn() else {
        info!("telemetry.sentry_dsn not set, skipping Sentry setup");
        return None;
    };

    Some(sentry::init((
        dsn.expose_secret(),
        sentry::ClientOptions {
            release: sentry::release_name!(),
            attach_stacktrace: true,
            ..Default::default()
        },
    )))
}
