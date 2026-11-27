#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MANIFEST="${SCRIPT_DIR}/platforms/reference/server/riscv-virt.toml"
BIN="${SCRIPT_DIR}/target/release/rhetsim-cli"

FIRMWARE="${RHETSIM_FIRMWARE:-${SCRIPT_DIR}/../opensbi/build/platform/generic/firmware/fw_jump.bin}"
KERNEL="${RHETSIM_KERNEL:-${SCRIPT_DIR}/../linux-6.1/arch/riscv/boot/Image}"

if [ ! -x "$BIN" ]; then
    echo "Building rhetsim-cli (release)..."
    cargo build --release --bin rhetsim-cli --manifest-path "${SCRIPT_DIR}/Cargo.toml"
fi

exec "$BIN" "${MANIFEST}" --firmware "$FIRMWARE" --kernel "$KERNEL" "$@"
