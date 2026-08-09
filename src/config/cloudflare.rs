//! The Cloudflare Access service token this proxy authenticates with.

use secrecy::SecretString;
use serde::Deserialize;

/// The Cloudflare Access service token injected into every forwarded request.
///
/// Both halves are [`SecretString`]: they are credentials, and this struct is nested inside a
/// [`Config`](crate::config::Config) that is logged with `?` on a failed boot.
#[derive(Debug, Clone, Deserialize, Getters)]
#[getset(get = "pub")]
pub struct CloudFlareConfig {
    /// `CF-Access-Client-Id` header value.
    client_id: SecretString,
    /// `CF-Access-Client-Secret` header value.
    client_secret: SecretString,
}
