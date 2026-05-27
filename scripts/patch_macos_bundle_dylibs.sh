#!/usr/bin/env bash
# Rewrite macOS release binaries to load bundled dylibs from Contents/Frameworks.
set -euo pipefail

if [ "$(uname -s)" != "Darwin" ]; then
  echo "Skipping macOS dylib patching on $(uname -s)"
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TAURI_DIR="$PROJECT_DIR/src-tauri"

find_frameworks_dirs() {
  find "$TAURI_DIR/target" -type d -name Frameworks -print 2>/dev/null
}

change_if_present() {
  local binary="$1"
  local old="$2"
  local new="$3"

  if otool -L "$binary" | awk '{ print $1 }' | grep -Fxq "$old"; then
    install_name_tool -change "$old" "$new" "$binary"
  fi
}

patch_binary() {
  local binary="$1"

  while IFS= read -r dep; do
    case "$dep" in
      *libarrow*.dylib)
        change_if_present "$binary" "$dep" "@rpath/libarrow.icebug.dylib"
        ;;
      *libomp.dylib)
        change_if_present "$binary" "$dep" "@rpath/libomp.dylib"
        ;;
      *libnetworkit.dylib)
        change_if_present "$binary" "$dep" "@rpath/libnetworkit.dylib"
        ;;
      *liblbug*.dylib)
        change_if_present "$binary" "$dep" "@rpath/liblbug.0.dylib"
        ;;
    esac
  done < <(otool -L "$binary" | awk 'NR > 1 { print $1 }')
}

patched_any=0
while IFS= read -r binary; do
  chmod u+w "$binary"
  patch_binary "$binary"
  patched_any=1
  echo "Patched load commands in $binary"
done < <(find "$TAURI_DIR/target" -path '*/release/bugscope' -type f -perm -111 -print 2>/dev/null)

if [ "$patched_any" -eq 0 ]; then
  echo "Could not find a release bugscope binary under $TAURI_DIR/target" >&2
  exit 1
fi

while IFS= read -r frameworks_dir; do
  [ -d "$frameworks_dir" ] || continue

  for dylib in "$frameworks_dir"/*.dylib; do
    [ -f "$dylib" ] || continue
    chmod u+w "$dylib"
  done

  if [ -f "$frameworks_dir/libarrow.icebug.dylib" ]; then
    install_name_tool -id "@rpath/libarrow.icebug.dylib" "$frameworks_dir/libarrow.icebug.dylib"
    patch_binary "$frameworks_dir/libarrow.icebug.dylib"
  fi

  if [ -f "$frameworks_dir/libnetworkit.dylib" ]; then
    install_name_tool -id "@rpath/libnetworkit.dylib" "$frameworks_dir/libnetworkit.dylib"
    patch_binary "$frameworks_dir/libnetworkit.dylib"
  fi

  if [ -f "$frameworks_dir/liblbug.0.dylib" ]; then
    install_name_tool -id "@rpath/liblbug.0.dylib" "$frameworks_dir/liblbug.0.dylib"
    patch_binary "$frameworks_dir/liblbug.0.dylib"
  fi

  if [ -f "$frameworks_dir/liblbug.dylib" ]; then
    install_name_tool -id "@rpath/liblbug.dylib" "$frameworks_dir/liblbug.dylib"
    patch_binary "$frameworks_dir/liblbug.dylib"
  fi

  if [ -f "$frameworks_dir/libomp.dylib" ]; then
    install_name_tool -id "@rpath/libomp.dylib" "$frameworks_dir/libomp.dylib"
    patch_binary "$frameworks_dir/libomp.dylib"
  fi

  echo "Patched bundled dylibs in $frameworks_dir"
done < <(find_frameworks_dirs)
