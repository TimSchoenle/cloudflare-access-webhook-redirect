//! The dialect of [`terrace_config`] this service boots through.
//!
//! The layering itself — the TOML file, the `WEBHOOK_REDIRECT_*` environment layer, the
//! secrets-directory provider, the `_FILE` indirection and the shadow-key rejection — belongs
//! to `terrace-config`. What stays here is the one thing that is ours: which environment names
//! this deployment spells.

use serde::de::DeserializeOwned;
use terrace_config::Terrace;

pub use terrace_config::explain::Explanation;
pub use terrace_config::{Error as ConfigError, Loaded, Sources};

/// The prefix every configuration variable carries.
const PREFIX: &str = "WEBHOOK_REDIRECT_";

/// The loader this service boots through.
///
/// Layers, lowest precedence first: struct defaults, TOML at `$WEBHOOK_REDIRECT_CONFIG` (a
/// file, or every `*.toml` in it if it names a directory, defaulting to `./config.toml`),
/// `WEBHOOK_REDIRECT_`-prefixed `__`-nested environment variables,
/// `$WEBHOOK_REDIRECT_SECRETS_DIR`, and `WEBHOOK_REDIRECT_<KEY>_FILE` indirection. The last
/// three are mutually exclusive per key: a key supplied by two of them is refused at boot
/// rather than resolved by precedence, because a stale environment variable shadowing a
/// rotated mounted secret keeps the service running on the old credential.
///
/// Both names are spelled out even though `Terrace::new(PREFIX)` derives exactly these: they
/// are the documented operator surface, and a variable that exists only as a derivation inside
/// a dependency is one the README cannot be held to.
pub fn terrace() -> Terrace {
    Terrace::new(PREFIX)
        .config_var("WEBHOOK_REDIRECT_CONFIG")
        .secrets_dir_var("WEBHOOK_REDIRECT_SECRETS_DIR")
}

/// Load a typed config.
///
/// # Errors
/// Returns [`ConfigError`] if a required value is missing, a value fails to parse, a
/// file-backed source cannot be read, or one key is supplied by more than one of the last
/// three layers.
pub fn load<T: DeserializeOwned>() -> Result<T, ConfigError> {
    terrace().load()
}

/// Load a typed config together with everything a reload needs to load it again.
///
/// # Errors
/// As [`load`].
pub fn load_watched<T: DeserializeOwned>() -> Result<Loaded<T>, ConfigError> {
    terrace().load_watched()
}

/// Report which layer supplied each key, re-reading them at the moment it is called.
///
/// The question a boot log cannot otherwise answer. [`Config`](crate::config::Config) is full of
/// [`SecretString`](secrecy::SecretString), so no layer of it is ever logged as a value — which
/// leaves "the rotated secret is not being picked up" with nothing to go on. An
/// [`Explanation`] holds no configuration value at all, only the names of the files and
/// variables each key arrived from, so it is safe in a log that the values can never enter.
///
/// It does not fail for the reason it is being run: a configuration [`load`] refuses because one
/// key was supplied twice still explains, and reports that key with both of its sources.
///
/// # Errors
/// Returns [`ConfigError`] if a file-backed source cannot be read.
pub fn explain() -> Result<Explanation, ConfigError> {
    terrace().explain()
}
