# Migrating a configuration forward

Every rename the configuration surface has been through, newest first, and what to write instead.

## Sentry moved into a block of its own

`telemetry.sentry_dsn` is gone. Sentry is now `[telemetry.sentry]`, with an explicit switch in
front of it: a DSN on its own no longer turns reporting on.

| Before | After |
|---|---|
| `WEBHOOK_REDIRECT_TELEMETRY__SENTRY_DSN` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__ENABLED=true` **and** `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__DSN` |

```toml
[telemetry.sentry]
enabled = true
# The DSN belongs in the secrets directory or behind a `_FILE`, not in a committed file: it
# carries the project key that authorises event submission.
```

Two keys rather than one because the switch is what the rest of the block hangs off. `enabled`
set without a usable DSN now fails the boot, where a DSN that resolved to an empty string used to
install a client that reported nowhere and said nothing about it.

The old spelling is not read, and it does not fail quietly either: the published configuration
contract declares the variables this image consults, so a deployment still exporting
`WEBHOOK_REDIRECT_TELEMETRY__SENTRY_DSN` is a variable the image never asked for, and the chart's
contract check reports it.

The two keys above are the whole of the move: every other key in the block has a default, and the
full list is in the settings table in the README. What those defaults change is worth knowing,
though, because on balance a migrated deployment reports *more* than it did:

- `error` records now arrive as issues and `info` and above are kept as breadcrumbs. Before,
  only panics and explicitly captured events were reported. Both thresholds are keys —
  `capture_level` and `breadcrumb_level` — and `capture_level = "off"` restores the old reach.
- One transaction is started per request, and an inbound `sentry-trace` header is continued and
  propagated to the target. None of those transactions are *kept* until
  `traces_sample_rate` is raised above its default of `0.0`; `http_transactions = false` opts
  out of starting them at all.
- Events no longer carry the SDK's OS, host and runtime context block. It reported constants of
  the image, which the release tag already names, and collecting it cost a dependency tree for
  platforms this image does not target.

## Migrating from the environment-only configuration

The mapping from the environment variables releases before 1.0.0 read to the keys 1.0.0 reads.

Release 1.0.0 moved the configuration onto terrace-config. Everything before it read unprefixed,
dot-separated environment variables.

| Before | After |
|---|---|
| `SERVER.HOST` | `WEBHOOK_REDIRECT_SERVER__HOST` |
| `SERVER.PORT` | `WEBHOOK_REDIRECT_SERVER__PORT` |
| `CLOUDFLARE.CLIENT_ID` | `WEBHOOK_REDIRECT_CLOUDFLARE__CLIENT_ID` |
| `CLOUDFLARE.CLIENT_SECRET` | `WEBHOOK_REDIRECT_CLOUDFLARE__CLIENT_SECRET` |
| `WEBHOOK.TARGET_BASE` | `WEBHOOK_REDIRECT_WEBHOOK__TARGET_BASE` |
| `WEBHOOK.PATHS` | the `[webhook.paths]` table in the config file |
| `LOG_LEVEL` | `WEBHOOK_REDIRECT_TELEMETRY__LOG_LEVEL` |
| `SENTRY_DSN` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY__DSN`, with `..._SENTRY__ENABLED=true` |

`WEBHOOK.PATHS` packed every pattern into one string as `<regex>:<methods>` separated by `; `,
which made a pattern containing either separator unspellable. The table has no such limit:

```text
WEBHOOK.PATHS=/webhook/.*:ALL; /api/public/.*:POST,GET
```

becomes

```toml
[webhook.paths]
"/webhook/.*" = ["ALL"]
"/api/public/.*" = ["POST", "GET"]
```

The two credentials were plain environment variables and can stay that way, but the layer that
replaced them is the reason to move: name a file `cloudflare__client_secret` in the secrets
directory, or point `WEBHOOK_REDIRECT_CLOUDFLARE__CLIENT_SECRET_FILE` at one, and the value never
enters the process environment or a `docker inspect`. Rotating that file rebuilds the proxy
without a restart.

Supplying the same key from two of the environment, secrets-directory and `_FILE` layers fails
the boot rather than resolving by precedence, so a leftover variable from the old spelling is a
failed deploy rather than a proxy quietly running on the credential you thought you had replaced.
