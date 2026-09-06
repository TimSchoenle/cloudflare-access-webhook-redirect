//! Listener settings.

use serde::Deserialize;

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8080;

/// Where the proxy binds its listener.
///
/// `Serialize` is the schema generator's, not the service's: `with_defaults_from` reads the
/// `Default` column out of a serialised value, and this is one of the two blocks that has
/// defaults to report.
#[derive(Debug, Clone, Deserialize, Getters)]
#[cfg_attr(
    feature = "config-schema",
    derive(serde::Serialize, terrace_config::schema::Describe)
)]
#[getset(get = "pub")]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Bind address. Containers usually want `0.0.0.0`.
    #[serde(default = "ServerConfig::default_host")]
    host: String,
    /// Bind port.
    #[serde(default = "ServerConfig::default_port")]
    port: u16,
}

impl ServerConfig {
    fn default_host() -> String {
        DEFAULT_HOST.to_string()
    }

    fn default_port() -> u16 {
        DEFAULT_PORT
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: Self::default_host(),
            port: Self::default_port(),
        }
    }
}
