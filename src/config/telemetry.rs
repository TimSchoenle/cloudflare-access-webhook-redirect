//! Logging and error-reporting settings.
//!
//! Both are installed process-globally, once, before the reloadable runtime exists — so this is
//! the one block a configuration reload cannot apply. Changing it still needs a restart.

use secrecy::SecretString;
use serde::{Deserialize, Deserializer};
use std::str::FromStr;
use tracing::Level;

const DEFAULT_LOG_LEVEL: Level = Level::INFO;

/// Observability settings.
#[derive(Debug, Clone, Deserialize, Getters)]
#[getset(get = "pub")]
pub struct TelemetryConfig {
    /// Minimum level emitted by the subscriber (`trace`, `debug`, `info`, `warn`, `error`).
    ///
    /// Parsed here rather than in `main` so an unusable value fails the boot with a
    /// configuration error naming the key, instead of after the subscriber is installed.
    #[serde(
        default = "TelemetryConfig::default_log_level",
        deserialize_with = "deserialize_level_from_string"
    )]
    log_level: Level,
    /// Sentry DSN. Error reporting is disabled when absent.
    ///
    /// A [`SecretString`]: a DSN carries the project key that authorises event submission.
    #[serde(default)]
    sentry_dsn: Option<SecretString>,
}

impl TelemetryConfig {
    fn default_log_level() -> Level {
        DEFAULT_LOG_LEVEL
    }
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            log_level: Self::default_log_level(),
            sentry_dsn: None,
        }
    }
}

fn deserialize_level_from_string<'de, D>(deserializer: D) -> Result<Level, D::Error>
where
    D: Deserializer<'de>,
{
    let string: String = Deserialize::deserialize(deserializer)?;
    Level::from_str(&string).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::TelemetryConfig;
    use figment::providers::Format;
    use tracing::Level;

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

        assert_eq!(config.log_level(), &Level::INFO);
        assert!(config.sentry_dsn().is_none());
    }

    #[test]
    fn a_log_level_is_parsed_case_insensitively() {
        assert_eq!(
            deserialize("log_level = \"DEBUG\"").unwrap().log_level(),
            &Level::DEBUG
        );
        assert_eq!(
            deserialize("log_level = \"warn\"").unwrap().log_level(),
            &Level::WARN
        );
    }

    #[test]
    fn an_unknown_log_level_is_refused() {
        let error = deserialize("log_level = \"chatty\"").expect_err("not a level");

        assert!(
            error.to_string().contains("log_level"),
            "the error must name the key: {error}"
        );
    }
}
