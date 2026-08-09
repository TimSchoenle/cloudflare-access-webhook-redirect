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
    let boot = cloudflare_access_webhook_redirect::config::load_watched::<Config>()?;

    // Both are process-global and installed once, which is why `telemetry.*` is the one block
    // a configuration reload cannot apply.
    setup_tracing(boot.value.telemetry())?;
    // Prevents the process from exiting until all events are sent
    let _sentry = setup_sentry(boot.value.telemetry());

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

fn setup_tracing(telemetry: &TelemetryConfig) -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_filter(filter::LevelFilter::from_level(*telemetry.log_level())),
        )
        .init();

    Ok(())
}

fn setup_sentry(telemetry: &TelemetryConfig) -> Option<ClientInitGuard> {
    match telemetry.sentry_dsn() {
        Some(dsn) => Some(sentry::init((
            dsn.expose_secret(),
            sentry::ClientOptions {
                release: sentry::release_name!(),
                attach_stacktrace: true,
                ..Default::default()
            },
        ))),
        None => {
            info!("telemetry.sentry_dsn not set, skipping Sentry setup");
            None
        }
    }
}
