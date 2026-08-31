//! The error everything outside the request path returns.
//!
//! A request that fails midway does not use this. The handler in `routes::redirect` converts to
//! [`actix_web::Error`] at the point it gives up, and the actix-to-reqwest translation has its own
//! [`ConverterError`](crate::converter::ConverterError).

use actix_web::http::Method;
use thiserror::Error;

/// Anything that stops the proxy starting or reloading.
#[derive(Error, Debug)]
pub enum Error {
    /// The listener could not bind `server.host` and `server.port`, or stopped with an error.
    #[error("IO error")]
    IoError(#[from] std::io::Error),
    /// JSON that could not be parsed or rendered. No path in this crate produces one.
    #[error("Serde error")]
    Serde(#[from] serde_json::Error),
    /// A `webhook.paths` key is not a valid regular expression.
    #[error("Regex error")]
    Regex(#[from] regex::Error),
    /// A method reached the forwarder that it builds no request for. The route table registers
    /// only the five that it does, so this is a guard rather than a reachable state.
    #[error("Invalid route")]
    InvalidRoute(String),
    /// A configuration could not be assembled: a missing or unparseable value, an unreadable
    /// file-backed source, or one key supplied by more than one layer.
    #[error("Config error: {0}")]
    Config(#[from] terrace_config::Error),
    /// The filesystem watcher backing configuration reloads could not be installed.
    #[error(transparent)]
    Watch(#[from] terrace_config::reload::WatchError),
    /// `telemetry.sentry` is switched on but unusable: no DSN, a DSN that does not parse, or a
    /// sample rate outside `0.0..=1.0`. Carries the message rather than a wrapped SDK error,
    /// because the three are configuration mistakes and the text names the key that has to
    /// change.
    #[error("Sentry error: {0}")]
    Sentry(String),
    /// A failure with no better variant: a credential that is not a valid header value, a target
    /// URL that will not join, a method name the configuration invented.
    #[error("{0}")]
    Custom(String),
}

impl Error {
    /// Builds a [`Custom`](Error::Custom) from anything printable.
    #[must_use]
    pub fn custom<S: ToString + ?Sized>(msg: &S) -> Self {
        Self::Custom(msg.to_string())
    }

    /// Names the method the forwarder could not build a request for.
    #[must_use]
    pub fn invalid_route(route: &Method) -> Self {
        Self::InvalidRoute(route.to_string())
    }
}
