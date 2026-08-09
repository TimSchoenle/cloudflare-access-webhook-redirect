//! The typed configuration surface, and the dialect of the layered loader it is read through.
//!
//! The layering is [`terrace_config`]'s. Lowest precedence first: struct defaults, TOML at
//! `$WEBHOOK_REDIRECT_CONFIG` (`./config.toml` when unset), `WEBHOOK_REDIRECT_`-prefixed
//! `__`-nested environment variables, `$WEBHOOK_REDIRECT_SECRETS_DIR`, and
//! `WEBHOOK_REDIRECT_<KEY>_FILE` indirection. The last three are mutually exclusive per key: a
//! key supplied by two of them is refused at boot rather than resolved by precedence.
//!
//! Call [`load`], or [`load_watched`] when the process should be able to pick the
//! configuration up again after a mounted file changes.

mod cloudflare;
mod loader;
mod server;
mod telemetry;
mod webhook;

pub use cloudflare::CloudFlareConfig;
pub use loader::{ConfigError, Loaded, Sources, load, load_watched, terrace};
pub use server::ServerConfig;
pub use telemetry::TelemetryConfig;
pub use webhook::{AllowedMethod, WebhookConfig};

use serde::Deserialize;

/// Everything the proxy reads at boot.
#[derive(Debug, Clone, Deserialize, Getters)]
#[getset(get = "pub")]
pub struct Config {
    /// Where the listener binds. Defaults throughout, so the block may be omitted.
    #[serde(default)]
    server: ServerConfig,
    /// Logging and error reporting. Installed once per process, so a reload does not apply it.
    #[serde(default)]
    telemetry: TelemetryConfig,
    /// The Cloudflare Access service token. Required.
    cloudflare: CloudFlareConfig,
    /// The upstream and the paths allowed to reach it. Required.
    webhook: WebhookConfig,
}

#[cfg(test)]
mod tests {
    use super::{AllowedMethod, Config, load};
    use secrecy::ExposeSecret;
    use std::collections::HashSet;

    const CONFIG: &str = r#"
[cloudflare]
client_id = "client_id"
client_secret = "client_secret"

[webhook]
target_base = "https://example.com/"

[webhook.paths]
"/test" = ["ALL"]
"#;

    /// The dialect end to end: the default config path, the defaults that fill in around what
    /// the file supplied, and the `WEBHOOK_REDIRECT_` environment layer on top of it.
    /// `terrace-config` owns the layering and tests it; what this pins is that this crate
    /// wires it to the names an operator actually sets.
    #[test]
    #[expect(
        clippy::result_large_err,
        reason = "figment::Jail::expect_with fixes the closure's error type"
    )]
    fn a_config_file_supplies_the_required_blocks() {
        figment::Jail::expect_with(|jail| {
            jail.clear_env();
            jail.create_file("config.toml", CONFIG)?;
            jail.set_env(
                "WEBHOOK_REDIRECT_CONFIG",
                jail.directory().join("config.toml").display(),
            );

            let config: Config = load().map_err(|e| e.to_string())?;

            // Untouched blocks still materialise with their own defaults.
            assert_eq!(config.server().host(), "127.0.0.1");
            assert_eq!(config.server().port(), &8080);
            assert_eq!(config.telemetry().log_level(), &tracing::Level::INFO);
            assert!(config.telemetry().sentry_dsn().is_none());

            assert_eq!(config.cloudflare().client_id().expose_secret(), "client_id");
            assert_eq!(
                config.cloudflare().client_secret().expose_secret(),
                "client_secret"
            );
            assert_eq!(
                config.webhook().target_base().as_str(),
                "https://example.com/"
            );
            assert_eq!(
                config.webhook().paths().get("/test"),
                Some(&HashSet::from([AllowedMethod::ALL]))
            );
            Ok(())
        });
    }

    #[test]
    #[expect(
        clippy::result_large_err,
        reason = "figment::Jail::expect_with fixes the closure's error type"
    )]
    fn the_environment_layer_overrides_the_file() {
        figment::Jail::expect_with(|jail| {
            jail.clear_env();
            jail.create_file("config.toml", CONFIG)?;
            jail.set_env(
                "WEBHOOK_REDIRECT_CONFIG",
                jail.directory().join("config.toml").display(),
            );
            jail.set_env("WEBHOOK_REDIRECT_SERVER__HOST", "0.0.0.0");
            jail.set_env("WEBHOOK_REDIRECT_SERVER__PORT", "9090");

            let config: Config = load().map_err(|e| e.to_string())?;

            assert_eq!(config.server().host(), "0.0.0.0");
            assert_eq!(config.server().port(), &9090);
            Ok(())
        });
    }

    /// A mounted `Secret` outranks the TOML layer, so a `ConfigMap` carrying a placeholder
    /// credential cannot win over the `Secret` carrying the real one — through the variable
    /// names *this* crate configures, which is the half a dependency cannot pin.
    #[test]
    #[expect(
        clippy::result_large_err,
        reason = "figment::Jail::expect_with fixes the closure's error type"
    )]
    fn a_secrets_directory_outranks_the_file() {
        figment::Jail::expect_with(|jail| {
            jail.clear_env();
            jail.create_file("config.toml", CONFIG)?;
            jail.create_dir("secrets")?;
            jail.create_file("secrets/cloudflare__client_secret", "rotated\n")?;
            jail.set_env(
                "WEBHOOK_REDIRECT_CONFIG",
                jail.directory().join("config.toml").display(),
            );
            jail.set_env(
                "WEBHOOK_REDIRECT_SECRETS_DIR",
                jail.directory().join("secrets").display(),
            );

            let config: Config = load().map_err(|e| e.to_string())?;

            assert_eq!(
                config.cloudflare().client_secret().expose_secret(),
                "rotated"
            );
            Ok(())
        });
    }

    /// The failure the shadow-key rejection exists for: a stale environment variable next to a
    /// mounted secret that has since been rotated fails the boot instead of silently keeping
    /// the old credential.
    #[test]
    #[expect(
        clippy::result_large_err,
        reason = "figment::Jail::expect_with fixes the closure's error type"
    )]
    fn a_key_supplied_twice_fails_the_boot() {
        figment::Jail::expect_with(|jail| {
            jail.clear_env();
            jail.create_file("config.toml", CONFIG)?;
            jail.create_dir("secrets")?;
            jail.create_file("secrets/cloudflare__client_secret", "rotated\n")?;
            jail.set_env(
                "WEBHOOK_REDIRECT_CONFIG",
                jail.directory().join("config.toml").display(),
            );
            jail.set_env(
                "WEBHOOK_REDIRECT_SECRETS_DIR",
                jail.directory().join("secrets").display(),
            );
            jail.set_env("WEBHOOK_REDIRECT_CLOUDFLARE__CLIENT_SECRET", "stale");

            let error = load::<Config>().expect_err("the key is supplied twice");

            assert!(
                error.to_string().contains("cloudflare__client_secret"),
                "the error must name the key: {error}"
            );
            Ok(())
        });
    }

    #[test]
    #[expect(
        clippy::result_large_err,
        reason = "figment::Jail::expect_with fixes the closure's error type"
    )]
    fn a_missing_credential_fails_the_boot() {
        figment::Jail::expect_with(|jail| {
            jail.clear_env();
            jail.create_file(
                "config.toml",
                "[webhook]\ntarget_base = \"https://example.com/\"\n\n[webhook.paths]\n\"/test\" = [\"ALL\"]\n",
            )?;
            jail.set_env(
                "WEBHOOK_REDIRECT_CONFIG",
                jail.directory().join("config.toml").display(),
            );

            let error = load::<Config>().expect_err("cloudflare is required");

            assert!(
                error.to_string().contains("cloudflare"),
                "the error must name the block: {error}"
            );
            Ok(())
        });
    }
}
