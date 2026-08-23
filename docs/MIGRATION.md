# Migrating from the environment-only configuration

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
| `SENTRY_DSN` | `WEBHOOK_REDIRECT_TELEMETRY__SENTRY_DSN` |

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
