//! Dump the configuration surface for the documentation job and the container build.
//!
//! Every generated artefact comes from here, so none of them can drift from the types an operator
//! is actually configuring:
//!
//! ```text
//! just render markdown-keys                     # the README's settings table
//! just render markdown-loader                   # the README's loader-variable table
//! just render toml           > config.example.toml
//! just render contract       > docs/config.contract.json
//! just render labels         > contract.labels
//! just render dockerfile                        # the Dockerfile's LABEL region
//! ```
//!
//! `just regenerate` is the last two written in place. `.github/workflows/docs.yaml` runs the
//! documentation formats on every pull request: the Markdown is injected into
//! `.github/templates/README.md.hbs`, and the TOML replaces `config.example.toml`. A key added to
//! [`Config`] reaches the README and the example file without either being edited.
//!
//! The three container formats describe the same [`Config`] to a deployment rather than to a
//! person. `contract` is the document published beside the image, `labels` is what the build
//! checks the built image's own labels against, and `dockerfile` is the region the `Dockerfile`
//! carries between its `terrace-config:labels` markers — all from one run of one generator, which
//! is what makes it impossible for the labels to claim a prefix the document does not have. The
//! `rust/config-contract` action is what compares those three against what is committed and
//! against what was built.
//!
//! # What is left here
//!
//! The `--format` vocabulary, the argument parsing, the dispatch across the nine renderings, the
//! printing and the exit code are [`Cli`](terrace_config::schema::cli::Cli). They were the same
//! two hundred lines in every repository that had a generator, which is how three of them ended
//! up disagreeing about how to cut a `LABEL` block back out of a Dockerfile.
//!
//! What is genuinely this service's own is below: the dialect the service boots through, the two
//! blocks that have defaults, the app identity, the JSON Schema's `$id`, the `Docs::Full`
//! rendering `config.example.toml` wants, and the external surface no derive can find.
//!
//! It reads nothing from the environment. A documentation runner has none of the variables it
//! describes set, and that is the point — the schema is what the type *can* carry, not what this
//! machine happens to supply. The three fields that legitimately differ between builds of one
//! source tree are `--version`, `--revision` and `--created`, and they are flags for the same
//! reason: passing them makes the difference explicit, and omitting them keeps `--format contract`
//! byte-reproducible and therefore committable.

use std::process::ExitCode;

use cloudflare_access_webhook_redirect::config::{Config, ServerConfig, TelemetryConfig, terrace};
use serde::Serialize;
use terrace_config::schema::cli::Cli;
use terrace_config::schema::{App, Docs, External, JsonSchema, TomlExample};

/// The `$id` the generated JSON Schema carries.
const SCHEMA_ID: &str =
    "https://github.com/TimSchoenle/cloudflare-access-webhook-redirect/config.schema.json";

/// The blocks that have defaults, in the shape [`Config`] holds them.
///
/// [`Config`] itself cannot stand in here. `cloudflare` and `webhook` are required, so they have
/// no `Default` to construct one from — and a required key reports no default anyway, because
/// printing one would tell an operator they may leave the key out. What is left is exactly the
/// two optional blocks, which means nothing holding a credential is ever serialised.
#[derive(Serialize, Default)]
struct Defaults {
    server: ServerConfig,
    telemetry: TelemetryConfig,
}

fn main() -> ExitCode {
    // The dialect the service boots through, not a second one spelled the same: the variable
    // names in the generated table are the ones `config::load` reads, or the table is fiction.
    let schema = terrace()
        .schema::<Config>()
        .with_defaults_from(&Defaults::default());
    let schema = match schema {
        Ok(schema) => schema,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    Cli::new(
        // Spelled as the image tag spells it. `CARGO_PKG_VERSION` alone yields `1.2.1` where the
        // images are tagged `v1.2.1`, and the field exists to be compared against a tag.
        App::new("cloudflare-access-webhook-redirect")
            .version(concat!("v", env!("CARGO_PKG_VERSION")))
            .source("https://github.com/TimSchoenle/cloudflare-access-webhook-redirect"),
    )
    .json_schema(
        JsonSchema::new()
            .title("cloudflare-access-webhook-redirect configuration")
            .id(SCHEMA_ID),
    )
    // The whole `///` comment rather than its summary. A reference table is read at a glance and
    // wants one sentence per key; `config.example.toml` is read once, while it is being filled in,
    // and the paragraph below the summary is where `webhook.paths` shows the shape of the table
    // nothing else in the file could demonstrate.
    .toml_example(TomlExample::new().docs(Docs::Full))
    // The half of the contract no derive can reach. This service reads **nothing** outside its own
    // `WEBHOOK_REDIRECT_` namespace — the log level is a configuration key rather than `RUST_LOG`,
    // and the listener's address and port are keys too — so there is no `var` here, only ignores.
    //
    // They are not optional. `Unknown::Reject` is the default and the right one, but a pod carries
    // names no image asked for even when the image is `FROM scratch` running one static binary,
    // and a contract that rejects unknown variables has to account for them or fail every
    // deployment.
    .contract_with(&|builder| {
        builder.external(
            External::new()
                // From the container runtime, not from any chart.
                .ignore("HOSTNAME")
                // From the API server — `KUBERNETES_SERVICE_HOST` and its relatives. The service
                // links are a different problem and not one an image can declare: they are named
                // after the *release*, which is why they need `enableServiceLinks: false` on the
                // pod rather than a pattern here.
                .ignore("KUBERNETES_*")
                // The runtime image copies `/usr/share/zoneinfo` in deliberately, which is only
                // worth doing if something is expected to resolve a zone name. Nothing in this
                // service reads `TZ` itself, so it has no owner to declare — an ignore is the
                // honest instrument rather than a `var` claiming this binary consults it.
                .ignore("TZ"),
        )
    })
    .main(schema)
}
