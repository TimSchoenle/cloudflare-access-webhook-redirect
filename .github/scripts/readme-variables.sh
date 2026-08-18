#!/usr/bin/env bash
#
# Emits the variable payload for `.github/templates/README.md.hbs` as strict JSON on stdout.
#
# Every value here is read from something that is already the truth somewhere else, so that the
# README cannot disagree with the thing it documents:
#
#   version, tag    `[package] version` in Cargo.toml — the numbers the release pull request
#                   changes, so that commit carries the matching README with it
#   image           `DOCKER_REPO` in .github/workflows/release-please.yaml — the repository the
#                   release actually pushes to
#   repo            this repository, for the badges and issue links
#   config_loader   the two variables the loader reads, as a Markdown table
#   config_keys     every configuration key, as a Markdown table
#
# The last two come from `cargo run --example config-schema`, which walks the `Config` type. A
# key added to that type reaches the README without the README being edited, and a key removed
# from it cannot be left behind.
#
# Run it yourself to see what CI will render with:
#
#     bash .github/scripts/readme-variables.sh
#
# Deliberately POSIX tools only, no `jq`: it is not present in a default Git for Windows shell,
# and a script that only runs on the CI runner is a script nobody checks their edit against.
set -euo pipefail

manifest="Cargo.toml"
release_workflow=".github/workflows/release-please.yaml"

repo="TimSchoenle/cloudflare-access-webhook-redirect"

# Reads `<key><separator><value>` from a file and rejects anything that would need JSON escaping.
# Every field read this way is a version string or an image path, so the accepted alphabet is the
# whole contract — and constraining it is what makes the `printf` at the bottom safe without a
# JSON encoder.
field() {
    local file="$1" expression="$2" label="$3" pattern="$4" value
    value="$(sed -n "${expression}" "${file}" | head -n1)"

    if [ -z "${value}" ]; then
        echo "readme-variables: no '${label}' in ${file}" >&2
        return 1
    fi

    if ! printf '%s' "${value}" | grep -Eq "${pattern}"; then
        echo "readme-variables: '${label} = ${value}' in ${file} is malformed" >&2
        return 1
    fi

    printf '%s' "${value}"
}

# Encodes stdin as the *body* of a JSON string: no surrounding quotes, so the caller places them.
# `sed` does the escaping, where a replacement is unambiguous, and `awk` only joins the lines —
# awk's own `gsub` replacement text treats backslashes specially and is the wrong tool for it.
json_body() {
    sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/\r$//' -e 's/\t/\\t/g' |
        awk 'BEGIN { ORS = ""; newline = "\\" "n" }
             { if (NR > 1) printf "%s", newline; printf "%s", $0 }'
}

# The generator, once per rendering it produces. Both are Markdown tables destined for the
# template, so both are read through `json_body`.
schema() {
    cargo run --quiet --features config-schema --example config-schema -- --format "$1"
}

# Only `[package]` keys can match the manifest expression: a dependency's version sits inside an
# inline table (`figment = { version = "0.10", … }`) and never starts a line, so anchoring is
# enough.
version="$(field "${manifest}" 's/^version = "\([^"]*\)".*/\1/p' 'version' \
    '^[0-9A-Za-z][0-9A-Za-z.+-]*$')"
image="$(field "${release_workflow}" 's/^ *DOCKER_REPO: *\([^ ]*\) *$/\1/p' 'DOCKER_REPO' \
    '^[0-9a-z][0-9a-z._/-]*$')"

config_loader="$(schema markdown-loader | json_body)"
config_keys="$(schema markdown | json_body)"

printf '{"version":"%s","tag":"v%s","repo":"%s","image":"%s","config_loader":"%s","config_keys":"%s"}\n' \
    "${version}" "${version}" "${repo}" "${image}" "${config_loader}" "${config_keys}"
