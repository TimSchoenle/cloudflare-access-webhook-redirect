//! The `tracing` subscriber, and the optional Sentry client that shares its record stream.
//!
//! Both are process-global and installed once, before the reloadable runtime exists, which is why
//! `[telemetry]` is the one configuration block a reload cannot apply.
//!
//! Sentry is off unless `telemetry.sentry.enabled` is set, and then only with a DSN: a client
//! that reports nowhere is a deployment believing it has error reporting when it has none, so the
//! combination fails the boot rather than being logged and ignored.
//!
//! Three sinks, all fed from the one client [`init`] installs:
//! - **`tracing`** — `sentry_layer` turns records into issues and breadcrumbs, under the
//!   thresholds in [`SentryConfig`].
//! - **panics** — the SDK's own hook, added by `sentry::init`.
//! - **HTTP** — [`actix_middleware`], mounted by [`Server`].
//!
//! The extern crate is always spelled `::sentry`; the bare path is ambiguous with this crate's
//! own `config::sentry`, the private module behind [`SentryConfig`].
//!
//! [`Server`]: crate::server::Server

use std::sync::OnceLock;
use std::time::Duration;

use ::sentry::integrations::tracing::{EventFilter, SentryLayer, default_span_filter};
use secrecy::ExposeSecret;
use tracing::Level;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, filter};

use crate::config::{SentryConfig, SentryLevel, TelemetryConfig};
use crate::error::Error;

/// What [`Server`] mounts, decided once at boot.
///
/// Process-global because the client it describes is: `sentry::init` binds one client to
/// `Hub::main()` for the lifetime of the process. Reading `telemetry.sentry` out of the
/// configuration the server was built from would be the wrong source — that value is rebuilt on
/// every reload, and this half of the service is not, so a reloaded generation could mount
/// middleware for a client that was never installed.
///
/// Unset until [`init`] runs, which `main` does before it builds a listener.
///
/// [`Server`]: crate::server::Server
static HTTP: OnceLock<HttpOptions> = OnceLock::new();

/// The two independent halves of the HTTP integration.
#[derive(Debug, Clone, Copy)]
struct HttpOptions {
    /// A client is bound, so requests get their own hub and their request metadata.
    active: bool,
    /// Additionally start one transaction per request. Whether that transaction is *kept* is the
    /// sampler's decision, not this one.
    transactions: bool,
}

/// Keeps the Sentry client alive, and flushes what it has queued on drop.
///
/// Returned rather than leaked into a static, because a static is never dropped: the flush that
/// gets the last events of a terminating process out happens here, bounded by
/// `telemetry.sentry.shutdown_timeout_secs`. Bind it for the lifetime of `main` — `let _ = …`
/// drops it immediately and closes the client before the proxy has served anything.
#[must_use = "dropping the guard closes the Sentry client and stops reporting"]
pub struct TelemetryGuard(Option<::sentry::ClientInitGuard>);

impl TelemetryGuard {
    /// Whether a Sentry client was installed. The log line saying so is `main`'s to emit, after
    /// the subscriber exists.
    #[must_use]
    pub fn reporting(&self) -> bool {
        self.0.is_some()
    }
}

impl std::fmt::Debug for TelemetryGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TelemetryGuard")
            .field(&self.reporting())
            .finish()
    }
}

/// Install the global `tracing` subscriber, and the Sentry client when one is configured.
///
/// # Errors
/// [`Error::Sentry`] when `telemetry.sentry.enabled` is set but unusable: no DSN, a DSN that does
/// not parse, or a sample rate outside `0.0..=1.0`. All three are configuration mistakes whose
/// only other outcome is a deployment that silently reports nothing.
pub fn init(telemetry: &TelemetryConfig) -> crate::Result<TelemetryGuard> {
    // Before the subscriber: the layer below reports onto the client this installs, and the SDK's
    // panic hook should be in place for anything the subscriber build itself does.
    let guard = init_sentry(telemetry.sentry())?;

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_filter(filter::LevelFilter::from(telemetry.log_level())),
        )
        .with(sentry_layer(telemetry.sentry()))
        .init();

    Ok(TelemetryGuard(guard))
}

/// The per-request hub and request-metadata middleware, or `None` when Sentry is off.
///
/// The hub is not optional decoration: without one per request, breadcrumbs from concurrently
/// served requests all land on the main hub and every issue arrives with a trail belonging to
/// whoever else was in flight.
///
/// `None` before [`init`] has run, so a listener built without telemetry — a test — mounts
/// nothing.
#[must_use]
pub fn actix_middleware() -> Option<::sentry::integrations::actix::Sentry> {
    let options = HTTP.get().copied()?;
    if !options.active {
        return None;
    }

    Some(
        ::sentry::integrations::actix::Sentry::builder()
            .start_transaction(options.transactions)
            // The proxy relays the target's status verbatim, so a 5xx here is the *target's*
            // failure and is already logged as one. What this crate is responsible for reaches
            // Sentry through the `tracing` layer instead, with the path and the method on it.
            .capture_server_errors(false)
            .finish(),
    )
}

/// The trace-continuation headers for the request currently in scope: `sentry-trace`, and
/// whatever else the SDK adds to that set later.
///
/// Empty when Sentry is off, so a caller can attach the result unconditionally.
///
/// This is the half of distributed tracing the inbound middleware cannot do. It *continues* a
/// trace it is handed; without these headers on the way out, the Cloudflare Access protected
/// service starts a second, unrelated trace and one webhook delivery reads as two.
#[must_use]
pub fn trace_headers() -> Vec<(&'static str, String)> {
    let mut headers = Vec::new();
    // `configure_scope` returns `()`, so the iterator has to be drained into a binding the
    // closure captures rather than returned through it.
    ::sentry::configure_scope(|scope| headers.extend(scope.iter_trace_propagation_headers()));
    headers
}

/// Install the process-wide Sentry client, or nothing when it is switched off.
fn init_sentry(config: &SentryConfig) -> crate::Result<Option<::sentry::ClientInitGuard>> {
    if !config.enabled() {
        record_http(HttpOptions {
            active: false,
            transactions: false,
        });
        return Ok(None);
    }

    // Empty is absent, not a value. `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__DSN=""` is what an
    // unfilled chart value or a compose pass-through produces, and it has to land on the message
    // below rather than on the parse error, which would send an operator looking at their URL.
    let dsn = config
        .dsn()
        .as_ref()
        .map(|dsn| dsn.expose_secret().trim())
        .filter(|dsn| !dsn.is_empty());
    let Some(dsn) = dsn else {
        return Err(Error::Sentry(
            "telemetry.sentry.enabled is set but telemetry.sentry.dsn is empty; nothing would be \
             reported. Supply the DSN, or turn the block off."
                .to_string(),
        ));
    };
    // Parsed here rather than through `ClientOptions::dsn`, which panics on a malformed value.
    // The error deliberately does not quote the DSN: it is a credential, and this message reaches
    // the log stream.
    let dsn = dsn.parse::<::sentry::types::Dsn>().map_err(|e| {
        Error::Sentry(format!(
            "telemetry.sentry.dsn is not a valid Sentry DSN ({e}); expected \
             https://<key>@<host>/<project>"
        ))
    })?;

    check_rate("sample_rate", config.sample_rate())?;
    check_rate("traces_sample_rate", config.traces_sample_rate())?;

    let release = config.release().clone().unwrap_or_else(|| {
        concat!(env!("CARGO_PKG_NAME"), "@", env!("CARGO_PKG_VERSION")).to_string()
    });

    let mut options = ::sentry::ClientOptions::new()
        .debug(config.debug())
        .sample_rate(config.sample_rate())
        .traces_sample_rate(config.traces_sample_rate())
        .max_breadcrumbs(config.max_breadcrumbs())
        .attach_stacktrace(config.attach_stacktraces())
        .send_default_pii(config.send_default_pii())
        .shutdown_timeout(Duration::from_secs(config.shutdown_timeout_secs()))
        .release(release)
        .environment(config.environment().clone())
        // Marks this crate's own frames as application code, so a stack trace opens on the
        // handler rather than on an actix internal.
        .in_app_include(vec![env!("CARGO_CRATE_NAME")]);
    options.dsn = Some(dsn);
    if let Some(server_name) = config.server_name().clone() {
        options = options.server_name(server_name);
    }

    // `init` runs the SDK's own defaults, which fill an unset `dsn`, `release` or `environment`
    // from `SENTRY_DSN`, `SENTRY_RELEASE` and `SENTRY_ENVIRONMENT`. All three are set above, and
    // that is the point: those variables are a second configuration channel that bypasses the
    // layered loader and its shadow-key rejection, and an already-set field is one they cannot
    // reach. What is left — `HTTP_PROXY`, `HTTPS_PROXY` and `SSL_VERIFY`, which the transport
    // reads whatever these options say — has no field to close it with, so it is declared in the
    // published configuration contract instead of being left for a chart to discover.
    let guard = ::sentry::init(options);

    record_http(HttpOptions {
        active: true,
        transactions: config.http_transactions(),
    });

    Ok(Some(guard))
}

/// The `tracing` layer feeding the client, or `None` when Sentry is off.
///
/// Sits under the subscriber's level filter, which is the one surprise worth knowing: a record
/// `telemetry.log_level` drops never reaches this layer, so tightening the log level to `warn`
/// silently removes every `info` breadcrumb.
fn sentry_layer<S>(config: &SentryConfig) -> Option<SentryLayer<S>>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    if !config.enabled() {
        return None;
    }

    let capture = config.capture_level();
    let breadcrumb = config.breadcrumb_level();

    let mut layer = ::sentry::integrations::tracing::layer()
        .event_filter(move |metadata| {
            let level = *metadata.level();
            if accepts(capture, level) {
                EventFilter::Event
            } else if accepts(breadcrumb, level) {
                EventFilter::Breadcrumb
            } else {
                EventFilter::Ignore
            }
        })
        // Not additionally gated on `traces_sample_rate`. Whether a span is *recorded* is the
        // sampler's decision, and it is the one that can honour an inherited trace: at rate `0.0`
        // the proxy starts no trace of its own but still continues one it was handed, which is
        // the whole of what makes a webhook delivery readable end to end. Gating span creation
        // here would cut that trace at this hop.
        .span_filter(default_span_filter);

    if config.span_attributes() {
        layer = layer.enable_span_attributes();
    }

    Some(layer)
}

/// Whether a record at `level` is at least as severe as `threshold`.
///
/// [`Level`] orders `ERROR` lowest, so "at least as severe" is `<=`.
fn accepts(threshold: SentryLevel, level: Level) -> bool {
    let threshold = match threshold {
        SentryLevel::Off => return false,
        SentryLevel::Error => Level::ERROR,
        SentryLevel::Warn => Level::WARN,
        SentryLevel::Info => Level::INFO,
        SentryLevel::Debug => Level::DEBUG,
        SentryLevel::Trace => Level::TRACE,
    };
    level <= threshold
}

fn check_rate(name: &str, rate: f32) -> crate::Result<()> {
    if (0.0..=1.0).contains(&rate) {
        Ok(())
    } else {
        Err(Error::Sentry(format!(
            "telemetry.sentry.{name} must be between 0.0 and 1.0, got {rate}"
        )))
    }
}

/// First writer wins, matching the client itself: a second `init` in one process is a test
/// harness, not a reconfiguration.
fn record_http(options: HttpOptions) {
    let _ = HTTP.set(options);
}

#[cfg(test)]
mod tests {
    use super::{SentryConfig, SentryLevel, accepts, actix_middleware, check_rate, init_sentry};
    use figment::providers::Format;
    use tracing::Level;

    /// The block as an operator would write it. `SentryConfig`'s fields are private to
    /// `config::sentry`, and deserialising is what the service does with them anyway.
    fn config(toml: &str) -> SentryConfig {
        figment::Figment::from(figment::providers::Toml::string(toml))
            .extract()
            .expect("the block parses")
    }

    /// The rejection [`init_sentry`] produced. Not `expect_err`, which needs a `Debug` on the
    /// success type, and `ClientInitGuard` has none.
    fn rejection(toml: &str, why: &str) -> crate::error::Error {
        match init_sentry(&config(toml)) {
            Err(error) => error,
            Ok(_) => panic!("{why}"),
        }
    }

    /// [`Level`] sorts `ERROR` *below* `TRACE`, so a severity threshold reads as `<=` and not
    /// `>=`. Inverting it turns `capture_level = "error"` into "capture everything", which is a
    /// bill rather than a compile error.
    #[test]
    fn a_threshold_accepts_only_levels_at_least_as_severe() {
        assert!(accepts(SentryLevel::Error, Level::ERROR));
        assert!(!accepts(SentryLevel::Error, Level::WARN));
        assert!(!accepts(SentryLevel::Error, Level::TRACE));

        assert!(accepts(SentryLevel::Info, Level::ERROR));
        assert!(accepts(SentryLevel::Info, Level::WARN));
        assert!(accepts(SentryLevel::Info, Level::INFO));
        assert!(!accepts(SentryLevel::Info, Level::DEBUG));

        for level in [
            Level::ERROR,
            Level::WARN,
            Level::INFO,
            Level::DEBUG,
            Level::TRACE,
        ] {
            assert!(!accepts(SentryLevel::Off, level));
            assert!(accepts(SentryLevel::Trace, level));
        }
    }

    #[test]
    fn a_sample_rate_outside_the_unit_interval_is_refused() {
        assert!(check_rate("sample_rate", 0.0).is_ok());
        assert!(check_rate("sample_rate", 1.0).is_ok());

        let error = check_rate("traces_sample_rate", 1.1).expect_err("above the interval");
        assert!(
            error.to_string().contains("traces_sample_rate"),
            "the error must name the key: {error}"
        );
        assert!(check_rate("sample_rate", -0.1).is_err());
    }

    /// The disabled path must install no client at all — not a client with an empty DSN, which
    /// still starts a transport thread and still queues events — and no middleware around it.
    #[test]
    fn disabled_installs_no_client_and_no_middleware() {
        let config = SentryConfig::default();
        assert!(!config.enabled());
        assert!(super::sentry_layer::<tracing_subscriber::Registry>(&config).is_none());

        assert!(
            init_sentry(&config).is_ok_and(|guard| guard.is_none()),
            "switched off installs no client and is not a failure"
        );
        assert!(actix_middleware().is_none());
    }

    /// A client that reports nowhere is a deployment believing it has error reporting when it
    /// has none, so `enabled` without a DSN fails the boot.
    ///
    /// Asserted through the error rather than by installing a client: `sentry::init` binds to
    /// `Hub::main()` for the lifetime of the *process*, so a test that reached it would decide
    /// what every other test in this binary observes.
    #[test]
    fn enabled_without_a_dsn_is_a_boot_failure() {
        let error = rejection("enabled = true", "a client with no DSN reports nowhere");

        assert!(error.to_string().contains("dsn"), "{error}");
    }

    /// A pass-through that resolved to nothing — `…__SENTRY__DSN=""` from a compose file, an
    /// unfilled chart value — must read as *absent* rather than as a DSN that fails to parse.
    /// The two produce very different messages, and only one sends the operator to the right
    /// place.
    #[test]
    fn an_empty_dsn_reads_as_absent_rather_than_malformed() {
        let error = rejection(
            "enabled = true\ndsn = \"   \"",
            "a blank DSN reports nowhere either",
        );

        assert!(error.to_string().contains("is empty"), "{error}");
    }

    /// A DSN that does not parse must say so without quoting the value: the message reaches the
    /// log stream, and the value is a credential.
    #[test]
    fn a_malformed_dsn_is_refused_without_quoting_it() {
        let error = rejection(
            "enabled = true\ndsn = \"not-a-dsn\"",
            "the DSN does not parse",
        );

        assert!(
            error.to_string().contains("not a valid Sentry DSN"),
            "{error}"
        );
        assert!(
            !error.to_string().contains("not-a-dsn"),
            "the DSN must not be echoed: {error}"
        );
    }

    /// With no client bound there is no trace to continue, and the caller attaches the result
    /// unconditionally — so this has to be empty rather than a header with an empty value, which
    /// the target would parse as a malformed trace and log about on every request.
    #[test]
    fn trace_headers_are_empty_without_a_client() {
        assert!(super::trace_headers().is_empty());
    }
}
