# syntax=docker/dockerfile:1.21@sha256:27f9262d43452075f3c410287a2c43f5ef1bf7ec2bb06e8c9eeb1b8d453087bc

FROM lukemathwalker/cargo-chef:latest-rust-alpine AS chef
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static upx curl jq
# Install sentry-cli
RUN LATEST_VERSION=$(curl -s https://api.github.com/repos/getsentry/sentry-cli/releases/latest | jq -r .tag_name) && \
    wget -qO /usr/local/bin/sentry-cli "https://downloads.sentry-cdn.com/sentry-cli/${LATEST_VERSION}/sentry-cli-Linux-x86_64" && \
    chmod +x /usr/local/bin/sentry-cli
WORKDIR /app

FROM chef AS planner
COPY  . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner  /app/recipe.json recipe.json
RUN cargo chef cook --release --target x86_64-unknown-linux-musl --recipe-path recipe.json

COPY  . .

RUN cargo build --release --target x86_64-unknown-linux-musl

# Upload debug symbols to Sentry before stripping
ARG SENTRY_ORG
ARG SENTRY_PROJECT
ARG VERSION

RUN --mount=type=secret,id=sentry_token \
    if [ -f /run/secrets/sentry_token ]; then \
        sentry-cli debug-files upload \
            --auth-token $(cat /run/secrets/sentry_token) \
            --org ${SENTRY_ORG} \
            --project ${SENTRY_PROJECT} \
            --include-sources \
            /app/target/x86_64-unknown-linux-musl/release/cloudflare-access-webhook-redirect; \
    fi

# Strip and compress after uploading symbols
RUN strip --strip-all /app/target/x86_64-unknown-linux-musl/release/cloudflare-access-webhook-redirect && \
    upx --best --lzma /app/target/x86_64-unknown-linux-musl/release/cloudflare-access-webhook-redirect

FROM alpine:3.23@sha256:25109184c71bdad752c8312a8623239686a9a2071e8825f20acb8f2198c3f659 AS env

# mailcap is used for content type (MIME type) detection
# tzdata is used for timezones info
RUN apk update && \
    apk upgrade --no-cache && \
    apk add --no-cache ca-certificates mailcap tzdata

RUN update-ca-certificates

RUN adduser \
    --disabled-password \
    --gecos "" \
    --home "/nonexistent" \
    --shell "/sbin/nologin" \
    --no-create-home \
    --uid "10001" \
    "appuser"

FROM scratch AS runtime

ARG version=unknown
ARG release=unreleased
ARG vendor=unknown

LABEL org.opencontainers.image.version="${version}" \
      org.opencontainers.image.revision="${release}" \
      org.opencontainers.image.vendor="${vendor}" \
      org.opencontainers.image.title="cloudflare-access-webhook-redirect"

COPY --from=env  /etc/passwd /etc/passwd
COPY --from=env  /etc/group /etc/group
COPY --from=env  /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=env  /usr/share/zoneinfo /usr/share/zoneinfo

WORKDIR /app
COPY --from=builder --chmod=555 /app/target/x86_64-unknown-linux-musl/release/cloudflare-access-webhook-redirect ./app

USER 10001:10001

ENTRYPOINT ["./app"]
