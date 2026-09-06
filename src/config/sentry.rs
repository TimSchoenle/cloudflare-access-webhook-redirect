//! Sentry error reporting and performance tracing.
//!
//! Installed once per process, before the reloadable runtime exists, which is why this block —
//! like the rest of `[telemetry]` — is the one a configuration reload cannot apply.

use secrecy::SecretString;
use serde::Deserialize;

const DEFAULT_ENVIRONMENT: &str = "production";
const DEFAULT_SAMPLE_RATE: f32 = 1.0;
const DEFAULT_TRACES_SAMPLE_RATE: f32 = 0.0;
const DEFAULT_MAX_BREADCRUMBS: usize = 100;
const DEFAULT_SHUTDOWN_TIMEOUT_SECS: u64 = 2;

/// How much of the `tracing` stream one Sentry sink takes.
///
/// Ordered by severity, so a threshold names the *least* severe record it accepts: `warn` means
/// `error` and `warn`.
///
/// Deserialised by variant name in lower case rather than through [`FromStr`]: unlike
/// [`AllowedMethod`], nothing else in this crate parses these, and the generated settings table
/// lists the accepted spellings.
///
/// [`FromStr`]: std::str::FromStr
/// [`AllowedMethod`]: crate::config::AllowedMethod
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Deserialize)]
#[cfg_attr(
    feature = "config-schema",
    derive(serde::Serialize, terrace_config::schema::Describe)
)]
#[serde(rename_all = "lowercase")]
pub enum SentryLevel {
    /// Take nothing.
    Off,
    /// `error` only.
    #[default]
    Error,
    /// `error` and `warn`.
    Warn,
    /// Down to `info`.
    Info,
    /// Down to `debug`.
    Debug,
    /// Everything.
    Trace,
}

/// Sentry error reporting and performance tracing.
///
/// Off by default and off in the generated example: a DSN is an egress destination for whatever
/// a log line happens to carry, so turning it on is an operator's decision made once per
/// deployment. With [`Self::enabled`] set the service refuses to boot without a usable
/// [`Self::dsn`] rather than starting a reporter that reports nowhere.
///
/// `Serialize` is the schema generator's, not the service's — see [`ServerConfig`]. [`Self::dsn`]
/// is the one field that does not serialise as it deserialises: [`SecretString`] refuses to have
/// a `Serialize`, which is the point of the type.
///
/// [`ServerConfig`]: crate::config::ServerConfig
#[derive(Debug, Clone, Deserialize, Getters, CopyGetters)]
#[cfg_attr(
    feature = "config-schema",
    derive(serde::Serialize, terrace_config::schema::Describe)
)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "a settings block is a flat list of switches an operator writes by name, so the \
              lint's remedy — folding pairs of them into two-variant enums — would change the \
              TOML surface to quiet a lint about the shape of the Rust struct"
)]
#[serde(deny_unknown_fields)]
pub struct SentryConfig {
    /// Initialise the Sentry client. `false` installs no client, no panic hook, no `tracing`
    /// layer and no HTTP middleware, so every other key here is inert and nothing is sent
    /// anywhere.
    #[serde(default)]
    #[getset(get_copy = "pub")]
    enabled: bool,
    /// Ingest URL, `https://<key>@<host>/<project>`.
    ///
    /// A [`SecretString`]: the embedded key is a bearer credential for the project's ingest
    /// endpoint. Absent while `enabled` is set fails the boot rather than reporting nowhere.
    // Below the doc comment on purpose: `config.example.toml` is generated from the whole of
    // that comment, and how this crate serialises a field is not an operator's business.
    // `skip_serializing` costs the generated table nothing — `#[config(secret)]` renders
    // `<redacted>` in place of whatever the value is, so the only thing left out is the one
    // thing that must not reach a documentation file.
    #[serde(default, skip_serializing)]
    #[cfg_attr(feature = "config-schema", config(secret))]
    #[getset(get = "pub")]
    dsn: Option<SecretString>,
    /// Environment tag on every event, such as `production` or `staging`.
    ///
    /// Has a default rather than being optional so the field is never empty when the SDK's own
    /// defaults run: an unset `environment` is the one field they would fill from
    /// `SENTRY_ENVIRONMENT`, which is a second configuration channel that bypasses the layered
    /// loader and its shadow-key rejection. `release` and `dsn` are always set for the same
    /// reason.
    #[serde(default = "SentryConfig::default_environment")]
    #[getset(get = "pub")]
    environment: String,
    /// Release tag on every event. Defaults to the crate name and version the binary was built
    /// from, which is what makes a regression attributable to a deploy.
    #[serde(default)]
    #[getset(get = "pub")]
    release: Option<String>,
    /// Host tag on every event. Left unset, Sentry reports none: the hostname of a replica is
    /// infrastructure detail that `send_default_pii` would otherwise gate.
    #[serde(default)]
    #[getset(get = "pub")]
    server_name: Option<String>,
    /// Fraction of captured events actually sent, `0.0`-`1.0`. A blunt volume cap — it drops
    /// whole issues, not repetitions of one — so leave it at `1.0` unless quota forces it.
    #[serde(default = "SentryConfig::default_sample_rate")]
    #[cfg_attr(feature = "config-schema", config(range(min = 0.0, max = 1.0)))]
    #[getset(get_copy = "pub")]
    sample_rate: f32,
    /// Fraction of traces this proxy **starts** that are recorded, `0.0`-`1.0`.
    ///
    /// `0.0` (the default) means it starts none of its own. It does **not** mean the proxy is
    /// absent from a trace: a request arriving with a `sentry-trace` header already sampled is
    /// continued regardless, which is what keeps one webhook delivery readable across the
    /// caller, this hop and the Cloudflare Access protected service behind it.
    #[serde(default = "SentryConfig::default_traces_sample_rate")]
    #[cfg_attr(feature = "config-schema", config(range(min = 0.0, max = 1.0)))]
    #[getset(get_copy = "pub")]
    traces_sample_rate: f32,
    /// Least severe `tracing` level reported as a Sentry **issue**: `off`, `error`, `warn`,
    /// `info`, `debug` or `trace`.
    ///
    /// Bounded from above by `telemetry.log_level`: the Sentry layer sits under the same filter
    /// the console log does, so a record that level drops is never reported either.
    #[serde(default)]
    #[cfg_attr(feature = "config-schema", config(values))]
    #[getset(get_copy = "pub")]
    capture_level: SentryLevel,
    /// Least severe `tracing` level kept as a **breadcrumb** — the trail attached to the next
    /// issue. Same spellings as `capture_level`; records at or above it become issues instead.
    #[serde(default = "SentryConfig::default_breadcrumb_level")]
    #[cfg_attr(feature = "config-schema", config(values))]
    #[getset(get_copy = "pub")]
    breadcrumb_level: SentryLevel,
    /// How many breadcrumbs one event carries.
    #[serde(default = "SentryConfig::default_max_breadcrumbs")]
    #[getset(get_copy = "pub")]
    max_breadcrumbs: usize,
    /// Attach a stack trace to events that carry none of their own.
    #[serde(default = "SentryConfig::enabled_by_default")]
    #[getset(get_copy = "pub")]
    attach_stacktraces: bool,
    /// Send personally identifying data with every event: the client IP, the full request header
    /// set, and request bodies of a known content type.
    ///
    /// **Off, and worth leaving off.** Every header this proxy receives is forwarded to a
    /// Cloudflare Access protected service, so the header set of a webhook delivery routinely
    /// carries the caller's own signing secret — which is exactly what a crash report does not
    /// need in order to be actionable.
    #[serde(default)]
    #[getset(get_copy = "pub")]
    send_default_pii: bool,
    /// Record one Sentry transaction per request, named by the method and the matched path.
    ///
    /// Whether a started transaction is *kept* is `traces_sample_rate`'s decision. This is the
    /// switch for a deployment that should stay out of traces entirely while still reporting
    /// errors.
    #[serde(default = "SentryConfig::enabled_by_default")]
    #[getset(get_copy = "pub")]
    http_transactions: bool,
    /// Copy `tracing` span fields onto the Sentry span as attributes. Off: the request span this
    /// proxy opens carries the full request path, and a transaction is stored under a longer
    /// retention than a log line.
    #[serde(default)]
    #[getset(get_copy = "pub")]
    span_attributes: bool,
    /// How long process exit waits for queued events to drain.
    #[serde(default = "SentryConfig::default_shutdown_timeout_secs")]
    #[getset(get_copy = "pub")]
    shutdown_timeout_secs: u64,
    /// Print the SDK's own diagnostics to stderr. For proving a DSN works, not for running.
    #[serde(default)]
    #[getset(get_copy = "pub")]
    debug: bool,
}

impl SentryConfig {
    fn default_environment() -> String {
        DEFAULT_ENVIRONMENT.to_string()
    }

    fn default_sample_rate() -> f32 {
        DEFAULT_SAMPLE_RATE
    }

    fn default_traces_sample_rate() -> f32 {
        DEFAULT_TRACES_SAMPLE_RATE
    }

    fn default_breadcrumb_level() -> SentryLevel {
        SentryLevel::Info
    }

    fn default_max_breadcrumbs() -> usize {
        DEFAULT_MAX_BREADCRUMBS
    }

    fn default_shutdown_timeout_secs() -> u64 {
        DEFAULT_SHUTDOWN_TIMEOUT_SECS
    }

    /// Shared by the two keys that default to on, so neither can drift from what [`Default`]
    /// reports to the settings table.
    fn enabled_by_default() -> bool {
        true
    }
}

impl Default for SentryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dsn: None,
            environment: Self::default_environment(),
            release: None,
            server_name: None,
            sample_rate: Self::default_sample_rate(),
            traces_sample_rate: Self::default_traces_sample_rate(),
            capture_level: SentryLevel::default(),
            breadcrumb_level: Self::default_breadcrumb_level(),
            max_breadcrumbs: Self::default_max_breadcrumbs(),
            attach_stacktraces: Self::enabled_by_default(),
            send_default_pii: false,
            http_transactions: Self::enabled_by_default(),
            span_attributes: false,
            shutdown_timeout_secs: Self::default_shutdown_timeout_secs(),
            debug: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SentryConfig, SentryLevel};
    use figment::providers::Format;

    /// The error is stringified because `figment::Error` is large enough to trip
    /// `clippy::result_large_err`, and every assertion below reads its message anyway.
    fn deserialize(toml: &str) -> Result<SentryConfig, String> {
        figment::Figment::from(figment::providers::Toml::string(toml))
            .extract()
            .map_err(|e| e.to_string())
    }

    /// A deployment that says nothing about Sentry gets no client and no egress. Every key is
    /// `#[serde(default)]`, so an absent block must still materialise rather than failing the
    /// boot of every deployment that has not been told about this section.
    #[test]
    fn an_unmentioned_block_is_off() {
        let config = deserialize("").unwrap();

        assert!(!config.enabled());
        assert!(config.dsn().is_none());
        assert_eq!(config.capture_level(), SentryLevel::Error);
        assert_eq!(config.breadcrumb_level(), SentryLevel::Info);
        assert!(!config.send_default_pii());
        assert!(config.http_transactions());
    }

    /// `0.0` rather than `1.0`: a proxy that starts a trace for every request it is handed
    /// would bill a Sentry quota for traffic nobody asked to trace. An inherited trace is
    /// continued regardless, which is the case that actually matters here.
    #[test]
    fn traces_are_not_started_by_default() {
        let config = deserialize("").unwrap();

        // Compared with a tolerance rather than `assert_eq!`, because `clippy::float_cmp` is on
        // and it is right in general. Both sides here are the literal defaults, so any tolerance
        // at all is enough to tell them from the value the assertion is guarding against.
        assert!(
            config.traces_sample_rate().abs() < f32::EPSILON,
            "no trace is started by default: {}",
            config.traces_sample_rate()
        );
        assert!(
            (config.sample_rate() - 1.0).abs() < f32::EPSILON,
            "every captured event is sent by default: {}",
            config.sample_rate()
        );
    }

    #[test]
    fn a_level_is_parsed_from_its_lower_case_name() {
        let config = deserialize("capture_level = \"warn\"\nbreadcrumb_level = \"off\"").unwrap();

        assert_eq!(config.capture_level(), SentryLevel::Warn);
        assert_eq!(config.breadcrumb_level(), SentryLevel::Off);
    }

    #[test]
    fn an_unknown_level_is_refused() {
        let error = deserialize("capture_level = \"chatty\"").expect_err("not a level");

        assert!(
            error.contains("capture_level"),
            "the error must name the key: {error}"
        );
    }
}
