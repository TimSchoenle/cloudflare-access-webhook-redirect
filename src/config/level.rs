//! The severity types the configuration surface is written in.
//!
//! One module owns both, because they are the same six words twice: a variant list that lived in
//! two files would document `warn` twice and drift once. [`LogLevel`] is the subscriber's own
//! threshold, [`SentryLevel`] is a Sentry sink's, and the only difference between them is that a
//! sink can be turned off while a subscriber cannot.
//!
//! Neither is a `tracing` type. Both are deserialised by `serde`'s derive from the lower-case
//! variant name, which is what makes the variant list *be* the accepted set: the schema publishes
//! it, the settings table renders it, and no second parser exists to accept a spelling the list
//! does not contain. The conversion to what `tracing` wants happens once, in
//! [`crate::telemetry::init`], through the [`From`] impls below.

use serde::Deserialize;

/// Minimum severity the subscriber emits.
///
/// [`Ord`] follows declaration order, and the variants are declared in the same direction
/// [`tracing::Level`] sorts in: more verbose is *greater*, so `Error` is the least of these and
/// `Trace` the greatest, and "at least as severe as this threshold" reads `<=` on both types.
/// [`SentryLevel`] below is declared in that same direction.
///
/// Deserialised from the lower-case variant name and nothing else. `INFO` and `"3"` — both
/// accepted while this key was a [`tracing::Level`] parsed through `FromStr` — are refused, and
/// refused at boot with an error naming the key.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd, Deserialize)]
#[cfg_attr(
    feature = "config-schema",
    derive(serde::Serialize, terrace_config::schema::Describe)
)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// `error` only.
    Error,
    /// `error` and `warn`.
    Warn,
    /// Down to `info`.
    #[default]
    Info,
    /// Down to `debug`.
    Debug,
    /// Everything.
    Trace,
}

/// How much of the `tracing` stream one Sentry sink takes.
///
/// Ordered by severity, so a threshold names the *least* severe record it accepts: `warn` means
/// `error` and `warn`. [`Off`](Self::Off) is below all of them and takes nothing.
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

impl From<LogLevel> for tracing::Level {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Error => Self::ERROR,
            LogLevel::Warn => Self::WARN,
            LogLevel::Info => Self::INFO,
            LogLevel::Debug => Self::DEBUG,
            LogLevel::Trace => Self::TRACE,
        }
    }
}

impl From<LogLevel> for tracing_subscriber::filter::LevelFilter {
    fn from(level: LogLevel) -> Self {
        Self::from_level(level.into())
    }
}

#[cfg(test)]
mod tests {
    use super::LogLevel;
    use tracing::Level;
    use tracing_subscriber::filter::LevelFilter;

    /// The direction the type documents, pinned. Inverting it turns a threshold into its
    /// opposite, which is a quiet change in behaviour rather than a compile error.
    #[test]
    fn severity_orders_the_way_tracing_does() {
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Trace);

        // The same direction `tracing` itself sorts in, which is what makes `<=` mean "at least
        // as severe" on a level and on a threshold alike.
        assert!(Level::ERROR < Level::TRACE);
    }

    #[test]
    fn every_level_converts_to_the_tracing_one_it_names() {
        for (level, expected) in [
            (LogLevel::Error, Level::ERROR),
            (LogLevel::Warn, Level::WARN),
            (LogLevel::Info, Level::INFO),
            (LogLevel::Debug, Level::DEBUG),
            (LogLevel::Trace, Level::TRACE),
        ] {
            assert_eq!(Level::from(level), expected, "{level:?}");
            assert_eq!(
                LevelFilter::from(level),
                LevelFilter::from_level(expected),
                "{level:?}"
            );
        }
    }

    #[test]
    fn the_default_is_info() {
        assert_eq!(LogLevel::default(), LogLevel::Info);
    }
}
