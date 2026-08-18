//! The Cloudflare Access service token this proxy authenticates with.

use secrecy::SecretString;
use serde::Deserialize;

/// The Cloudflare Access service token injected into every forwarded request.
///
/// Both halves are [`SecretString`]: they are credentials, and this struct is nested inside a
/// [`Config`](crate::config::Config) that is logged with `?` on a failed boot.
#[derive(Debug, Clone, Deserialize, Getters)]
#[cfg_attr(feature = "config-schema", derive(terrace_config::schema::Describe))]
#[getset(get = "pub")]
pub struct CloudFlareConfig {
    /// `CF-Access-Client-Id` header value.
    #[cfg_attr(feature = "config-schema", config(secret))]
    client_id: SecretString,
    /// `CF-Access-Client-Secret` header value.
    #[cfg_attr(feature = "config-schema", config(secret))]
    client_secret: SecretString,
}
