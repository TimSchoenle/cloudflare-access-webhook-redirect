#!/usr/bin/env bash
#
# Emits the half of `.github/templates/README.md.hbs`'s payload that no shared action can derive,
# as strict JSON on stdout. `readme-variables` merges it over the repository, release, toolchain
# and documentation facts it reads for itself.
#
# Every value here is read from something that is already the truth somewhere else, so that the
# README cannot disagree with the thing it documents:
#
#   image           `DOCKER_REPO` in .github/workflows/release-please.yaml — the repository the
#                   release actually pushes to
#   config_loader   the two variables the loader reads, as a Markdown table
#   config_keys     every configuration key, as a Markdown table
#
# The last two come from `cargo run --example config-schema`, which walks the `Config` type. A
# key added to that type reaches the README without the README being edited, and a key removed
# from it cannot be left behind.
#
# `version`, `tag` and `repo` used to be here and are gone. They are what `readme-variables` reads
# out of `Cargo.toml` and the event, and a same-named key in this object replaces the one it
# derived — `repo` in particular would have flattened an object carrying the slug, the URL, the
# description and the licence into a bare string.
#
# Run it yourself to see what CI will render with:
#
#     bash .github/scripts/readme-variables.sh
#
# Deliberately POSIX tools only, no `jq`: it is not present in a default Git for Windows shell,
# and a script that only runs on the CI runner is a script nobody checks their edit against.
set -euo pipefail

release_workflow=".github/workflows/release-please.yaml"

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
#
# `markdown-keys` rather than `markdown`: since terrace-config v0.9.0 the latter is both tables at
# once, and this template places them in different sections — the loader's variables beside the
# prose about the five layers, the keys in the settings reference below it.
schema() {
    cargo run --quiet --features config-schema --example config-schema -- --format "$1"
}

image="$(field "${release_workflow}" 's/^ *DOCKER_REPO: *\([^ ]*\) *$/\1/p' 'DOCKER_REPO' \
    '^[0-9a-z][0-9a-z._/-]*$')"

config_loader="$(schema markdown-loader | json_body)"
config_keys="$(schema markdown-keys | json_body)"

printf '{"image":"%s","config_loader":"%s","config_keys":"%s"}\n' \
    "${image}" "${config_loader}" "${config_keys}"
