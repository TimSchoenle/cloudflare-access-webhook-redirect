#!/usr/bin/env bash
#
# Check that the Dockerfile's hand-written contract `LABEL` block is still what the generator emits.
#
#   verify-dockerfile-labels.sh <expected-block-file> [dockerfile]
#
# `expected-block-file` holds `config-schema --format dockerfile` output.
#
# This is the cheap half of the check, and it runs a step earlier than the real one: it catches a
# prefix rename in the pull request that renamed it, rather than in the container build. It is not
# a substitute for `verify-contract-labels.sh` and cannot be — a Dockerfile that says the right
# thing still says nothing about the image that was built from it, which is why both exist.
#
# Line endings are normalised on both sides. The repository is authored on Windows and checked out
# with `core.autocrlf`, so the working copy carries CRLF while the committed blob and every CI
# runner carry LF; a check that failed on that difference would fail only on the maintainer's
# machine, which is the worst place for it to be wrong.

set -euo pipefail

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo "usage: $0 <expected-block-file> [dockerfile]" >&2
    exit 2
fi

expected_file="$1"
dockerfile="${2:-Dockerfile}"

for file in "${expected_file}" "${dockerfile}"; do
    if [ ! -s "${file}" ]; then
        echo "error: '${file}' is missing or empty, so there is nothing to compare." >&2
        exit 2
    fi
done

expected="$(tr -d '\r' < "${expected_file}")"
actual="$(tr -d '\r' < "${dockerfile}")"

if [ -z "${expected}" ]; then
    echo "error: '${expected_file}' held nothing after normalisation, so there is no block to \
look for." >&2
    exit 2
fi

# A genuine multi-line substring test, over the whole file rather than line by line: the block
# spans several lines joined with backslashes, and what is worth asserting is that those lines
# appear together and in that order. Checking them one at a time would pass a Dockerfile that had
# them scattered across three unrelated stages.
#
# `[[ … == *"…"* ]]` rather than the obvious `grep -F`, which does the wrong thing quietly: grep
# splits a pattern containing newlines into one pattern per line and matches *any* of them, so a
# Dockerfile carrying only the `prefix` line would pass. That is the same class of silent pass this
# check exists to close. Bash's pattern match takes the needle whole, and needs no interpreter that
# a Git Bash shell might not have.
if [[ "${actual}" == *"${expected}"* ]]; then
    echo "${dockerfile} carries the contract LABEL block the generator emits."
    exit 0
fi

echo "error: ${dockerfile} does not carry the contract LABEL block that \
'config-schema --format dockerfile' emits. The configuration surface changed and the pasted block \
did not follow. Expected to find, verbatim:" >&2
echo >&2
printf '%s\n' "${expected}" >&2
echo >&2
echo "The labels currently in ${dockerfile}:" >&2
grep -n 'dev\.terrace\.config\.' "${dockerfile}" >&2 || echo "  (none at all)" >&2
exit 1
