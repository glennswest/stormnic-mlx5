#!/usr/bin/env bash
# Host tests of the UEFI-independent modules. Run only on the remote build
# box, through sc-build after pushing.
set -euo pipefail
out=${CARGO_TARGET_DIR:-target}
mkdir -p "$out"
for t in trace layout; do
    rustc --edition=2021 -D warnings --test "test/$t.rs" -o "$out/$t-tests"
    "$out/$t-tests"
done
