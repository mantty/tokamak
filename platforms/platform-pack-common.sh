#!/usr/bin/env bash
# The prologue every platform pack script runs before building its target.
set -euo pipefail

if [[ $# -ne 3 || "$1" != build ]]; then
  echo "usage: platform-pack build TARGET OUTPUT" >&2
  exit 2
fi

target=$2
output=$3
workspace=$PWD
# Where Cargo builds, which `cargo run -p xtask -- platform-pack` passes.
target_dir=${CARGO_TARGET_DIR:?run this recipe through cargo run -p xtask -- platform-pack}
