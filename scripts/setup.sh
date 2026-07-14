#!/usr/bin/env bash
# One-shot setup: download the prebuilt native libraries into src-tauri/.
# The liblbug version is pinned to the lbug crate version in src-tauri/Cargo.toml.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
LIBLBUG_DIR="$PROJECT_DIR/src-tauri/liblbug"
SIGMA_DIR="$PROJECT_DIR/.deps/sigma.js"
SIGMA_REPO="${SIGMA_REPO:-https://github.com/adsharma/sigma.js}"
SIGMA_REF="${SIGMA_REF:-icebug-arrow-graph}"

# Vendored sigma.js fork (same source as .github/workflows/build.yml).
if [ ! -f "$SIGMA_DIR/package.json" ]; then
  rm -rf "$SIGMA_DIR"
  git clone --branch "$SIGMA_REF" --depth 1 "$SIGMA_REPO" "$SIGMA_DIR"
fi
if [ ! -d "$SIGMA_DIR/packages/sigma/dist" ]; then
  npm ci --prefix "$SIGMA_DIR"
  npm run build --prefix "$SIGMA_DIR"
fi

LBUG_VERSION="$(sed -n 's/^lbug = { version = "\([^"]*\)".*/\1/p' "$PROJECT_DIR/src-tauri/Cargo.toml")"
if [ -z "$LBUG_VERSION" ]; then
  echo "Could not read the lbug version from src-tauri/Cargo.toml" >&2
  exit 1
fi

# Wipe a cached prebuilt that does not match the pinned version.
if [ -d "$LIBLBUG_DIR" ] && [ "$(cat "$LIBLBUG_DIR/.version" 2>/dev/null)" != "$LBUG_VERSION" ]; then
  echo "Cached liblbug does not match pinned version $LBUG_VERSION, re-downloading"
  rm -rf "$LIBLBUG_DIR"
fi

LBUG_TARGET_DIR="$LIBLBUG_DIR" LBUG_LIB_KIND="${LBUG_LIB_KIND:-shared}" LBUG_VERSION="$LBUG_VERSION" \
  bash "$SCRIPT_DIR/download-liblbug.sh"
echo "$LBUG_VERSION" > "$LIBLBUG_DIR/.version"

bash "$SCRIPT_DIR/download_icebug.sh"
bash "$SCRIPT_DIR/stage_macos_frameworks.sh"
