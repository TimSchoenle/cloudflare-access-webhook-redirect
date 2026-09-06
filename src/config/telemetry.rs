//! Logging and error-reporting settings.
//!
//! Both are installed process-globally, once, before the reloadable runtime exists. That makes
//! this the one block a configuration reload cannot apply. Changing it still needs a restart.

use crate::config::{LogLevel, SentryConfig};
use serde::Deserialize;

/// Observability settings.
///
/// `Serialize` is the schema generator's, not the service's. See [`ServerConfig`].
///
/// [`ServerConfig`]: crate::config::ServerConfig
#[derive(Debug, Clone, Default, Deserialize, Getters, CopyGetters)]
#[cfg_attr(
    feature = "config-schema",
    derive(serde::Serialize, terrace_config::schema::Describe)
)]
#[serde(deny_unknown_fields)]
pub struct TelemetryConfig {
    /// Minimum level emitted by the subscriber.
    ///
    /// Read as a [`LogLevel`] rather than as a `tracing` type so an unusable value fails the
    /// boot with a configuration error naming the key, instead of after the subscriber is
    /// installed — and so the spellings this key takes are the ones the settings table lists.
    #[serde(default)]
    #[cfg_attr(feature = "config-schema", config(values))]
    #[getset(get_copy = "pub")]
    log_level: LogLevel,
    /// Sentry error reporting and performance tracing. Off unless `sentry.enabled` is set.
    #[serde(default)]
    #[cfg_attr(feature = "config-schema", config(nested))]
    #[getset(get = "pub")]
    sentry: SentryConfig,
}

#[cfg(test)]
mod tests {
    use super::{LogLevel, TelemetryConfig};
    use figment::providers::Format;

    /// The error is stringified because `figment::Error` is large enough to trip
    /// `clippy::result_large_err`, and every assertion below reads its message anyway.
    fn deserialize(toml: &str) -> Result<TelemetryConfig, String> {
        figment::Figment::from(figment::providers::Toml::string(toml))
            .extract()
            .map_err(|e| e.to_string())
    }

    #[test]
    fn empty_block_falls_back_to_the_defaults() {
        let config = deserialize("").unwrap();

        assert_eq!(config.log_level(), LogLevel::Info);
        assert!(!config.sentry().enabled());
    }

    #[test]
    fn a_log_level_is_parsed_from_its_lower_case_name() {
        for (spelling, expected) in [
            ("trace", LogLevel::Trace),
            ("debug", LogLevel::Debug),
            ("info", LogLevel::Info),
            ("warn", LogLevel::Warn),
            ("error", LogLevel::Error),
        ] {
            let config = deserialize(&format!("log_level = \"{spelling}\""))
                .unwrap_or_else(|error| panic!("`{spelling}` is a level: {error}"));

            assert_eq!(config.log_level(), expected, "{spelling}");
        }
    }

    /// The narrowing, pinned. [`Level`]'s `FromStr` folded ASCII case and additionally took `"1"`
    /// through `"5"`; the derive matches one spelling per variant and nothing else. Asserted
    /// rather than left to the derive, because widening it back is one `#[serde(alias)]` away and
    /// would otherwise be invisible.
    ///
    /// [`Level`]: tracing::Level
    #[test]
    fn an_upper_case_or_numeric_log_level_is_refused() {
        for spelling in ["INFO", "Info", "3", "1"] {
            let error = deserialize(&format!("log_level = \"{spelling}\""))
                .expect_err("only the lower-case names are levels");

            assert!(
                error.contains("log_level"),
                "the error must name the key: {error}"
            );
        }
    }

    #[test]
    fn an_unknown_log_level_is_refused() {
        let error = deserialize("log_level = \"chatty\"").expect_err("not a level");

        assert!(
            error.contains("log_level"),
            "the error must name the key: {error}"
        );
    }
}
