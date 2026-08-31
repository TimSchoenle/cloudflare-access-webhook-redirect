# Local tooling. `just` with no arguments lists what there is.
#
# Everything a contributor has to run by hand lives here rather than in a script under
# `.github/scripts/`, so that the command a README quotes, the command CI runs and the command a
# developer types are one string. Recipes that only wrap `cargo` are here for the same reason:
# the flags are the part people get wrong.
#
#     https://github.com/casey/just
#
# There is deliberately no recipe that *checks* the generated artefacts. Checking is
# `TimSchoenle/actions/actions/rust/config-contract`, which does it in three places this file
# cannot reach — against the Dockerfile, against the committed document, and against the labels a
# built image actually carries. A second implementation here would be a second opinion, and the
# whole point of the shared action is that there is only one.

# The generator, and where its output belongs. These five lines are the only per-repository part
# of this file.
example := "config-schema"
features := "config-schema"
package := ""
contract := "docs/config.contract.json"
dockerfile := "Dockerfile"

# The markers `--format dockerfile` emits around the LABEL block. Defined by terrace-config, not
# by this repository: cutting the region by line count reads correctly right up until a fourth
# label is added, and then compares two of three lines and passes.
begin := "# terrace-config:labels:begin"
end := "# terrace-config:labels:end"

[private]
default:
    @just --list --unsorted

[doc('Rewrite everything generated from src/config.rs')]
regenerate: contract-json dockerfile-labels

[doc('Print one rendering: json|markdown|markdown-loader|markdown-keys|toml|json-schema|contract|labels|dockerfile')]
[group('generate')]
render format:
    #!/usr/bin/env bash
    set -euo pipefail
    args=(run --quiet --example "{{ example }}")
    [ -n "{{ package }}" ] && args+=(-p "{{ package }}")
    [ -n "{{ features }}" ] && args+=(--features "{{ features }}")
    cargo "${args[@]}" -- --format "{{ format }}"

# Rendered without `--version`, `--revision` or `--created`, so it is byte-reproducible across
# rebuilds and releases: the committed copy describes the configuration surface, and the copy
# inside an image additionally names the build it came from.

[doc('Rewrite the committed contract document')]
[group('generate')]
contract-json:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "$(dirname "{{ contract }}")"
    just render contract > "{{ contract }}"
    echo "wrote {{ contract }}"

# The file is rebuilt around the markers rather than substituted in place: `sed` cannot replace a
# multi-line block portably, and `--format dockerfile` emits both markers along with the block
# between them.

[doc('Rewrite the LABEL region in the Dockerfile')]
[group('generate')]
dockerfile-labels:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! grep -qF '{{ begin }}' "{{ dockerfile }}" || ! grep -qF '{{ end }}' "{{ dockerfile }}"; then
        echo "error: {{ dockerfile }} carries no '{{ begin }}' … '{{ end }}' region, so the" >&2
        echo "       generated LABEL block has nowhere to go. Paste 'just render dockerfile'" >&2
        echo "       into it once, markers included." >&2
        exit 1
    fi
    block="$(mktemp)"
    rewritten="$(mktemp)"
    trap 'rm -f "$block" "$rewritten"' EXIT
    just render dockerfile > "$block"
    {
        sed -n "1,/^{{ begin }}\$/p" "{{ dockerfile }}" | sed '$d'
        cat "$block"
        sed -n "/^{{ end }}\$/,\$p" "{{ dockerfile }}" | sed '1d'
    } > "$rewritten"
    mv "$rewritten" "{{ dockerfile }}"
    echo "wrote the LABEL region in {{ dockerfile }}"

[doc('Format, lint, test and document — what a pull request is going to run anyway')]
[group('check')]
verify: fmt lint test doc

[group('check')]
fmt:
    cargo fmt --all

[group('check')]
lint:
    cargo clippy --all-features --all-targets -- -D warnings

[group('check')]
test:
    cargo test --all-features

# `cargo test` runs the doctests; `cargo nextest`, which CI's test job uses, does not. The rustdoc
# lints in Cargo.toml only fire under rustdoc, so `clippy` above never sees them either.

[doc('Render the documentation with warnings as errors, and run the doctests')]
[group('check')]
doc:
    #!/usr/bin/env bash
    set -euo pipefail
    RUSTDOCFLAGS='-D warnings' cargo doc --all-features --no-deps
    cargo test --doc --all-features
