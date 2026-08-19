#!/usr/bin/env bash
#
# Check that a built image carries the configuration-contract labels its own contract asks for.
#
#   verify-contract-labels.sh <labels.json> <contract.labels> [description]
#
# `labels.json` is the image's label map as a JSON object — `docker inspect` reports it under
# `.Config.Labels`, `crane config` under `.config.Labels`. `contract.labels` is the expected set,
# one `NAME=value` per line, as written by `config-schema --format labels`.
#
# Why this exists at all: a `LABEL` key cannot be interpolated, so the block in the Dockerfile is
# hand-written, and a hand-written block with nothing checking it is the failure mode this whole
# scheme is built to avoid. It checks the **image**, never the Dockerfile — a source diff cannot
# see a base image that overrode a label, a `LABEL` line deleted on a branch nobody diffed, or a
# build argument that silently failed to interpolate.
#
# It mirrors `Contract::verify_labels` in `terrace-config`: presence and equality of the labels the
# contract names, and nothing more. Extra labels pass on purpose — every image carries
# `org.opencontainers.image.*` and whatever its base contributed, and none of that is this
# document's business.
#
# One deliberate difference from the Rust original: every violation is reported before exiting,
# rather than failing on the first. A build that names one missing label and hides two is a second
# round trip through CI for no reason.

set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 <labels.json> <contract.labels> [description]" >&2
    exit 2
fi

labels_json="$1"
expected_file="$2"
description="${3:-the image}"

for file in "${labels_json}" "${expected_file}"; do
    if [ ! -s "${file}" ]; then
        echo "error: '${file}' is missing or empty, so there is nothing to compare. This is a \
broken pipeline step, not a failing check — exiting without a verdict rather than reporting one \
nothing was actually checked for." >&2
        exit 2
    fi
done

# An image that carries no labels at all reports `null`, and so does a caller who read the wrong
# JSON path — `.Config.Labels` where the tool writes `.config.Labels`, or the reverse. The two are
# indistinguishable from here, so `null` becomes the empty set and every expected label is then
# reported missing. That fails closed either way, which is the point: treating `null` as "nothing
# to compare" is exactly how a careless check passes an image carrying none of these.
if ! actual_labels="$(jq -c 'if . == null then {} else . end' "${labels_json}" 2>/dev/null)"; then
    echo "error: '${labels_json}' is not valid JSON, so the image's labels could not be read." >&2
    exit 2
fi

if [ "$(printf '%s' "${actual_labels}" | jq -r 'type')" != "object" ]; then
    echo "error: '${labels_json}' holds a $(printf '%s' "${actual_labels}" | jq -r 'type'), not an \
object of labels. Read the label map itself — '.Config.Labels' from 'docker inspect', \
'.config.Labels' from 'crane config'." >&2
    exit 2
fi

status=0
checked=0

# `|| [ -n "${line}" ]` so a final line with no trailing newline is still processed. The generator
# terminates every line, but a file that lost its last byte to a truncated redirect would
# otherwise skip the label it named — silently, and a silently skipped check is indistinguishable
# from a passing one.
while IFS= read -r line || [ -n "${line}" ]; do
    # Blank lines are skipped; a line with no `=` is not, because it means the expectations file is
    # not the file this script was pointed at.
    [ -n "${line}" ] || continue
    line="${line%$'\r'}"
    [ -n "${line}" ] || continue

    if [ "${line}" = "${line#*=}" ]; then
        echo "error: '${expected_file}' has a line with no '=' in it: '${line}'. Expected \
'config-schema --format labels' output, one NAME=value per line." >&2
        exit 2
    fi

    name="${line%%=*}"
    expected="${line#*=}"
    checked=$((checked + 1))

    actual="$(printf '%s' "${actual_labels}" | jq -r --arg name "${name}" '.[$name] // ""')"

    if [ "${actual}" = "${expected}" ]; then
        continue
    fi

    if [ -z "${actual}" ]; then
        echo "error: ${description} carries no '${name}', so nothing can discover this contract \
from its config blob. 'config-schema --format dockerfile' emits the block to paste." >&2
    else
        echo "error: ${description}'s '${name}' is '${actual}', and this contract's is \
'${expected}'. A label that disagrees with the document is a contract a pipeline will look for in \
the wrong place, or not recognise at all." >&2
    fi
    status=1
done < "${expected_file}"

# A floor rather than an equality: the contract names exactly three labels today, and a later
# version of `terrace-config` adding a fourth should not fail here. Fewer than three means the
# expectations file was truncated, which would otherwise pass by having checked almost nothing.
if [ "${checked}" -lt 3 ]; then
    echo "error: only ${checked} label(s) were named by '${expected_file}', and this contract has \
at least three. The file is truncated, so this run checked less than it appears to have." >&2
    exit 2
fi

if [ "${status}" -ne 0 ]; then
    echo "error: ${description} does not carry the labels its configuration contract declares." >&2
    exit "${status}"
fi

echo "${description}: all ${checked} configuration-contract labels match the generated contract."
