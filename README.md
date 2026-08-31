<!--
Generated from .github/templates/README.md.hbs — edit that file, not this one. CI renders it on
every pull request and commits the result back to the branch; a push to master whose README.md
does not match its template fails the `Verify` job in .github/workflows/docs.yaml.

The payload is two halves, deep-merged. The repository, release, toolchain and documentation
facts come from TimSchoenle/actions/actions/common/readme-variables, which reads Cargo.toml and
walks docs/. Everything the shared action cannot know comes from one command:

    bash .github/scripts/readme-variables.sh

That script contributes the published image name and the two configuration tables, and the
tables are `cargo run --features config-schema --example config-schema` walking the `Config`
type — so a key that exists below exists in the service.
-->

# cloudflare-access-webhook-redirect

Reverse proxy exposing chosen paths of a Cloudflare Access protected service, with the service token injected.

[![Release](https://img.shields.io/github/v/release/TimSchoenle/cloudflare-access-webhook-redirect?sort=semver)](https://github.com/TimSchoenle/cloudflare-access-webhook-redirect/releases)
[![Build](https://img.shields.io/github/actions/workflow/status/TimSchoenle/cloudflare-access-webhook-redirect/build.yaml?branch=master)](https://github.com/TimSchoenle/cloudflare-access-webhook-redirect/actions/workflows/build.yaml)
[![Coverage](https://codecov.io/gh/TimSchoenle/cloudflare-access-webhook-redirect/branch/master/graph/badge.svg?token=dDUZjsYmh2)](https://codecov.io/gh/TimSchoenle/cloudflare-access-webhook-redirect)
[![License](https://img.shields.io/github/license/TimSchoenle/cloudflare-access-webhook-redirect)](LICENSE)

## What this is

Cloudflare Access protects a service by refusing every request that carries neither a signed
session nor a service token. A webhook sender carries neither, and usually cannot be configured
to.

This proxy stands in front of such a service. A request whose path matches a configured pattern,
with a method that pattern allows, is forwarded with `CF-Access-Client-Id` and
`CF-Access-Client-Secret` attached; everything else is answered `404` and no request is made. The
credential stays in the proxy, so the sender never holds it and the rest of the service stays
behind Access.

The tables under [Configuration](#configuration) are generated from the Rust type that loads the
configuration, as are `config.example.toml` and the labels the image carries, so renaming a key
corrects all three in the commit that renames it.

## Quick start

```toml
# config.toml
[cloudflare]
client_id = "your-client-id"
client_secret = "your-client-secret"

[webhook]
target_base = "https://your-protected-service.com"

[webhook.paths]
"/webhook/.*" = ["ALL"]
"/api/public/.*" = ["POST"]
```

Mount it at `/app/config.toml`, which is where the image looks:

```bash
docker run --rm -p 8080:8080 \
  -e WEBHOOK_REDIRECT_SERVER__HOST=0.0.0.0 \
  -v "$(pwd)/config.toml:/app/config.toml:ro" \
  timmi6790/cloudflare-access-webhook-redirect:v2.0.0
```

`server.host` defaults to `127.0.0.1`, which inside a container answers nothing from outside it.
That is what the environment variable above overrides.

## Table of contents

- [Features](#features)
- [Installation](#installation)
- [Usage](#usage)
- [Configuration](#configuration)
- [Operations](#operations)
- [Compatibility](#compatibility)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [Security](#security)
- [License](#license)

## Features

- GET, POST, PUT, PATCH and DELETE reach the target. A path no pattern matches, or a method its
  pattern does not list, is answered `404` without the target being contacted.
- Every key in `[webhook.paths]` is a regex, anchored at both ends before it is compiled, so
  `/test/` cannot also match `/d/test/d`. Methods are listed per pattern.
- The query string and the request body are forwarded as they arrive. `host` is dropped from the
  headers, and the two Cloudflare Access headers are appended.
- The target's status code and body come back to the caller. Its response headers do not, yet.
- Every request leaves one line at `info` saying whether it was forwarded and where to, because
  the allow list is the only thing standing in front of the credentials.
- **Five configuration layers**, from struct defaults to a mounted secrets directory, all
  spelling a key the same way. A key supplied by two of the last three fails the boot instead of
  being resolved by precedence.
- **Rotation without a restart.** A change under a watched directory rebuilds the HTTP client,
  the compiled patterns, the credentials and the listener.
- The image is `FROM scratch` around one static musl binary and runs as `10001:10001`. It carries
  the configuration contract it was built from at `/config/contract.json`.

## Installation

```bash
docker pull timmi6790/cloudflare-access-webhook-redirect:v2.0.0
```

Images cover `linux/amd64` and `linux/arm64` in one manifest list, so Docker picks the
architecture and no platform flag is needed. Each tag is signed with
[cosign](https://docs.sigstore.dev/) under this repository's GitHub OIDC identity. `latest`
follows releases; pin the tag above where an unattended restart must not change the version.

Compose, a Kubernetes Deployment and the Helm chart are in
[docs/INSTALLATION.md](docs/INSTALLATION.md).

## Usage

Nothing loads until `cloudflare.client_id`, `cloudflare.client_secret`, `webhook.target_base` and
`webhook.paths` are supplied. A boot without one of them names the key, and prints the files and
variables that were read looking for it beside the error.

With the quick start above running, a POST to an allowed path is forwarded to
`https://your-protected-service.com/webhook/build-finished`:

```bash
curl -X POST http://localhost:8080/webhook/build-finished \
  -H 'content-type: application/json' \
  -d '{"status":"ok"}'
```

From a checkout, [`just`](https://github.com/casey/just) is the tooling, and `just` on its own
lists every recipe:

```bash
cargo run                 # reads ./config.toml
just verify               # fmt, clippy, test, doc
just render toml          # config.example.toml, as the docs job writes it
```

## Configuration

[terrace-config](https://github.com/TimSchoenle/terrace-config) resolves five layers, lowest
precedence first:

| # | Layer | Source | Example |
|---|---|---|---|
| 1 | Defaults | built in | `server.port = 8080` |
| 2 | TOML | `$WEBHOOK_REDIRECT_CONFIG`, defaulting to `./config.toml` | `[server]`<br>`port = 8080` |
| 3 | Environment | `WEBHOOK_REDIRECT_`-prefixed, `__`-nested | `WEBHOOK_REDIRECT_SERVER__PORT=8080` |
| 4 | Secrets directory | every key-named file in `$WEBHOOK_REDIRECT_SECRETS_DIR` | `/run/secrets/cloudflare__client_secret` |
| 5 | File indirection | `WEBHOOK_REDIRECT_<KEY>_FILE=/path` | `WEBHOOK_REDIRECT_CLOUDFLARE__CLIENT_SECRET_FILE=/run/secrets/cf` |

All five spell the same field the same way: `__` separates nesting levels and case is folded, so
`cloudflare.client_id` is `WEBHOOK_REDIRECT_CLOUDFLARE__CLIENT_ID` as a variable and
`cloudflare__client_id` as a file name.

**A key supplied by two of the last three layers fails the boot** rather than being resolved by
precedence. A stale environment variable shadowing a mounted secret that has since been rotated
would otherwise keep the proxy running on the revoked credential, and the discrepancy would
surface during an incident rather than during a deploy.

If `$WEBHOOK_REDIRECT_CONFIG` names a directory, every `*.toml` directly inside it is merged in
sorted order, which is how a mounted `ConfigMap` of `10-base.toml` and `20-overrides.toml`
behaves. A missing config file is not an error.

When the layer a value came from is the question, the boot log answers it. At `debug`, and on any
boot that fails to load, the proxy reports every key with the file or variable that supplied it
and anything it is shadowing. The report carries no configuration value, only names, so it is
safe in a log the credentials themselves can never enter.

### Settings

Generated from the `Config` type, so a key that exists in the table exists in the service and a
key that does not is not configurable. `Flags` reads `required` when nothing supplies a default,
and `secret` when the value belongs in a mounted file rather than in this table's `Environment`
column. [`config.example.toml`](config.example.toml) is the same surface as a file to copy.

| TOML | Type | Environment | Default | Flags | Purpose |
|---|---|---|---|---|---|
| `server.host` | `String` | `WEBHOOK_REDIRECT_SERVER__HOST` | `127.0.0.1` | — | Bind address. Containers usually want `0.0.0.0`. |
| `server.port` | `u16` | `WEBHOOK_REDIRECT_SERVER__PORT` | `8080` | — | Bind port. |
| `telemetry.log_level` | `Level` | `WEBHOOK_REDIRECT_TELEMETRY__LOG_LEVEL` | `info` | — | Minimum level emitted by the subscriber (`trace`, `debug`, `info`, `warn`, `error`). |
| `telemetry.sentry.enabled` | `bool` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__ENABLED` | `false` | — | Initialise the Sentry client. `false` installs no client, no panic hook, no `tracing` layer and no HTTP middleware, so every other key here is inert and nothing is sent anywhere. |
| `telemetry.sentry.dsn` | `SecretString` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__DSN` | unset | secret | Ingest URL, `https://<key>@<host>/<project>`. |
| `telemetry.sentry.environment` | `String` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__ENVIRONMENT` | `production` | — | Environment tag on every event, such as `production` or `staging`. |
| `telemetry.sentry.release` | `String` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__RELEASE` | unset | — | Release tag on every event. Defaults to the crate name and version the binary was built from, which is what makes a regression attributable to a deploy. |
| `telemetry.sentry.server_name` | `String` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__SERVER_NAME` | unset | — | Host tag on every event. Left unset, Sentry reports none: the hostname of a replica is infrastructure detail that `send_default_pii` would otherwise gate. |
| `telemetry.sentry.sample_rate` | `f32` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__SAMPLE_RATE` | `1` | — | Fraction of captured events actually sent, `0.0`-`1.0`. A blunt volume cap — it drops whole issues, not repetitions of one — so leave it at `1.0` unless quota forces it. |
| `telemetry.sentry.traces_sample_rate` | `f32` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__TRACES_SAMPLE_RATE` | `0` | — | Fraction of traces this proxy **starts** that are recorded, `0.0`-`1.0`. |
| `telemetry.sentry.capture_level` | `SentryLevel` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__CAPTURE_LEVEL` | `error` | — | Least severe `tracing` level reported as a Sentry **issue**: `off`, `error`, `warn`, `info`, `debug` or `trace`. |
| `telemetry.sentry.breadcrumb_level` | `SentryLevel` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__BREADCRUMB_LEVEL` | `info` | — | Least severe `tracing` level kept as a **breadcrumb** — the trail attached to the next issue. Same spellings as `capture_level`; records at or above it become issues instead. |
| `telemetry.sentry.max_breadcrumbs` | `usize` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__MAX_BREADCRUMBS` | `100` | — | How many breadcrumbs one event carries. |
| `telemetry.sentry.attach_stacktraces` | `bool` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__ATTACH_STACKTRACES` | `true` | — | Attach a stack trace to events that carry none of their own. |
| `telemetry.sentry.send_default_pii` | `bool` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__SEND_DEFAULT_PII` | `false` | — | Send personally identifying data with every event: the client IP, the full request header set, and request bodies of a known content type. |
| `telemetry.sentry.http_transactions` | `bool` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__HTTP_TRANSACTIONS` | `true` | — | Record one Sentry transaction per request, named by the method and the matched path. |
| `telemetry.sentry.span_attributes` | `bool` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__SPAN_ATTRIBUTES` | `false` | — | Copy `tracing` span fields onto the Sentry span as attributes. Off: the request span this proxy opens carries the full request path, and a transaction is stored under a longer retention than a log line. |
| `telemetry.sentry.shutdown_timeout_secs` | `u64` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__SHUTDOWN_TIMEOUT_SECS` | `2` | — | How long process exit waits for queued events to drain. |
| `telemetry.sentry.debug` | `bool` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__DEBUG` | `false` | — | Print the SDK's own diagnostics to stderr. For proving a DSN works, not for running. |
| `cloudflare.client_id` | `SecretString` | `WEBHOOK_REDIRECT_CLOUDFLARE__CLIENT_ID` | — | required, secret | `CF-Access-Client-Id` header value. |
| `cloudflare.client_secret` | `SecretString` | `WEBHOOK_REDIRECT_CLOUDFLARE__CLIENT_SECRET` | — | required, secret | `CF-Access-Client-Secret` header value. |
| `webhook.target_base` | `Url` | `WEBHOOK_REDIRECT_WEBHOOK__TARGET_BASE` | — | required | Base URL of the Cloudflare Access protected service every allowed path is joined onto. |
| `webhook.paths` | `HashMap<String, HashSet<AllowedMethod>>` | `WEBHOOK_REDIRECT_WEBHOOK__PATHS` | — | required | Path regex to the methods allowed on it. |

`webhook.paths` is the one setting the environment layer cannot express in practice. The spelling
in its `Environment` cell is mechanical, and a regex-keyed table has no scalar form, so it comes
from the TOML file:

```toml
[webhook.paths]
"/webhook/.*" = ["ALL"]
"/api/public/.*" = ["GET", "POST"]
```

Two more spellings are derived from every row above rather than listed beside it: append `_FILE`
to the `Environment` cell to name a file holding the value, and substitute `__` for the dots in
the `TOML` cell to name that value's file inside the secrets directory.

Two variables configure the loader itself rather than the service, and are read straight from the
environment:

| Variable | Role | Default | Purpose |
|---|---|---|---|
| `WEBHOOK_REDIRECT_CONFIG` | config | `config.toml` | Names the TOML layer: a file, or a directory whose `*.toml` files are all merged in name order. |
| `WEBHOOK_REDIRECT_SECRETS_DIR` | secrets dir | — | Names a directory of key-named files — a mounted Kubernetes `Secret` volume. Each file supplies the key its name spells. |

### Reloading

The config file's directory, the secrets directory and any `_FILE` target's directory are
watched. When one changes and then goes quiet for 500 ms, the configuration is re-read and the
proxy is rebuilt. A reload that fails to load, or that resolves to the values already running,
leaves the running proxy exactly as it is and logs why.

`telemetry.*` is the exception, and [Operations](#operations) says why.

Releases before 1.0.0 read unprefixed, dot-separated variables.
[docs/MIGRATION.md](docs/MIGRATION.md) maps the old names onto the new ones.

## Operations

`GET /health` answers `200` while the process is up. It is registered ahead of the catch-all, so
a `[webhook.paths]` pattern that also matches `/health` cannot take it over. That endpoint is
what the liveness and readiness probes in [docs/INSTALLATION.md](docs/INSTALLATION.md) call.

`SIGTERM` and `SIGINT` cancel one process-wide token: the listener stops accepting, and the
process exits once in-flight requests have drained. A configuration reload stops the previous
generation through a child of that token, which is what lets the next one bind the same address.

`telemetry.sentry.enabled`, with a DSN, turns on Sentry error reporting and performance tracing.
It is off by default: a DSN is an egress destination for whatever a log line happens to carry, so
turning it on is a decision made once per deployment. Enabled without a usable DSN fails the boot
rather than starting a reporter that reports nowhere.

On, the proxy captures `error` records as issues and keeps `info` and above as breadcrumbs — both
thresholds are keys — installs the SDK's panic hook, and opens one transaction per request. A
request arriving with a `sentry-trace` header is continued rather than restarted, and the header
is rewritten onto the forwarded request, so one webhook delivery reads as a single trace across
the caller, this proxy and the service behind Cloudflare Access. The `traces_sample_rate` key
decides only whether the proxy starts traces of its *own*, and defaults to `0.0`.

Two limits are worth knowing. The Sentry layer sits under `telemetry.log_level`, so a record that
level drops is never reported either. And `telemetry.sentry.send_default_pii` is off and worth
leaving off: every header this proxy receives is forwarded upstream, so the header set of a
webhook delivery routinely carries the caller's own signing secret.

The whole of `telemetry.*` is what a reload cannot apply: the tracing subscriber and the Sentry
client are installed once per process, before the reloadable runtime exists.

The image publishes the configuration surface it was built from. Three `dev.terrace.config.*`
labels name the prefix and the contract path, `/config/contract.json` holds the document, and the
release attaches the same bytes to the image digest as a cosign-signed OCI referrer.

## Compatibility

| | Supported |
| --- | --- |
| Rust | edition 2024 |
| Platforms | `linux/amd64`, `linux/arm64` |
| Helm chart | [`cloudflare-access-webhook-redirect`](https://github.com/TimSchoenle/helm-charts/tree/main/charts/cloudflare-access-webhook-redirect) |

## Documentation

| Document | Purpose |
| --- | --- |
| [Installation](docs/INSTALLATION.md) | Running the published image: a container, Compose, a Kubernetes Deployment, or the Helm chart. |
| [Migrating a configuration forward](docs/MIGRATION.md) | Every rename the configuration surface has been through, newest first, and what to write instead. |
| [docs/config.contract.json](docs/config.contract.json) | — |

## Contributing

Issues and pull requests are welcome. Commits follow
[Conventional Commits](https://www.conventionalcommits.org/), which is what release-please reads
to cut a release.

`just` with no arguments lists the recipes. `just verify` is the fmt, clippy, test and doc a
pull request runs anyway. After changing anything under `src/config/`, run `just regenerate`: it
rewrites `docs/config.contract.json` and the `Dockerfile`'s `LABEL` region from the types, and CI
fails a branch whose generated files do not match what those types produce. This README is one of
them.

## Security

Do not open a public issue for a vulnerability. [SECURITY.md](SECURITY.md) has the reporting
instructions and the supported versions.

## License

`GPL-3.0-only`. [LICENSE](LICENSE) carries the full text.
