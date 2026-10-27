# syntax=docker/dockerfile:1.28@sha256:bb22d9815c728170f72750f4e5b0d672e06176142e1d602c7e66c050100b7e5b

# Pinned by digest, which Renovate bumps like the other base images. The floating tag alone let two
# builds of one commit compile with different toolchain images and cargo-chef releases.
FROM lukemathwalker/cargo-chef:latest-rust-alpine@sha256:c48fa69aff194617af79ddc9473a58b946068012721979a9aef9c3e1466ccbd8 AS chef
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static upx

# sentry-cli, for the debug-symbol upload in `builder`. Copied from Sentry's image pinned by digest,
# which Renovate bumps like the other base images. It used to be downloaded from the tag an
# unauthenticated `releases/latest` API call returned: a rate-limited call returned `null` and
# failed the build, and two builds of one commit could ship different CLIs. The index is
# multi-platform, so BuildKit picks the binary for the architecture being built.
COPY --from=getsentry/sentry-cli:3.8.2@sha256:633122f5fbd774716a0ebbcd7ece9c0584aa781cc61a84e7ce57964c635d6559 \
    /bin/sentry-cli /usr/local/bin/sentry-cli

# CI never supplies the upload token, so without this a CLI that cannot start on the build
# architecture would first fail in a release.
RUN sentry-cli --version
WORKDIR /app

# The compiler `rust-toolchain.toml` pins, installed before any stage forks. Without it the
# toolchain would be whichever release the base image happens to carry, and only the stages that
# `COPY . .` would see the pin: `cargo chef cook` and `cargo build` would then compile with
# different rustc releases, and the build would recompile every dependency the cook just cached.
# Copied on its own so that no other change to the tree invalidates this layer.
COPY rust-toolchain.toml ./
RUN rustup toolchain install --no-self-update

FROM chef AS planner
COPY  . .
RUN cargo chef prepare --recipe-path recipe.json

# The dependencies, compiled once and shared by `builder` and `contract-builder` below. Both stages
# start from this layer rather than one from the other, so BuildKit runs the release build and the
# contract generator side by side once the cook is done.
FROM chef AS cook
ARG TARGETARCH

# Resolve the Rust target triple once; later steps read it back from /rust-target.
RUN case "${TARGETARCH}" in \
        amd64) target="x86_64-unknown-linux-musl" ;; \
        arm64) target="aarch64-unknown-linux-musl" ;; \
        *) echo "unsupported target architecture: ${TARGETARCH:-<unset>}" >&2; exit 1 ;; \
    esac && \
    echo "${target}" > /rust-target

# `--locked` here and in the two stages below: the image compiles the versions `Cargo.lock` pins.
# The lockfile used to be kept out of the build context, so `cargo chef prepare` resolved the
# newest compatible versions into the recipe instead. The image then shipped dependencies nobody
# had tested, and any upstream release changed the recipe and discarded this layer.
COPY --from=planner  /app/recipe.json recipe.json
RUN cargo chef cook --release --locked --target "$(cat /rust-target)" --recipe-path recipe.json

FROM cook AS builder
COPY  . .

# The binary is moved to an architecture-independent path so that the runtime
# stage can COPY it without knowing the target triple.
#
# `target/` is removed in the same step, so this layer holds the binary and not the build output
# around it. The layer changes with every commit, and the layer cache exported it on every run.
RUN cargo build --release --locked --target "$(cat /rust-target)" && \
    install -D "target/$(cat /rust-target)/release/cloudflare-access-webhook-redirect" /out/app && \
    rm -rf target

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
# validate against. Behind `config-schema`, so the release binary still never links the schema
# machinery.
#
# `FROM cook` rather than `FROM builder`: the generator needs the source tree and the cooked
# dependencies, and nothing the release build, the Sentry upload or UPX produce. Based on
# `builder`, it waited for all three and added its own 60 to 80 seconds to every build that
# compiled the binary.
FROM cook AS contract-builder
COPY  . .

# One stage, one source tree, one compiled generator — so the document and the labels that
# advertise it are two renderings of the same thing rather than two opinions produced at different
# times. That matters more here than it looks: a `LABEL` key cannot be interpolated from anything,
# so the block in `runtime` below is hand-written, and `contract.labels` is what CI checks the
# built image against.
#
# `--release` reuses the profile `cook` compiled. `target/` is removed for the reason given in
# `builder`, and because the export step in CI pulls this layer from the cache to read the two
# files out of it.
RUN mkdir -p /out && \
    cargo run --quiet --release --locked --target "$(cat /rust-target)" \
        --features config-schema --example config-schema \
        -- --format contract > /out/contract.json && \
    cargo run --quiet --release --locked --target "$(cat /rust-target)" \
        --features config-schema --example config-schema \
        -- --format labels > /out/contract.labels && \
    rm -rf target

# The two generated files and nothing else, so one `--output type=local` against this target puts
# them on the host. Exporting `contract-builder` itself would write the entire Rust toolchain to
# disk to retrieve ten kilobytes of JSON.
FROM scratch AS contract
COPY --from=contract-builder /out/contract.json /out/contract.labels /

FROM alpine:3.24@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6 AS env

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
