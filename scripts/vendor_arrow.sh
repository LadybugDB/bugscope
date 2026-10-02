#!/usr/bin/env bash
# Vendor libarrow + libomp shared libraries into ./icebug/lib so the
# DOWNLOADED algo extension (INSTALL algo) can dlopen: it references
# `@rpath/libarrow.<soname>` / `libarrow.so.<soname>`, resolved against the
# host process rpaths (see .cargo/config.toml) — not against absolute
# system paths. Same mechanism scripts/stage_macos_bundle.sh uses for dist.
#
# macOS: copies from Homebrew (apache-arrow, libomp).
# Linux: copies from ldconfig / well-known apt locations.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
LIB_DIR="${ICEBUG_LIB_DIR:-$PROJECT_DIR/icebug/lib}"
mkdir -p "$LIB_DIR"

search_dirs() {
  if [ "$(uname -s)" = "Darwin" ]; then
    echo "/opt/homebrew/opt/apache-arrow/lib /usr/local/opt/apache-arrow/lib"
    echo "/opt/homebrew/opt/libomp/lib /usr/local/opt/libomp/lib"
  else
    echo "/usr/lib/x86_64-linux-gnu /usr/lib/aarch64-linux-gnu /usr/lib64 /usr/lib /usr/local/lib"
  fi
}

vendored=0
for d in $(search_dirs); do
  [ -d "$d" ] || continue
  for src in "$d"/libarrow.so* "$d"/libarrow.*.dylib "$d"/libomp.so* "$d"/libomp.dylib; do
    [ -e "$src" ] || continue
    case "$src" in
      *.so) ;; # dev symlink, skip (runtime needs the soname file)
      *) : ;;
    esac
    base="$(basename "$src")"
    # Skip bare dev symlinks like libarrow.so / libarrow.dylib.
    case "$base" in
      libarrow.so|libarrow.dylib|libomp.so) continue ;;
    esac
    if [ ! -f "$LIB_DIR/$base" ]; then
      cp -fL "$src" "$LIB_DIR/$base"
      echo "vendored $src -> $LIB_DIR/$base"
      vendored=1
    fi
  done
done

if [ "$vendored" -eq 0 ] && ! ls "$LIB_DIR" | grep -qE "libarrow|libomp"; then
  echo "warning: no libarrow/libomp found to vendor; INSTALL'd extension may fail to LOAD" >&2
fi
