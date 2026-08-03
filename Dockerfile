# syntax=docker/dockerfile:1.23@sha256:2780b5c3bab67f1f76c781860de469442999ed1a0d7992a5efdf2cffc0e3d769

FROM lukemathwalker/cargo-chef:latest-rust-alpine AS chef
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static upx curl jq
# Install sentry-cli. Images are built natively (one runner per target platform),
# so the build machine's architecture is also the target architecture.
RUN LATEST_VERSION=$(curl -s https://api.github.com/repos/getsentry/sentry-cli/releases/latest | jq -r .tag_name) && \
    wget -qO /usr/local/bin/sentry-cli "https://downloads.sentry-cdn.com/sentry-cli/${LATEST_VERSION}/sentry-cli-Linux-$(uname -m)" && \
    chmod +x /usr/local/bin/sentry-cli
WORKDIR /app

FROM chef AS planner
COPY  . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG TARGETARCH

# Resolve the Rust target triple once; later steps read it back from /rust-target.
RUN case "${TARGETARCH}" in \
        amd64) target="x86_64-unknown-linux-musl" ;; \
        arm64) target="aarch64-unknown-linux-musl" ;; \
        *) echo "unsupported target architecture: ${TARGETARCH:-<unset>}" >&2; exit 1 ;; \
    esac && \
    echo "${target}" > /rust-target

COPY --from=planner  /app/recipe.json recipe.json
RUN cargo chef cook --release --target "$(cat /rust-target)" --recipe-path recipe.json

COPY  . .

# The binary is moved to an architecture-independent path so that the runtime
# stage can COPY it without knowing the target triple.
RUN cargo build --release --target "$(cat /rust-target)" && \
    install -D "target/$(cat /rust-target)/release/cloudflare-access-webhook-redirect" /out/app

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
            /out/app; \
    fi

# Strip and compress after uploading symbols
RUN strip --strip-all /out/app && \
    upx --best --lzma /out/app

FROM alpine:3.23@sha256:fd791d74b68913cbb027c6546007b3f0d3bc45125f797758156952bc2d6daf40 AS env

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
COPY --from=builder --chmod=555 /out/app ./app

USER 10001:10001

ENTRYPOINT ["./app"]
