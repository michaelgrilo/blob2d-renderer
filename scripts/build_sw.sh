#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SW_WASM="${ROOT_DIR}/sw/target/wasm32-unknown-unknown/debug/bvb_sw.wasm"
SW_OUT_DIR="${ROOT_DIR}/assets/sw"
SW_BOOTSTRAP_SYNC="${ROOT_DIR}/sw_bootstrap_sync.js"
SW_BOOTSTRAP_LEGACY="${ROOT_DIR}/sw_bootstrap.js"
SW_BOOTSTRAP_SYNC_TMP="${SW_BOOTSTRAP_SYNC}.tmp"
SW_BOOTSTRAP_LEGACY_TMP="${SW_BOOTSTRAP_LEGACY}.tmp"

WASM_BINDGEN_BIN="${WASM_BINDGEN_BIN:-${HOME}/Library/Caches/dev.trunkrs.trunk/wasm-bindgen-0.2.108/wasm-bindgen}"
if [[ ! -x "${WASM_BINDGEN_BIN}" ]]; then
  WASM_BINDGEN_BIN="$(command -v wasm-bindgen || true)"
fi
if [[ -z "${WASM_BINDGEN_BIN}" || ! -x "${WASM_BINDGEN_BIN}" ]]; then
  echo "error: wasm-bindgen executable not found. Set WASM_BINDGEN_BIN or install wasm-bindgen." >&2
  exit 1
fi

cargo build --manifest-path "${ROOT_DIR}/sw/Cargo.toml" --target wasm32-unknown-unknown
mkdir -p "${SW_OUT_DIR}"

"${WASM_BINDGEN_BIN}" \
  --target web \
  --out-dir "${SW_OUT_DIR}" \
  --out-name bvb_sw \
  "${SW_WASM}" \
  --no-typescript

# Generate a synchronous service-worker bootstrap so event handlers are
# registered during initial script evaluation.
{
  cat <<'EOF'
import { initSync, start } from "./assets/sw/bvb_sw.js";

// Register placeholder handlers at initial evaluation time so Chromium treats
// this worker as lifecycle-complete before wasm init runs.
self.addEventListener("install", () => {});
self.addEventListener("activate", () => {});
self.addEventListener("fetch", () => {});

function decodeBase64ToBytes(base64) {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}
EOF
  printf 'const wasmBase64 = "'
  base64 < "${SW_OUT_DIR}/bvb_sw_bg.wasm" | tr -d '\n'
  printf '";\n\n'
  cat <<'EOF'
try {
  initSync({ module: decodeBase64ToBytes(wasmBase64) });
  start();
} catch (error) {
  console.error("bvb service worker bootstrap failed", error);
}
EOF
} > "${SW_BOOTSTRAP_SYNC_TMP}"

cp "${SW_BOOTSTRAP_SYNC_TMP}" "${SW_BOOTSTRAP_LEGACY_TMP}"
mv "${SW_BOOTSTRAP_SYNC_TMP}" "${SW_BOOTSTRAP_SYNC}"
mv "${SW_BOOTSTRAP_LEGACY_TMP}" "${SW_BOOTSTRAP_LEGACY}"
