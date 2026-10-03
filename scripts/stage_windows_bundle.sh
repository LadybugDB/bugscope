#!/usr/bin/env bash
# Stage Windows runtime DLLs for the release zip.
#
#   bash scripts/stage_windows_bundle.sh <dist-dir> [extra-dll-dir ...]
#
# Copies every runtime DLL flat next to `bugscope.exe` AND preserves the
# historical `icebug/lib` + `liblbug` subdir layout. The flat copies are the
# fix for https://github.com/LadybugDB/bugscope/issues/5: the Windows loader
# resolves load-time DLLs *before* `main` runs and searches only the exe
# directory, System32, the Windows directory, and `PATH` — there is no
# RPATH/`@rpath` equivalent, so DLLs sitting in subdirs (`lbug_shared.dll`,
# `networkit_state.dll`, `arrow.dll`, …) were invisible and the exe refused
# to start. Hand-copying those then surfaced round two (`bz2.dll`,
# `brotlidec.dll`, `brotlienc.dll`, `lz4.dll`, …): Arrow's transitive
# compression deps, which the old packaging never shipped because it globbed
# only `arrow*.dll`. This script therefore stages *all* DLLs from the
# caller-supplied dirs (vcpkg `bin`, OpenSSL `bin`, …), not just arrow's.
#
# Typical CI invocation (pwsh step):
#   bash scripts/stage_windows_bundle.sh "dist/$name" "$vcpkgBin" "$osslBin"
#
# Fails fast when a known load-time dep is still missing flat next to the
# exe, so a packaging regression breaks the build instead of the user.
set -euo pipefail

DIST_DIR="${1:?usage: stage_windows_bundle.sh <dist-dir> [extra-dll-dir ...]}"
shift

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# The pwsh CI step may hand us `D:\...` paths under git-bash: normalize when
# cygpath exists, otherwise pass through (macOS/Linux shells lack it).
norm() {
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -u "$1" 2>/dev/null || printf '%s' "$1"
  else
    printf '%s' "$1"
  fi
}
DIST_DIR="$(norm "$DIST_DIR")"

EXE="$DIST_DIR/bugscope.exe"
[ -f "$EXE" ] || { echo "stage_windows_bundle.sh: missing $EXE" >&2; exit 1; }

staged=0
stage_flat() {
  # stage_flat <dll-path>: copy basename next to the exe, first wins on
  # (case-insensitive) name clashes so vcpkg/icebug/liblbug duplicates
  # cannot silently overwrite each other mid-loop.
  local src="$1" base dest
  base="$(basename "$src")"
  dest="$DIST_DIR/$base"
  if [ -f "$dest" ]; then
    return 0
  fi
  cp -f "$src" "$dest"
  echo "staged $base (from $src)"
  staged=$((staged + 1))
}

# 1) icebug DLLs, recursive: networkit_state.dll lives in lib/networkit/
#    (the win prebuilt statically links networkit.lib and splits only
#    GlobalState into that DLL), plus the parallel-leiden extension DLLs.
if [ -d "$PROJECT_DIR/icebug/lib" ]; then
  while IFS= read -r dll; do
    [ -n "$dll" ] && stage_flat "$dll"
  done < <(find "$PROJECT_DIR/icebug/lib" -iname '*.dll' -type f | sort) || true
fi

# 2) shared liblbug (lbug_shared.dll) — top level only.
if [ -d "$PROJECT_DIR/liblbug" ]; then
  while IFS= read -r dll; do
    [ -n "$dll" ] && stage_flat "$dll"
  done < <(find "$PROJECT_DIR/liblbug" -maxdepth 1 -iname '*.dll' -type f | sort) || true
fi

# 3) caller-supplied dirs (vcpkg bin, OpenSSL bin, ...): every DLL, top
#    level. This is the issue-#5 round-two fix: arrow's bz2 / brotli / lz4 /
#    zstd / snappy / ssl / crypto family lives here.
for raw in "$@"; do
  [ -n "$raw" ] || continue
  dir="$(norm "$raw")"
  if [ ! -d "$dir" ]; then
    echo "warning: extra dll dir not found, skipping: $raw" >&2
    continue
  fi
  while IFS= read -r dll; do
    [ -n "$dll" ] && stage_flat "$dll"
  done < <(find "$dir" -maxdepth 1 -iname '*.dll' -type f | sort) || true
done

# 4) keep the historical subdir layout too (with the real `networkit/`
#    nesting, which the old flattened copy dropped): harmless, and it keeps
#    dev-tree-shaped path references working.
mkdir -p "$DIST_DIR/icebug" "$DIST_DIR/liblbug"
if [ -d "$PROJECT_DIR/icebug/lib" ]; then
  cp -rf "$PROJECT_DIR/icebug/lib" "$DIST_DIR/icebug/"
fi
if [ -d "$PROJECT_DIR/liblbug" ]; then
  find "$PROJECT_DIR/liblbug" -maxdepth 1 -iname '*.dll' -type f \
    -exec cp -f {} "$DIST_DIR/liblbug/" \;
fi

# 5) fail fast when a known load-time dep is still missing flat: a
#    regression must break the build, not the user's launch.
missing=0
for want in lbug_shared.dll networkit_state.dll; do
  if [ ! -f "$DIST_DIR/$want" ]; then
    echo "ERROR: required DLL not staged flat next to bugscope.exe: $want" >&2
    missing=1
  fi
done
if ! ls "$DIST_DIR"/arrow*.dll >/dev/null 2>&1; then
  echo "ERROR: no arrow*.dll staged flat in $DIST_DIR" >&2
  missing=1
fi
# Transitive compression family (issue #5 round two): warn, don't fail — the
# exact vcpkg-mapped names drift (bz2/bzip2/brotli/lz4/zstd/snappy/...).
if ! ls "$DIST_DIR" | grep -qiE 'bz2|bzip|brotli|lz4|zstd|snappy'; then
  echo "warning: no compression-family DLL (bz2/brotli/lz4/...) staged flat; arrow may fail at runtime" >&2
fi
if [ "$missing" -ne 0 ]; then
  exit 1
fi

echo "staged $staged DLL(s) flat next to bugscope.exe"
