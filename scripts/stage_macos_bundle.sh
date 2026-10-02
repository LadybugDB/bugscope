#!/usr/bin/env bash
# Stage native deps into a macOS .app bundle and rewrite load commands to
# @rpath. Adapted from ../bugscope/scripts/{stage_macos_frameworks,patch_macos_bundle_dylibs}.sh.
#
#   bash scripts/stage_macos_bundle.sh dist/Bugscope.app
#
# Result:
#   Contents/MacOS/bugscope            (already placed by the caller)
#   Contents/Frameworks/libnetworkit.dylib
#   Contents/Frameworks/libarrow.<soname>.dylib (soname form: the INSTALL'd
#     algo extension references @rpath/libarrow.<soname>, so keep the name)
#   Contents/Frameworks/libomp.dylib
#   Contents/Frameworks/liblbug.0*.dylib
set -euo pipefail

APP_DIR="${1:?usage: stage_macos_bundle.sh <Bugscope.app>}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ICEBUG_DIR="${ICEBUG_DIR:-$PROJECT_DIR/icebug}"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "stage_macos_bundle.sh is macOS-only" >&2
  exit 1
fi

MACOS_DIR="$APP_DIR/Contents/MacOS"
FRAMEWORKS_DIR="$APP_DIR/Contents/Frameworks"
BINARY="$MACOS_DIR/bugscope"
NETWORKIT_SRC="$ICEBUG_DIR/lib/libnetworkit.dylib"
LBUG_DIR="${LBUG_DIR:-$PROJECT_DIR/liblbug}"

[ -f "$BINARY" ] || { echo "missing $BINARY" >&2; exit 1; }
[ -f "$NETWORKIT_SRC" ] || { echo "missing $NETWORKIT_SRC (run scripts/download_icebug.sh)" >&2; exit 1; }
LBUG_SRC="$(ls "$LBUG_DIR"/liblbug.0*.dylib 2>/dev/null | head -n1)"
[ -n "$LBUG_SRC" ] || { echo "missing liblbug.0*.dylib in $LBUG_DIR (run scripts/download-liblbug.sh)" >&2; exit 1; }

dep_path() {
  otool -L "$1" | awk -v pattern="$2" '$1 ~ pattern { print $1; exit }'
}

mkdir -p "$FRAMEWORKS_DIR"
cp -f "$NETWORKIT_SRC" "$FRAMEWORKS_DIR/libnetworkit.dylib"
cp -f "$LBUG_SRC" "$FRAMEWORKS_DIR/$(basename "$LBUG_SRC")"
[ -f "$LBUG_DIR/liblbug.dylib" ] && cp -f "$LBUG_DIR/liblbug.dylib" "$FRAMEWORKS_DIR/liblbug.dylib"

# Keep the SONAME filename (libarrow.<n>.dylib): the INSTALL'd algo extension
# references @rpath/libarrow.<soname>, so renaming would break its dlopen.
# Prefer the vendored copy (scripts/vendor_arrow.sh) so bundled arrow matches
# what the extension was built against; fall back to the brew original.
ARROW_SRC="$(dep_path "$NETWORKIT_SRC" 'libarrow[.].*[.]dylib')"
ARROW_NAME="$(basename "$ARROW_SRC")"
if [ -f "$ICEBUG_DIR/lib/$ARROW_NAME" ]; then
  cp -f "$ICEBUG_DIR/lib/$ARROW_NAME" "$FRAMEWORKS_DIR/$ARROW_NAME"
else
  case "$ARROW_SRC" in
    /opt/homebrew/*|/usr/local/*) cp -fL "$ARROW_SRC" "$FRAMEWORKS_DIR/$ARROW_NAME" ;;
    *) echo "cannot stage arrow dep: $ARROW_SRC" >&2; exit 1 ;;
  esac
fi

OMP_SRC="$(dep_path "$NETWORKIT_SRC" 'libomp[.]dylib')"
case "$OMP_SRC" in
  /opt/homebrew/*|/usr/local/*) cp -f "$OMP_SRC" "$FRAMEWORKS_DIR/libomp.dylib" ;;
  "") echo "no libomp dep in libnetworkit; skipping" ;;
  *) echo "cannot stage omp dep: $OMP_SRC" >&2; exit 1 ;;
esac

change_if_present() {
  if otool -L "$1" | awk '{ print $1 }' | grep -Fxq "$2"; then
    install_name_tool -change "$2" "$3" "$1"
  fi
}

# Well-known system/Homebrew locations, so the runtime-downloaded algo
# extension (whose own LC_RPATHs point at its build machine) resolves its
# @rpath deps with no DYLD_LIBRARY_PATH. Absolute LC_RPATH entries are
# honored by dyld and harmless when the dir is absent.
WELL_KNOWN_RPATHS=(
  /opt/homebrew/lib
  /opt/homebrew/opt/apache-arrow/lib
  /opt/homebrew/opt/libomp/lib
  /usr/local/lib
  /usr/local/opt/apache-arrow/lib
  /usr/local/opt/libomp/lib
)

add_rpath_if_missing() {
  local file="$1" rpath="$2"
  if ! otool -l "$file" | grep -A 2 LC_RPATH | grep -Fq "path $rpath "; then
    install_name_tool -add_rpath "$rpath" "$file"
  fi
}

patch_binary() {
  local binary="$1"
  while IFS= read -r dep; do
    case "$dep" in
      *libarrow*.dylib) change_if_present "$binary" "$dep" "@rpath/$ARROW_NAME" ;;
      *libomp.dylib) change_if_present "$binary" "$dep" "@rpath/libomp.dylib" ;;
      *libnetworkit.dylib) change_if_present "$binary" "$dep" "@rpath/libnetworkit.dylib" ;;
      *liblbug*.dylib) change_if_present "$binary" "$dep" "@rpath/$(basename "$LBUG_SRC")" ;;
    esac
  done < <(otool -L "$binary" | awk 'NR > 1 { print $1 }')
}

chmod u+w "$BINARY" "$FRAMEWORKS_DIR"/*.dylib
patch_binary "$BINARY"
for dylib in "$FRAMEWORKS_DIR"/*.dylib; do
  install_name_tool -id "@rpath/$(basename "$dylib")" "$dylib"
  patch_binary "$dylib"
done
# The shipped binary (and each bundled dylib, for transitive deps) also
# searches Homebrew / system locations.
for rpath in "${WELL_KNOWN_RPATHS[@]}"; do
  add_rpath_if_missing "$BINARY" "$rpath"
  for dylib in "$FRAMEWORKS_DIR"/*.dylib; do
    add_rpath_if_missing "$dylib" "$rpath"
  done
done
chmod -w "$FRAMEWORKS_DIR"/*.dylib 2>/dev/null || true

echo "Staged macOS bundle in $APP_DIR"
