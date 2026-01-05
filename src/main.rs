use std::env;
use std::str::FromStr;

use reqwest_middleware::ClientBuilder;
use reqwest_tracing::{SpanBackendWithUrl, TracingMiddleware};
use sentry::ClientInitGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, filter};

use cloudflare_access_webhook_redirect::Result;
use cloudflare_access_webhook_redirect::config::Config;
use cloudflare_access_webhook_redirect::data::WebHookData;
use cloudflare_access_webhook_redirect::server::Server;

#[macro_use]
extern crate tracing;

const ENV_SENTRY_DSN: &str = "SENTRY_DSN";
const ENV_LOG_LEVEL: &str = "LOG_LEVEL";

const DEFAULT_LOG_LEVEL: &str = "info";

#[tokio::main]
async fn main() -> Result<()> {
    setup_tracing()?;

    // Prevents the process from exiting until all events are sent
    let _sentry = setup_sentry();

    let server;
    let web_hook_data;
    {
        let config = Config::get_configuration()?;

        server = Server::new(config.server.host.to_string(), config.server.port);
        let client = ClientBuilder::new(reqwest::Client::new())
            .with(TracingMiddleware::<SpanBackendWithUrl>::new())
            .build();

        web_hook_data = WebHookData::new(
            client,
            config.webhook.target_base.clone(),
            config.webhook.paths.clone().try_into()?,
            config.cloudflare.client_id.clone(),
            config.cloudflare.client_secret.clone(),
        )?;
    }

    server.run_until_stopped(web_hook_data).await?;

    Ok(())
}

fn setup_tracing() -> Result<()> {
    let level = env::var(ENV_LOG_LEVEL).unwrap_or_else(|_| DEFAULT_LOG_LEVEL.to_string());
    let level = tracing::Level::from_str(&level)?;

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter::LevelFilter::from_level(level)))
        .init();

    Ok(())
}

fn setup_sentry() -> Option<ClientInitGuard> {
    match env::var(ENV_SENTRY_DSN) {
        Ok(dns) => Some(sentry::init((
            dns,
            sentry::ClientOptions {
                release: sentry::release_name!(),
                attach_stacktrace: true,
                ..Default::default()
            },
        ))),
        Err(_) => {
            info!("{ENV_SENTRY_DSN} not set, skipping Sentry setup");
            None
        }
    }
}
