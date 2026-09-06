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
mod sentry;
mod server;
mod telemetry;
mod webhook;

pub use cloudflare::CloudFlareConfig;
pub use loader::{ConfigError, Explanation, Loaded, Sources, explain, load, load_watched, terrace};
pub use sentry::{SentryConfig, SentryLevel};
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
#[serde(deny_unknown_fields)]
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
    use super::{AllowedMethod, Config, SentryLevel, terrace};
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
            assert!(!config.telemetry().sentry().enabled());

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

    /// `[telemetry.sentry]` is the only block two levels deep, which is one deeper than every
    /// other key this crate configures: the loader has to reach
    /// `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__*`, and a DSN mounted as a file has to outrank the
    /// environment the same way a Cloudflare credential does.
    #[test]
    fn the_nested_sentry_keys_resolve_through_the_dialect() {
        harness().run(|jail| {
            jail.config(CONFIG)?;
            jail.env_key("telemetry.sentry.enabled", true);
            jail.env_key("telemetry.sentry.traces_sample_rate", "0.25");
            jail.env_key("telemetry.sentry.capture_level", "warn");
            jail.secret_key("telemetry.sentry.dsn", "https://key@sentry.example/42\n")?;

            let config: Config = jail.load()?;
            let sentry = config.telemetry().sentry();

            assert!(sentry.enabled());
            assert_eq!(
                sentry
                    .dsn()
                    .as_ref()
                    .expect("the mounted DSN is read")
                    .expose_secret(),
                "https://key@sentry.example/42"
            );
            // With a tolerance rather than `assert_eq!`: `clippy::float_cmp` is on, and what this
            // asserts is that the environment layer reached the key at all, not a bit pattern.
            assert!(
                (sentry.traces_sample_rate() - 0.25).abs() < f32::EPSILON,
                "the environment supplied the nested rate: {}",
                sentry.traces_sample_rate()
            );
            assert_eq!(sentry.capture_level(), SentryLevel::Warn);
            // Untouched keys in a block that was partly supplied still take their own defaults.
            assert_eq!(sentry.environment(), "production");
            assert!(sentry.http_transactions());
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

    /// Every block is `#[serde(deny_unknown_fields)]`, which is what turns a misspelt key from a
    /// silently ignored line into a boot failure naming it.
    ///
    /// Asserted through the loader rather than on one struct, because that is where it could
    /// still go wrong: the layers merge into a single figment before anything is deserialised,
    /// so a provider contributing a key of its own would fail every boot rather than none.
    #[test]
    fn a_misspelt_key_fails_the_boot() {
        harness().run(|jail| {
            jail.config(format!("{CONFIG}\n[server]\nhsot = \"0.0.0.0\"\n"))?;

            let error = jail.load::<Config>().expect_err("`hsot` is not a key");

            assert!(
                error.to_string().contains("hsot"),
                "the error must name the misspelt key: {error}"
            );
            Ok(())
        });
    }

    /// The check `#[config(element_values("…"))]` cannot make for itself.
    ///
    /// A literal spelling list is an assertion about another type's `Deserialize`, and nothing in
    /// the derive reads that. This reads it: every spelling the schema publishes for
    /// `webhook.paths` is fed to the deserialiser the loader would use, so a variant renamed or
    /// an alias dropped fails here instead of shipping a schema that refuses a file the service
    /// takes.
    #[cfg(feature = "config-schema")]
    #[test]
    fn every_published_method_spelling_deserialises() {
        let schema = terrace().schema::<Config>();
        let key = schema
            .keys
            .iter()
            .find(|key| key.path == "webhook.paths")
            .expect("`webhook.paths` is described");
        let published = key
            .constraint
            .as_ref()
            .expect("the key publishes a constraint")["additionalProperties"]["items"]["enum"]
            .as_array()
            .expect("the element publishes its spellings")
            .clone();

        // Six variants, each with an `UPPERCASE` rename and one lowercase alias. Pinned so that
        // a variant added without its alias fails here rather than shrinking the published set
        // by one spelling nobody notices.
        assert_eq!(
            published.len(),
            12,
            "one rename and one alias per variant: {published:?}"
        );

        for value in &published {
            let spelling = value.as_str().expect("a spelling is a string");
            serde_json::from_value::<AllowedMethod>(value.clone()).unwrap_or_else(|error| {
                panic!("the schema publishes `{spelling}`, which does not deserialise: {error}")
            });
        }
    }

    /// The other half of the assertion above: the published list is not merely a subset of what
    /// deserialises, it is the whole of it. `serde`'s derive matches a spelling exactly, so a
    /// case that is in neither the `UPPERCASE` rename nor the lowercase alias must be refused —
    /// which is why no case-folded spelling may be added to the list.
    #[cfg(feature = "config-schema")]
    #[test]
    fn an_unpublished_method_spelling_is_refused() {
        for spelling in ["Get", "gEt", "ALL ", "options"] {
            let value = serde_json::Value::String(spelling.to_owned());
            assert!(
                serde_json::from_value::<AllowedMethod>(value).is_err(),
                "`{spelling}` is not published, so it must not deserialise"
            );
        }
    }
}
