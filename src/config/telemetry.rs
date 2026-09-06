//! Logging and error-reporting settings.
//!
//! Both are installed process-globally, once, before the reloadable runtime exists. That makes
//! this the one block a configuration reload cannot apply. Changing it still needs a restart.

use crate::config::SentryConfig;
use serde::{Deserialize, Deserializer};
use std::str::FromStr;
use tracing::Level;

const DEFAULT_LOG_LEVEL: Level = Level::INFO;

/// Observability settings.
///
/// `Serialize` is the schema generator's, not the service's. See [`ServerConfig`]. [`Level`]
/// does not serialise as it deserialises, because it has no `Serialize` at all.
///
/// `Describe` is written out below rather than derived, for [`Self::log_level`]'s sake. See the
/// implementation for why no `#[config(...)]` attribute can describe that key truthfully.
///
/// [`ServerConfig`]: crate::config::ServerConfig
#[derive(Debug, Clone, Deserialize, Getters)]
#[cfg_attr(feature = "config-schema", derive(serde::Serialize))]
#[getset(get = "pub")]
#[serde(deny_unknown_fields)]
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
    /// Sentry error reporting and performance tracing. Off unless `sentry.enabled` is set.
    #[serde(default)]
    sentry: SentryConfig,
}

/// Reported by hand, because [`Level`] defeats every attribute the derive offers.
///
/// `terrace-config` v0.11.0 refuses a field whose type is a bare name it does not recognise,
/// and rightly: such a key published no shape at all. The six resolving attributes are the
/// wrong answer here, every one of them.
///
/// [`Level`] has no `Deserialize` of its own, so `deserialize_level_from_string` below parses it
/// through [`FromStr`], which folds ASCII case **and** accepts `"1"` through `"5"`
/// (`tracing_core::metadata`, `impl FromStr for Level`). The accepted set is therefore not a
/// fixed list of spellings, so `#[config(values("trace", …))]` would publish a schema refusing
/// `INFO` and `"3"` — files this service boots on. It is also a newtype over a private enum, so
/// `#[serde(remote)]` cannot mirror it and `values_from` has nothing to point at. That leaves
/// `#[config(skip)]`, which would delete the key from the settings table and the example file
/// for a key operators do set.
///
/// So the leaf below reports exactly what is true and no more: the type as written, its prose,
/// and no constraint. It is what the derive emitted for this field before v0.11.0 made the
/// silence deliberate rather than accidental.
///
/// The doc text is spelled twice — once as the field's `///` comment, once here — which is the
/// cost of the hand-written route. Nothing checks that they agree.
///
/// [`FromStr`]: std::str::FromStr
#[cfg(feature = "config-schema")]
impl terrace_config::schema::Describe for TelemetryConfig {
    fn describe(sink: &mut terrace_config::schema::Sink) {
        // First, so the level it closes is this type's own rather than one a key below pushed.
        sink.deny_unknown_fields();
        sink.leaf(terrace_config::schema::Leaf {
            name: "log_level",
            docs: concat!(
                "Minimum level emitted by the subscriber (`trace`, `debug`, `info`, `warn`, ",
                "`error`).\n\nParsed here rather than in `main` so an unusable value fails the ",
                "boot with a\nconfiguration error naming the key, instead of after the ",
                "subscriber is installed."
            ),
            ty: Some("Level"),
            values: None,
            bounds: None,
            aliases: &[],
            note: None,
            required: false,
            secret: false,
        });
        sink.nested(
            "sentry",
            <SentryConfig as terrace_config::schema::Describe>::describe,
        );
    }
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
            sentry: SentryConfig::default(),
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
        assert!(!config.sentry().enabled());
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
