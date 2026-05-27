#!/usr/bin/env bash
# Stage dylibs that Tauri should copy into Contents/Frameworks on macOS.
set -euo pipefail

if [ "$(uname -s)" != "Darwin" ]; then
  echo "Skipping macOS framework staging on $(uname -s)"
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TAURI_DIR="$PROJECT_DIR/src-tauri"
ICEBUG_DIR="${ICEBUG_DIR:-$TAURI_DIR/icebug}"
FRAMEWORKS_DIR="$TAURI_DIR/macos-frameworks"

NETWORKIT_SRC="$ICEBUG_DIR/lib/libnetworkit.dylib"
LBUG_SRC="$TAURI_DIR/liblbug/liblbug.0.dylib"

if [ ! -f "$NETWORKIT_SRC" ]; then
  echo "Missing $NETWORKIT_SRC. Run scripts/download_icebug.sh first." >&2
  exit 1
fi

if [ ! -f "$LBUG_SRC" ]; then
  echo "Missing $LBUG_SRC. Run scripts/download-liblbug.sh first." >&2
  exit 1
fi

dep_path() {
  local lib="$1"
  local pattern="$2"
  otool -L "$lib" | awk -v pattern="$pattern" '$1 ~ pattern { print $1; exit }'
}

resolve_dep() {
  local dep="$1"
  local loader_dir="$2"

  case "$dep" in
    @loader_path/*)
      echo "$loader_dir/${dep#@loader_path/}"
      ;;
    @rpath/*)
      local name="${dep#@rpath/}"
      if [ -f "$ICEBUG_DIR/lib/$name" ]; then
        echo "$ICEBUG_DIR/lib/$name"
      elif [ -f "/opt/homebrew/opt/apache-arrow/lib/$name" ]; then
        echo "/opt/homebrew/opt/apache-arrow/lib/$name"
      elif [ -f "/usr/local/opt/apache-arrow/lib/$name" ]; then
        echo "/usr/local/opt/apache-arrow/lib/$name"
      elif [ -f "/opt/homebrew/opt/libomp/lib/$name" ]; then
        echo "/opt/homebrew/opt/libomp/lib/$name"
      elif [ -f "/usr/local/opt/libomp/lib/$name" ]; then
        echo "/usr/local/opt/libomp/lib/$name"
      else
        echo "$dep"
      fi
      ;;
    *)
      echo "$dep"
      ;;
  esac
}

NETWORKIT_DIR="$(cd "$(dirname "$NETWORKIT_SRC")" && pwd)"
ARROW_DEP="$(dep_path "$NETWORKIT_SRC" 'libarrow[.].*[.]dylib')"
OMP_DEP="$(dep_path "$NETWORKIT_SRC" 'libomp[.]dylib')"

if [ -z "$ARROW_DEP" ]; then
  echo "Could not find libarrow dependency in $NETWORKIT_SRC" >&2
  exit 1
fi

ARROW_SRC="$(resolve_dep "$ARROW_DEP" "$NETWORKIT_DIR")"
if [ ! -f "$ARROW_SRC" ]; then
  echo "Icebug requires $ARROW_DEP, but it was not found at $ARROW_SRC" >&2
  echo "Install the matching Apache Arrow C++ package before building." >&2
  exit 1
fi

if [ -n "$OMP_DEP" ]; then
  OMP_SRC="$(resolve_dep "$OMP_DEP" "$NETWORKIT_DIR")"
  if [ ! -f "$OMP_SRC" ]; then
    echo "Icebug requires $OMP_DEP, but it was not found at $OMP_SRC" >&2
    echo "Install libomp before building." >&2
    exit 1
  fi
else
  OMP_SRC=""
fi

mkdir -p "$FRAMEWORKS_DIR"
cp -f "$NETWORKIT_SRC" "$FRAMEWORKS_DIR/libnetworkit.dylib"
cp -f "$ARROW_SRC" "$FRAMEWORKS_DIR/libarrow.icebug.dylib"
cp -f "$LBUG_SRC" "$FRAMEWORKS_DIR/liblbug.0.dylib"
cp -f "$LBUG_SRC" "$FRAMEWORKS_DIR/liblbug.dylib"

if [ -n "$OMP_SRC" ]; then
  cp -f "$OMP_SRC" "$FRAMEWORKS_DIR/libomp.dylib"
fi

chmod u+w "$FRAMEWORKS_DIR"/*.dylib

echo "Staged macOS frameworks in $FRAMEWORKS_DIR"
echo "  libnetworkit.dylib <= $NETWORKIT_SRC"
echo "  libarrow.icebug.dylib <= $ARROW_SRC (from $ARROW_DEP)"
echo "  liblbug.0.dylib <= $LBUG_SRC"
if [ -n "$OMP_SRC" ]; then
  echo "  libomp.dylib <= $OMP_SRC"
fi
