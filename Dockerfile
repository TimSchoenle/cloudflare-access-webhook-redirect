# syntax=docker/dockerfile:1.27@sha256:bde3983e9c939224420ddaf6b784cc30e09b035a4dea01f581230c50809f372e

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

# The configuration contract: every key this image's binary reads, in the shape a deployment can
# validate against. On the builder image so no toolchain is added, and behind `config-schema` so
# the release binary still never links the schema machinery.
FROM builder AS contract-builder

# One stage, one source tree, one compiled generator — so the document and the labels that
# advertise it are two renderings of the same thing rather than two opinions produced at different
# times. That matters more here than it looks: a `LABEL` key cannot be interpolated from anything,
# so the block in `runtime` below is hand-written, and `contract.labels` is what CI checks the
# built image against.
#
# `--release` reuses the profile the stage above already compiled. No `--locked`: `.dockerignore`
# keeps `Cargo.lock` out of the build context.
RUN cargo run --quiet --release --target "$(cat /rust-target)" \
        --features config-schema --example config-schema \
        -- --format contract > /out/contract.json && \
    cargo run --quiet --release --target "$(cat /rust-target)" \
        --features config-schema --example config-schema \
        -- --format labels > /out/contract.labels

# The two generated files and nothing else, so one `--output type=local` against this target puts
# them on the host. Exporting `contract-builder` itself would write the entire Rust toolchain to
# disk to retrieve ten kilobytes of JSON.
FROM scratch AS contract
COPY --from=contract-builder /out/contract.json /out/contract.labels /

FROM alpine:3.24@sha256:28bd5fe8b56d1bd048e5babf5b10710ebe0bae67db86916198a6eec434943f8b AS env

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

# How a deployment discovers the contract below. All three are constants for this service, which
# is why they are a plain `LABEL` block: there is nothing to interpolate, and feeding `--label` on
# the `docker build` command line cannot reach a file produced inside a builder stage without
# running the generator a second time on the host.
#
# So this is generated and pasted, and nothing in the Dockerfile can enforce it. `just regenerate`
# writes the region below from the same generator CI checks it with; what makes it *true* is the
# step that reads these back off the **built image** and compares them with `--format labels` — a
# source diff cannot see a base image that overrode a label or a line deleted on a branch nobody
# diffed.
#
# The markers are the crate's own, and they are what both halves cut on. The region between them
# is compared whole, so a fourth label added tomorrow is inside it rather than one line past the
# end of a three-line window — which is what a `grep -A2` or a line count would have compared, and
# would have passed.
# terrace-config:labels:begin
LABEL dev.terrace.config.contract.version="1" \
      dev.terrace.config.contract.path="/config/contract.json" \
      dev.terrace.config.prefix="WEBHOOK_REDIRECT_"
# terrace-config:labels:end

COPY --from=env  /etc/passwd /etc/passwd
COPY --from=env  /etc/group /etc/group
COPY --from=env  /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=env  /usr/share/zoneinfo /usr/share/zoneinfo

# The offline copy. The registry artifact is what a chart fetches, because it costs no layer pull;
# this is what makes the image self-describing where there is no registry at all — a `docker save`
# tarball, an air-gapped mirror, a future initContainer reading it in-cluster.
COPY --from=contract-builder /out/contract.json /config/contract.json

WORKDIR /app
COPY --from=builder --chmod=555 /out/app ./app

USER 10001:10001

ENTRYPOINT ["./app"]
