#!/usr/bin/env bash
# Runs every automated check: formatting, lints, unit + integration +
# regression tests, and the headless UI smoke test. See docs/testing.md.
set -euo pipefail
cd "$(dirname "$0")/.."

# Homebrew's pkgconf finds the GTK 4 dev files (docs/dev-setup.md).
if [[ -z "${PKG_CONFIG:-}" && -x /home/linuxbrew/.linuxbrew/bin/pkgconf ]]; then
    export PKG_CONFIG=/home/linuxbrew/.linuxbrew/bin/pkgconf
fi
# Tests render on the software adapter (lavapipe): no GPU needed, deterministic.
export BACKSTAGE_WGPU_FALLBACK=1

step() { printf '\n==> %s\n' "$*"; }

step "cargo fmt --check"
cargo fmt --all --check

step "cargo clippy"
cargo clippy --workspace --all-targets --locked -- -D warnings

step "cargo test"
cargo test --workspace --locked

step "UI smoke test (headless cage)"
cargo test -p backstage_tools --test ui_smoke --locked -- --ignored

step "all checks passed"
