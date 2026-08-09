//! Listener settings.

use serde::Deserialize;

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8080;

/// Where the proxy binds its listener.
#[derive(Debug, Clone, Deserialize, Getters)]
#[getset(get = "pub")]
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
