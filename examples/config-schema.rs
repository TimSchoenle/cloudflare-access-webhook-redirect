//! Dump the configuration surface for the documentation job and the container build.
//!
//! Every generated artefact comes from here, so none of them can drift from the types an operator
//! is actually configuring:
//!
//! ```text
//! cargo run --features config-schema --example config-schema -- --format markdown
//! cargo run --features config-schema --example config-schema -- --format toml       > config.example.toml
//! cargo run --features config-schema --example config-schema -- --format contract   > docs/config.contract.json
//! cargo run --features config-schema --example config-schema -- --format labels     > contract.labels
//! cargo run --features config-schema --example config-schema -- --format dockerfile
//! ```
//!
//! `.github/workflows/docs.yaml` runs the documentation formats on every pull request: the
//! Markdown is injected into `.github/templates/README.md.hbs`, and the TOML replaces
//! `config.example.toml`. A key added to [`Config`] reaches the README and the example file
//! without either being edited.
//!
//! The three container formats describe the same [`Config`] to a deployment rather than to a
//! person. `contract` is the document published beside the image, `labels` is what the build
//! checks the built image's own labels against, and `dockerfile` is the block pasted into the
//! `Dockerfile` — all from one run of one generator, which is what makes it impossible for the
//! labels to claim a prefix the document does not have.
//!
//! It reads nothing from the environment. A documentation runner has none of the variables it
//! describes set, and that is the point — the schema is what the type *can* carry, not what this
//! machine happens to supply. The two fields that legitimately differ between builds of one
//! source tree, `--revision` and `--created`, are flags for the same reason: passing them makes
//! the difference explicit, and omitting them keeps `--format contract` reproducible.

use std::process::ExitCode;

use cloudflare_access_webhook_redirect::config::{Config, ServerConfig, TelemetryConfig, terrace};
use serde::Serialize;
use terrace_config::schema::{
    App, Column, Contract, DEFAULT_PATH, Docs, External, Schema, TomlExample,
};

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
    let options = match Options::from_args() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    match render(&options) {
        Ok(rendered) => {
            print!("{rendered}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn render(options: &Options) -> Result<String, terrace_config::Error> {
    // The dialect the service boots through, not a second one spelled the same: the variable
    // names in the generated table are the ones `config::load` reads, or the table is fiction.
    let schema = terrace()
        .schema::<Config>()
        .with_defaults_from(&Defaults::default())?;

    Ok(match options.format {
        // The loader variables and the keys are rendered apart because the README documents them
        // apart — the five layers are prose there, and the two tables sit in different sections.
        Format::Markdown => schema.to_markdown_keys(Column::DEFAULT),
        Format::MarkdownLoader => schema.to_markdown_loader(),
        // The whole `///` comment rather than its summary. A reference table is read at a
        // glance and wants one sentence per key; this file is read once, while filling it in,
        // and the paragraph below the summary is where `webhook.paths` shows the shape of the
        // table nothing else in the file could demonstrate.
        Format::Toml => schema.to_toml_example_with(&TomlExample::new().docs(Docs::Full)),
        // The three image formats. `labels` and `dockerfile` are derived from the same
        // [`Contract`] the `contract` format prints, so a prefix rename moves all three at once
        // and the build's label check compares a document against its own labels rather than
        // against a second opinion about what they should be.
        // Printed exactly as `Contract::to_json` writes it, with no newline appended. These are
        // the bytes that get embedded in the image and attached to its digest, and the build
        // compares the two copies byte for byte — so the one thing this arm must not do is
        // decorate them.
        Format::Contract => contract(schema, options)?.to_json()?,
        // Line-oriented, and terminated. A file whose last line has no newline is a file a
        // `while read` loop drops the last line of, which here would mean the prefix label
        // silently never being checked — the exact failure the check exists to catch.
        Format::Labels => contract(schema, options)?
            .labels(DEFAULT_PATH)
            .into_iter()
            .map(|(name, value)| format!("{name}={value}\n"))
            .collect::<String>(),
        Format::Dockerfile => contract(schema, options)?.to_dockerfile_labels(DEFAULT_PATH),
    })
}

/// The whole contract this image publishes: every configuration key, and everything else it reads.
///
/// The `external` half is the part a derive cannot reach. This service reads **nothing** outside
/// its own `WEBHOOK_REDIRECT_` namespace — the log level is a configuration key rather than
/// `RUST_LOG`, and the listener's address and port are keys too — so `env` stays empty and the
/// declarations here are all ignores.
///
/// They are not optional. [`Unknown::Reject`] is the default and the right one, but a pod carries
/// names no image asked for even when the image is `FROM scratch` running one static binary, and
/// a contract that rejects unknown variables has to account for them or fail every deployment.
///
/// [`Unknown::Reject`]: terrace_config::schema::Unknown::Reject
fn contract(schema: Schema, options: &Options) -> Result<Contract, terrace_config::Error> {
    // Spelled as the image tag spells it. `CARGO_PKG_VERSION` alone yields `1.1.0` where the
    // images are tagged `v1.1.0`, and the field exists to be compared against a tag.
    let mut app = App::new("cloudflare-access-webhook-redirect")
        .version(concat!("v", env!("CARGO_PKG_VERSION")))
        .source("https://github.com/TimSchoenle/cloudflare-access-webhook-redirect");

    if let Some(revision) = &options.revision {
        app = app.revision(revision);
    }
    if let Some(created) = &options.created {
        app = app.created(created);
    }

    schema
        .into_contract(app)
        .external(
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
        .build()
}

/// What to emit, and which build to describe.
struct Options {
    format: Format,
    /// The commit this build is of, for `--format contract`.
    revision: Option<String>,
    /// When this build happened, RFC 3339, for `--format contract`.
    created: Option<String>,
}

impl Options {
    fn from_args() -> Result<Self, String> {
        let mut format = None;
        let mut revision = None;
        let mut created = None;
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--format" => {
                    format = Some(match args.next().as_deref() {
                        Some("markdown") => Format::Markdown,
                        Some("markdown-loader") => Format::MarkdownLoader,
                        Some("toml") => Format::Toml,
                        Some("contract") => Format::Contract,
                        Some("labels") => Format::Labels,
                        Some("dockerfile") => Format::Dockerfile,
                        Some(other) => return Err(format!("unknown format `{other}`; {USAGE}")),
                        None => return Err(format!("--format takes a value; {USAGE}")),
                    });
                }
                "--revision" => revision = Some(Self::value(&mut args, "--revision")?),
                "--created" => created = Some(Self::value(&mut args, "--created")?),
                other => return Err(format!("unknown argument `{other}`; {USAGE}")),
            }
        }
        // No default: every caller redirects the output into a file that is committed, and
        // guessing which one is wanted is how the wrong rendering ends up in the wrong file.
        let format = format.ok_or_else(|| format!("--format is required; {USAGE}"))?;

        // Refused rather than ignored. Both fields land in the contract's `app` block and nowhere
        // else, so a build passing them to `--format toml` has wired a step to the wrong format,
        // and silently accepting it would leave the contract without the revision the caller
        // believed it had recorded.
        if !matches!(format, Format::Contract) && (revision.is_some() || created.is_some()) {
            return Err(format!(
                "--revision and --created describe the build, which only `--format contract` \
                 records; {USAGE}"
            ));
        }

        Ok(Self {
            format,
            revision,
            created,
        })
    }

    fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
        args.next()
            .ok_or_else(|| format!("{flag} takes a value; {USAGE}"))
    }
}

/// Which rendering to emit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    /// The settings table, for the README template.
    Markdown,
    /// The loader-variable table, for the same.
    MarkdownLoader,
    /// The commented file an operator copies to `config.toml`.
    Toml,
    /// The contract document published beside the image, and embedded in it.
    Contract,
    /// The contract's three image labels, one `NAME=value` per line, for the build's check.
    Labels,
    /// The same three as a `LABEL` instruction, to paste into the `Dockerfile`.
    Dockerfile,
}

const USAGE: &str = "usage: config-schema --format \
                     markdown|markdown-loader|toml|contract|labels|dockerfile \
                     [--revision <sha>] [--created <rfc3339>]";
