#!/usr/bin/env bash
# Stage native deps into a macOS .app bundle and rewrite load commands to
# @rpath. Adapted from ../bugscope/scripts/{stage_macos_frameworks,patch_macos_bundle_dylibs}.sh.
#
#   bash scripts/stage_macos_bundle.sh dist/Bugscope.app
#
# Result:
#   Contents/MacOS/bugscope            (already placed by the caller)
#   Contents/Frameworks/*.dylib       (libnetworkit, liblbug, libarrow,
#     libomp PLUS the full transitive Homebrew closure: aws-sdk-cpp,
#     aws-c-*, brotli, lz4, snappy, zstd, openssl, ...)
#
# Background (https://github.com/LadybugDB/bugscope/issues/10): the bundle
# used to stage only libnetworkit/liblbug/libarrow/libomp. libarrow itself
# links Homebrew libs (libaws-cpp-sdk-*, libbrotli*, liblz4, libsnappy,
# libzstd, ...) which resolved on CI/dev machines via the WELL_KNOWN_RPATHS
# fallback to /opt/homebrew, but crashed at launch on clean machines with:
#   Library not loaded: .../libaws-cpp-sdk-identity-management.dylib
# Mirrors the Windows fix for #5 (stage_windows_bundle.sh): ship every
# load-time dep so the .app is standalone with no Homebrew requirement.
#
# The INSTALL'd algo extension references @rpath/libarrow.<soname>, so the
# SONAME filename (libarrow.<n>.dylib) is preserved, not renamed.
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
LBUG_BASENAME="$(basename "$LBUG_SRC")"
cp -f "$LBUG_SRC" "$FRAMEWORKS_DIR/$LBUG_BASENAME"

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

# ---------------------------------------------------------------------------
# Recursive transitive closure (#10): copy every non-system dylib referenced
# (directly or transitively) by the binary + Frameworks into Frameworks.
# ---------------------------------------------------------------------------
is_system_dep() {
  case "$1" in
    /usr/lib/*|/System/*) return 0 ;;
    *) return 1 ;;
  esac
}

# Resolve a load command to a file on disk. Prints the source path or nothing
# when unresolvable (caller falls back to a wider search).
resolve_dep_source() {
  local dep="$1" referrer="$2"
  local base refdir
  case "$dep" in
    @rpath/*|@loader_path/*|@executable_path/*)
      base="$(basename "$dep")"
      refdir="$(dirname "$referrer")"
      # Sibling of the referrer first (already-staged copies reference each
      # other this way after rewriting).
      if [ -f "$refdir/$base" ]; then
        printf '%s' "$refdir/$base"
        return 0
      fi
      ;;
    /opt/homebrew/*|/usr/local/*|/opt/local/*)
      if [ -f "$dep" ]; then
        printf '%s' "$dep"
        return 0
      fi
      base="$(basename "$dep")"
      ;;
    *) return 1 ;;
  esac
  # Project dirs first: the downloaded prebuilts (./icebug/lib, ./liblbug)
  # shadow any same-named Homebrew install (e.g. a `brew install ladybug`
  # liblbug.0.dylib must never win over the pinned ./liblbug copy).
  local d
  for d in "$LBUG_DIR/$base" "$ICEBUG_DIR/lib/$base"; do
    if [ -f "$d" ]; then
      printf '%s' "$d"
      return 0
    fi
  done
  for d in /opt/homebrew/opt/*/lib/"$base" /opt/homebrew/lib/"$base" \
           /usr/local/opt/*/lib/"$base" /usr/local/lib/"$base"; do
    if [ -f "$d" ]; then
      printf '%s' "$d"
      return 0
    fi
  done
  return 1
}

stage_transitive_deps() {
  local iter=0 max_iters=20
  local changed=1
  while [ "$changed" -eq 1 ] && [ "$iter" -lt "$max_iters" ]; do
    changed=0
    iter=$((iter + 1))
    # Snapshot: files present at the start of this pass (newly staged files
    # are picked up on the next pass).
    local targets=("$BINARY")
    local f
    for f in "$FRAMEWORKS_DIR"/*.dylib; do
      [ -e "$f" ] || continue
      targets+=("$f")
    done
    local t dep src base self_id
    for t in "${targets[@]}"; do
      # A dylib's own install-ID (otool -D) is listed as the first otool -L
      # line but is not a dependency: staging it would duplicate the file
      # under the ID basename (e.g. liblbug.0.21.1.dylib's ID
      # @rpath/liblbug.0.dylib pulled a stale brew ladybug copy + its
      # openssl4 deps). Skip it.
      self_id="$(otool -D "$t" 2>/dev/null | tail -n1)"
      while IFS= read -r dep; do
        [ -n "$dep" ] || continue
        if [ "$dep" = "$self_id" ]; then
          continue
        fi
        if is_system_dep "$dep"; then
          continue
        fi
        case "$dep" in
          @rpath/*|@loader_path/*|@executable_path/*|/opt/homebrew/*|/usr/local/*|/opt/local/*) ;;
          *) continue ;; # framework / system / already-relative: skip
        esac
        base="$(basename "$dep")"
        if [ -f "$FRAMEWORKS_DIR/$base" ]; then
          continue # already bundled
        fi
        if src="$(resolve_dep_source "$dep" "$t")"; then
          # cp -L: dereference Cellar symlinks but keep the requested
          # soname (e.g. libaws-c-common.1.dylib).
          cp -fL "$src" "$FRAMEWORKS_DIR/$base"
          echo "staged transitive dep $base (from $src, via $t)"
          changed=1
        else
          echo "warning: cannot resolve transitive dep $dep (via $t)" >&2
        fi
      done < <(otool -L "$t" | awk 'NR > 1 { print $1 }')
    done
  done
  if [ "$iter" -ge "$max_iters" ]; then
    echo "warning: transitive dep staging did not converge after $max_iters passes" >&2
  fi
}

stage_transitive_deps

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

# Rewrite every bundled-basename reference to @rpath/<basename> (generalizes
# the old four-lib switch: now covers the whole transitive closure).
patch_to_rpath() {
  local target="$1" dep base
  while IFS= read -r dep; do
    [ -n "$dep" ] || continue
    case "$dep" in
      @rpath/*)
        base="$(basename "$dep")"
        if [ -f "$FRAMEWORKS_DIR/$base" ] && [ "$dep" != "@rpath/$base" ]; then
          change_if_present "$target" "$dep" "@rpath/$base"
        fi
        ;;
      /opt/homebrew/*|/usr/local/*|/opt/local/*|@loader_path/*|@executable_path/*)
        base="$(basename "$dep")"
        if [ -f "$FRAMEWORKS_DIR/$base" ]; then
          change_if_present "$target" "$dep" "@rpath/$base"
        fi
        ;;
    esac
  done < <(otool -L "$target" | awk 'NR > 1 { print $1 }')
}

chmod u+w "$BINARY" "$FRAMEWORKS_DIR"/*.dylib
patch_to_rpath "$BINARY"
for dylib in "$FRAMEWORKS_DIR"/*.dylib; do
  install_name_tool -id "@rpath/$(basename "$dylib")" "$dylib"
  patch_to_rpath "$dylib"
done
# Second pass: newly rewritten @rpath entries can expose further absolute
# refs in already-patched dylibs? Re-run once for idempotence.
for dylib in "$FRAMEWORKS_DIR"/*.dylib; do
  patch_to_rpath "$dylib"
done
# Each bundled dylib also searches its own dir, so sibling @rpath loads work
# even when the host binary's rpaths are not consulted (e.g. dlopen chains).
for dylib in "$FRAMEWORKS_DIR"/*.dylib; do
  add_rpath_if_missing "$dylib" "@loader_path"
done
# The shipped binary (and each bundled dylib, for transitive deps) also
# searches Homebrew / system locations.
for rpath in "${WELL_KNOWN_RPATHS[@]}"; do
  add_rpath_if_missing "$BINARY" "$rpath"
  for dylib in "$FRAMEWORKS_DIR"/*.dylib; do
    add_rpath_if_missing "$dylib" "$rpath"
  done
done

# Fail fast instead of shipping a bundle that crashes at launch (#10): every
# non-system load must be @rpath/<bundled-file>.
missing=0
check_target() {
  local target="$1" dep base self_id
  self_id="$(otool -D "$target" 2>/dev/null | tail -n1)"
  while IFS= read -r dep; do
    [ -n "$dep" ] || continue
    if [ "$dep" = "$self_id" ]; then
      continue # own install-ID, not a dependency
    fi
    if is_system_dep "$dep"; then
      continue
    fi
    case "$dep" in
      @rpath/*)
        base="$(basename "$dep")"
        if [ ! -f "$FRAMEWORKS_DIR/$base" ]; then
          echo "error: $target references missing bundled lib: $dep" >&2
          missing=1
        fi
        ;;
      /opt/homebrew/*|/usr/local/*|/opt/local/*)
        echo "error: $target still references absolute Homebrew path: $dep" >&2
        missing=1
        ;;
    esac
  done < <(otool -L "$target" | awk 'NR > 1 { print $1 }')
}
check_target "$BINARY"
for dylib in "$FRAMEWORKS_DIR"/*.dylib; do
  check_target "$dylib"
done
if [ "$missing" -ne 0 ]; then
  echo "error: unbundled non-system dylib references remain (see above)" >&2
  exit 1
fi
# Compat symlinks for the liblbug install-ID family: the prebuilt's ID is
# @rpath/liblbug.0.dylib while the staged file is versioned
# (liblbug.0.21.x.dylib, rewritten above). Anything resolving the old ID
# basename finds the same file. Symlinks are created last so the patch loop
# above only ever operates on real files.
if [ "$LBUG_BASENAME" != "liblbug.0.dylib" ]; then
  ln -sf "$LBUG_BASENAME" "$FRAMEWORKS_DIR/liblbug.0.dylib"
fi
ln -sf "$LBUG_BASENAME" "$FRAMEWORKS_DIR/liblbug.dylib"
chmod -w "$FRAMEWORKS_DIR"/*.dylib 2>/dev/null || true

echo "Staged macOS bundle in $APP_DIR ($(ls "$FRAMEWORKS_DIR"/*.dylib | wc -l | tr -d ' ') dylibs)"
