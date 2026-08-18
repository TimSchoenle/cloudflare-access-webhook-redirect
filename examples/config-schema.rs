//! Dump the configuration surface for the documentation job.
//!
//! Both generated artefacts come from here, so neither can drift from the types an operator is
//! actually configuring:
//!
//! ```text
//! cargo run --features config-schema --example config-schema -- --format markdown
//! cargo run --features config-schema --example config-schema -- --format toml > config.example.toml
//! ```
//!
//! `.github/workflows/docs.yaml` runs both on every pull request: the Markdown is injected into
//! `.github/templates/README.md.hbs`, and the TOML replaces `config.example.toml`. A key added
//! to [`Config`] reaches the README and the example file without either being edited.
//!
//! It reads nothing from the environment. A documentation runner has none of the variables it
//! describes set, and that is the point — the schema is what the type *can* carry, not what this
//! machine happens to supply.

use std::process::ExitCode;

use cloudflare_access_webhook_redirect::config::{Config, ServerConfig, TelemetryConfig, terrace};
use serde::Serialize;
use terrace_config::schema::{Column, Docs, TomlExample};

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
    let format = match Format::from_args() {
        Ok(format) => format,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    match render(format) {
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

fn render(format: Format) -> Result<String, terrace_config::Error> {
    // The dialect the service boots through, not a second one spelled the same: the variable
    // names in the generated table are the ones `config::load` reads, or the table is fiction.
    let schema = terrace()
        .schema::<Config>()
        .with_defaults_from(&Defaults::default())?;

    Ok(match format {
        // The loader variables and the keys are rendered apart because the README documents them
        // apart — the five layers are prose there, and the two tables sit in different sections.
        Format::Markdown => schema.to_markdown_keys(Column::DEFAULT),
        Format::MarkdownLoader => schema.to_markdown_loader(),
        // The whole `///` comment rather than its summary. A reference table is read at a
        // glance and wants one sentence per key; this file is read once, while filling it in,
        // and the paragraph below the summary is where `webhook.paths` shows the shape of the
        // table nothing else in the file could demonstrate.
        Format::Toml => schema.to_toml_example_with(&TomlExample::new().docs(Docs::Full)),
    })
}

/// Which rendering to emit.
#[derive(Clone, Copy)]
enum Format {
    /// The settings table, for the README template.
    Markdown,
    /// The loader-variable table, for the same.
    MarkdownLoader,
    /// The commented file an operator copies to `config.toml`.
    Toml,
}

impl Format {
    fn from_args() -> Result<Self, String> {
        let mut format = None;
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--format" => {
                    format = Some(match args.next().as_deref() {
                        Some("markdown") => Self::Markdown,
                        Some("markdown-loader") => Self::MarkdownLoader,
                        Some("toml") => Self::Toml,
                        Some(other) => return Err(format!("unknown format `{other}`; {USAGE}")),
                        None => return Err(format!("--format takes a value; {USAGE}")),
                    });
                }
                other => return Err(format!("unknown argument `{other}`; {USAGE}")),
            }
        }
        // No default: every caller redirects the output into a file that is committed, and
        // guessing which one is wanted is how the wrong rendering ends up in the wrong file.
        format.ok_or_else(|| format!("--format is required; {USAGE}"))
    }
}

const USAGE: &str = "usage: config-schema --format markdown|markdown-loader|toml";
