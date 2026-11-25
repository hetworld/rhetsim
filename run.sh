#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MANIFEST="${SCRIPT_DIR}/platforms/reference/server/riscv-virt.toml"

cargo build --release --bin rhetsim-cli --manifest-path "${SCRIPT_DIR}/Cargo.toml"

exec cargo run --release --bin rhetsim-cli --manifest-path "${SCRIPT_DIR}/Cargo.toml" -- "${MANIFEST}" "$@"
