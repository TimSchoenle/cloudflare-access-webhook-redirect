use actix_web::http::Method;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error")]
    IoError(#[from] std::io::Error),
    #[error("Serde error")]
    Serde(#[from] serde_json::Error),
    #[error("Regex error")]
    Regex(#[from] regex::Error),
    #[error("Invalid route")]
    InvalidRoute(String),
    /// A configuration could not be assembled: a missing or unparseable value, an unreadable
    /// file-backed source, or one key supplied by more than one layer.
    #[error("Config error: {0}")]
    Config(#[from] terrace_config::Error),
    /// The filesystem watcher backing configuration reloads could not be installed.
    #[error(transparent)]
    Watch(#[from] terrace_config::reload::WatchError),
    /// `telemetry.sentry` is switched on but unusable — no DSN, a DSN that does not parse, or a
    /// sample rate outside `0.0..=1.0`. Carries the message rather than a wrapped SDK error: the
    /// three are configuration mistakes, and the text names the key that has to change.
    #[error("Sentry error: {0}")]
    Sentry(String),
    #[error("{0}")]
    Custom(String),
}

impl Error {
    pub fn custom<S: ToString>(msg: S) -> Self {
        Self::Custom(msg.to_string())
    }

    pub fn invalid_route(route: &Method) -> Self {
        Self::InvalidRoute(route.to_string())
    }
}
