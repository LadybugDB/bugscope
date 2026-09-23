#!/usr/bin/env bash
# Stage Windows runtime DLLs for bundling (fixes LadybugDB/bugscope#23).
#
# Tauri v2 MSI/NSIS installers only ship the main exe plus `bundle.resources`
# — DLLs sitting next to the exe are NOT picked up automatically. The Windows
# loader resolves load-time-linked DLLs (lbug_shared, arrow, networkit_state)
# from the application directory, so they must be installed next to the exe.
#
# This script collects them into src-tauri/windows-dlls/, which tauri.conf.json
# maps to the install root via `"resources": { "windows-dlls": "" }`.
# (A map pattern targeting "" flattens files to the install root on both MSI
# and NSIS, and an empty dir is skipped gracefully on macOS/Linux builds.)
#
# On non-Windows platforms this is a no-op that just ensures the (gitignored)
# staging dir exists so the bundler doesn't fail on a missing path.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TAURI_DIR="$PROJECT_DIR/src-tauri"
STAGE_DIR="$TAURI_DIR/windows-dlls"

mkdir -p "$STAGE_DIR"

OS="$(uname -s)"
case "$OS" in
  MINGW*|MSYS*|CYGWIN*|Windows_NT) ;;
  *)
    echo "Not Windows ($OS): leaving $STAGE_DIR empty"
    exit 0
    ;;
esac

count_before="$(ls "$STAGE_DIR" 2>/dev/null | wc -l)"

# 1. LadybugDB shared library (fixes missing lbug_shared.dll).
if [ -f "$TAURI_DIR/liblbug/lbug_shared.dll" ]; then
  cp -f "$TAURI_DIR/liblbug/lbug_shared.dll" "$STAGE_DIR/"
  echo "staged lbug_shared.dll"
else
  echo "WARNING: $TAURI_DIR/liblbug/lbug_shared.dll not found" >&2
fi

# 2. Icebug/NetworkIt runtime DLLs (fixes missing networkit_state.dll).
# Keep the extension DLLs next to networkit_state.dll — they load as a set.
if [ -d "$TAURI_DIR/icebug/lib/networkit" ]; then
  cp -f "$TAURI_DIR/icebug/lib/networkit/"*.dll "$STAGE_DIR/" 2>/dev/null || true
  echo "staged networkit DLLs"
else
  echo "WARNING: $TAURI_DIR/icebug/lib/networkit not found" >&2
fi

# 3. Apache Arrow (fixes missing arrow.dll) + its transitive vcpkg deps.
# lbug_shared.dll links Arrow dynamically; copy every DLL from the vcpkg
# triplet bin dir so transitive deps (fmt, lz4, etc.) are covered too.
VCPKG_BIN=""
for candidate in \
  "${GITHUB_WORKSPACE:-}/vcpkg_installed/x64-windows/bin" \
  "$PROJECT_DIR/vcpkg_installed/x64-windows/bin" \
  "${VCPKG_ROOT:-}/installed/x64-windows/bin"; do
  if [ -d "$candidate" ]; then
    VCPKG_BIN="$candidate"
    break
  fi
done

if [ -n "$VCPKG_BIN" ]; then
  cp -f "$VCPKG_BIN/"*.dll "$STAGE_DIR/" 2>/dev/null || true
  echo "staged vcpkg DLLs from $VCPKG_BIN"
else
  echo "WARNING: vcpkg x64-windows bin dir not found; arrow.dll will be missing" >&2
fi

if [ ! -f "$STAGE_DIR/lbug_shared.dll" ]; then
  echo "ERROR: lbug_shared.dll was not staged" >&2
  exit 1
fi
if [ ! -f "$STAGE_DIR/networkit_state.dll" ]; then
  echo "ERROR: networkit_state.dll was not staged" >&2
  exit 1
fi
if [ ! -f "$STAGE_DIR/arrow.dll" ] && [ ! -f "$STAGE_DIR/libarrow.dll" ]; then
  # arrow.dll is the name in the issue; some builds call it libarrow.dll —
  # warn instead of failing since the name varies by toolchain.
  echo "WARNING: no arrow DLL staged ($(ls "$STAGE_DIR" | tr '\n' ' '))" >&2
fi

echo "Staged $(ls "$STAGE_DIR" | wc -l) DLLs (was $count_before) in $STAGE_DIR"
ls "$STAGE_DIR"
