#!/usr/bin/env bash
# The whole check sc-build runs: locked release build (default and verbose),
# clippy with warnings as errors, the PE subsystem, the host tests.
#   sc-build scripts/check.sh
set -euo pipefail
t=x86_64-unknown-uefi
out=${CARGO_TARGET_DIR:-target}
cargo build --locked --release --target $t
cargo clippy --locked --release --target $t --all-targets -- -D warnings
scripts/pe-subsystem.sh "$out/$t/release/stormnic-mlx5.efi"
ls -l "$out/$t/release/stormnic-mlx5.efi"
sha256sum "$out/$t/release/stormnic-mlx5.efi"
cargo build --locked --release --target $t --features verbose
scripts/test-host.sh
