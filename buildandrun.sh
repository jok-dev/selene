#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DRIVING_EMPIRE_DIR="C:/Users/Jok/work/voldex/Driving-Empire"
SELENE_BIN="$SCRIPT_DIR/target/release/selene.exe"

cargo build --release

pushd "$DRIVING_EMPIRE_DIR" >/dev/null
"$SELENE_BIN" generate-roblox-std
"$SELENE_BIN" src/ --config tools/selene/selene-errors-only.toml
popd >/dev/null