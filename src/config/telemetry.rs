//! Logging and error-reporting settings.
//!
//! Both are installed process-globally, once, before the reloadable runtime exists. That makes
//! this the one block a configuration reload cannot apply. Changing it still needs a restart.

use secrecy::SecretString;
use serde::{Deserialize, Deserializer};
use std::str::FromStr;
use tracing::Level;

const DEFAULT_LOG_LEVEL: Level = Level::INFO;

/// Observability settings.
///
/// `Serialize` is the schema generator's, not the service's. See [`ServerConfig`]. Neither
/// field serialises as it deserialises: [`Level`] has no `Serialize` at all, and
/// [`SecretString`] refuses to have one, which is the point of the type.
///
/// [`ServerConfig`]: crate::config::ServerConfig
#[derive(Debug, Clone, Deserialize, Getters)]
#[cfg_attr(
    feature = "config-schema",
    derive(serde::Serialize, terrace_config::schema::Describe)
)]
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
    #[cfg_attr(
        feature = "config-schema",
        serde(serialize_with = "serialize_level_as_written")
    )]
    log_level: Level,
    /// Sentry DSN. Error reporting is disabled when absent.
    ///
    /// A [`SecretString`]: a DSN carries the project key that authorises event submission.
    // Below the doc comment on purpose: `config.example.toml` is generated from the whole of
    // that comment, and how this crate serialises a field is not an operator's business.
    // `skip_serializing` costs the generated table nothing either way: `#[config(secret)]`
    // renders `<redacted>` in place of whatever the value is, so the only thing left out is the
    // one thing that must not reach a documentation file.
    #[serde(default, skip_serializing)]
    #[cfg_attr(feature = "config-schema", config(secret))]
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

/// Renders a [`Level`] the way the configuration file spells it.
///
/// [`Level`]'s own `Display` is uppercase, and a `Default` column reading `INFO` documents a
/// value an operator would then copy — correctly, since parsing folds case, but nowhere else in
/// the reference is a value written in a spelling the examples do not use.
#[cfg(feature = "config-schema")]
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's serialize_with hands the field by reference; the signature is not ours"
)]
fn serialize_level_as_written<S>(level: &Level, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&level.to_string().to_lowercase())
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
            error.contains("log_level"),
            "the error must name the key: {error}"
        );
    }
}
