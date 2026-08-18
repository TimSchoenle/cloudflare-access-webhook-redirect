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
pub use loader::{ConfigError, Explanation, Loaded, Sources, explain, load, load_watched, terrace};
pub use server::ServerConfig;
pub use telemetry::TelemetryConfig;
pub use webhook::{AllowedMethod, WebhookConfig};

use serde::Deserialize;

/// Everything the proxy reads at boot.
///
/// The root of the described surface: with the `config-schema` feature on, `examples/config-schema.rs`
/// walks this type to generate the README's settings table and `config.example.toml`, so a key
/// added below reaches both without either being edited.
#[derive(Debug, Clone, Deserialize, Getters)]
#[cfg_attr(feature = "config-schema", derive(terrace_config::schema::Describe))]
#[getset(get = "pub")]
pub struct Config {
    /// Where the listener binds. Defaults throughout, so the block may be omitted.
    #[serde(default)]
    #[cfg_attr(feature = "config-schema", config(nested))]
    server: ServerConfig,
    /// Logging and error reporting. Installed once per process, so a reload does not apply it.
    #[serde(default)]
    #[cfg_attr(feature = "config-schema", config(nested))]
    telemetry: TelemetryConfig,
    /// The Cloudflare Access service token. Required.
    #[cfg_attr(feature = "config-schema", config(nested))]
    cloudflare: CloudFlareConfig,
    /// The upstream and the paths allowed to reach it. Required.
    #[cfg_attr(feature = "config-schema", config(nested))]
    webhook: WebhookConfig,
}

#[cfg(test)]
mod tests {
    use super::{AllowedMethod, Config, terrace};
    use secrecy::ExposeSecret;
    use std::collections::HashSet;
    use terrace_config::explain::Layer;
    use terrace_config::testing::Harness;

    const CONFIG: &str = r#"
[cloudflare]
client_id = "client_id"
client_secret = "client_secret"

[webhook]
target_base = "https://example.com/"

[webhook.paths]
"/test" = ["ALL"]
"#;

    /// A sandbox over the loader this service actually boots through.
    ///
    /// Built from [`terrace`] rather than from a prefix, so every name the jail writes — the
    /// config path, the secrets directory, an environment key — is derived from the same dialect
    /// `main` loads with. A test that spelled `WEBHOOK_REDIRECT_SECRETS_DIR` out by hand would
    /// keep passing after the loader was pointed at a different variable, while testing one
    /// nothing reads.
    fn harness() -> Harness {
        Harness::over(terrace())
    }

    /// The dialect end to end: the default config path, the defaults that fill in around what
    /// the file supplied, and the `WEBHOOK_REDIRECT_` environment layer on top of it.
    /// `terrace-config` owns the layering and tests it; what this pins is that this crate
    /// wires it to the names an operator actually sets.
    #[test]
    fn a_config_file_supplies_the_required_blocks() {
        harness().run(|jail| {
            jail.config(CONFIG)?;

            let config: Config = jail.load()?;

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
    fn the_environment_layer_overrides_the_file() {
        harness().run(|jail| {
            jail.config(CONFIG)?;
            jail.env_key("server.host", "0.0.0.0");
            jail.env_key("server.port", 9090);

            let config: Config = jail.load()?;

            assert_eq!(config.server().host(), "0.0.0.0");
            assert_eq!(config.server().port(), &9090);
            Ok(())
        });
    }

    /// A mounted `Secret` outranks the TOML layer, so a `ConfigMap` carrying a placeholder
    /// credential cannot win over the `Secret` carrying the real one — through the variable
    /// names *this* crate configures, which is the half a dependency cannot pin.
    ///
    /// Asserted on the layer as well as on the value: a secret that a leftover environment
    /// variable happened to be shadowing would load the same string and test nothing.
    #[test]
    fn a_secrets_directory_outranks_the_file() {
        harness().run(|jail| {
            jail.config(CONFIG)?;
            jail.secret_key("cloudflare.client_secret", "rotated\n")?;

            let config: Config = jail.load()?;
            assert_eq!(
                config.cloudflare().client_secret().expose_secret(),
                "rotated"
            );

            let origin = jail
                .explain()?
                .origin("cloudflare.client_secret")
                .expect("the key is reported")
                .effective()
                .clone();
            assert!(
                matches!(origin, Layer::SecretsFile(_)),
                "the mounted secret must be the effective layer, not {origin:?}"
            );
            Ok(())
        });
    }

    /// The failure the shadow-key rejection exists for: a stale environment variable next to a
    /// mounted secret that has since been rotated fails the boot instead of silently keeping
    /// the old credential.
    #[test]
    fn a_key_supplied_twice_fails_the_boot() {
        harness().run(|jail| {
            jail.config(CONFIG)?;
            jail.secret_key("cloudflare.client_secret", "rotated\n")?;
            jail.env_key("cloudflare.client_secret", "stale");

            let error = jail
                .load::<Config>()
                .expect_err("the key is supplied twice");

            assert!(
                error.to_string().contains("cloudflare__client_secret"),
                "the error must name the key: {error}"
            );
            Ok(())
        });
    }

    #[test]
    fn a_missing_credential_fails_the_boot() {
        harness().run(|jail| {
            jail.config(
                "[webhook]\ntarget_base = \"https://example.com/\"\n\n[webhook.paths]\n\"/test\" = [\"ALL\"]\n",
            )?;

            let error = jail.load::<Config>().expect_err("cloudflare is required");

            assert!(
                error.to_string().contains("cloudflare"),
                "the error must name the block: {error}"
            );
            Ok(())
        });
    }
}
