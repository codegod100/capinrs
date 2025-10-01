#!/usr/bin/env bash
set -euo pipefail

# build-wasm.sh
# Compile the WASM server crate using wasm-pack and emit artifacts into worker/wasm

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WASM_CRATE_DIR="$ROOT_DIR/wasm"
OUTPUT_DIR="$ROOT_DIR/worker/wasm"

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "Error: wasm-pack is not installed or not on PATH." >&2
  echo "Install it from https://rustwasm.github.io/wasm-pack/installer/ and retry." >&2
  exit 1
fi

cd "$WASM_CRATE_DIR"

# Allow additional arguments to be passed through to wasm-pack
wasm-pack build --target web --out-dir "$OUTPUT_DIR" "$@"

echo "WASM build artifacts written to $OUTPUT_DIR"