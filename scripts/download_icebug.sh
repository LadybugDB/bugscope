#!/usr/bin/env bash
# Download the platform-specific Icebug prebuilt into src-tauri/icebug.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

REPOSITORY="${ICEBUG_GITHUB_REPOSITORY:-Ladybug-Memory/icebug}"
TARGET_DIR="${ICEBUG_TARGET_DIR:-$PROJECT_DIR/src-tauri/icebug}"
VERSION="${ICEBUG_VERSION:-12.8}"

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin)
    ASSET_OS="macos"
    ;;
  Linux)
    ASSET_OS="linux"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    ASSET_OS="win"
    ;;
  *)
    echo "Unsupported OS: $OS" >&2
    exit 1
    ;;
esac

case "$ARCH" in
  arm64|aarch64)
    ASSET_ARCH="arm64"
    ;;
  x86_64)
    if [ "$ASSET_OS" = "win" ]; then
      ASSET_ARCH="amd64"
    else
      ASSET_ARCH="x86_64"
    fi
    ;;
  *)
    echo "Unsupported architecture: $ARCH" >&2
    exit 1
    ;;
esac

if [ "$ASSET_OS" = "win" ]; then
  ARCHIVE="icebug-${ASSET_OS}-${ASSET_ARCH}.zip"
else
  ARCHIVE="icebug-${ASSET_OS}-${ASSET_ARCH}.tar.gz"
fi

LIB_NAME="libnetworkit.so"
case "$ASSET_OS" in
  macos)
    LIB_NAME="libnetworkit.dylib"
    ;;
  win)
    LIB_NAME="networkit.lib"
    ;;
esac

if [ -f "$TARGET_DIR/lib/$LIB_NAME" ] && [ -d "$TARGET_DIR/include/networkit" ]; then
  echo "icebug already exists in $TARGET_DIR"
  exit 0
fi

mkdir -p "$TARGET_DIR"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

DOWNLOAD_URL="https://github.com/${REPOSITORY}/releases/download/${VERSION}/${ARCHIVE}"
echo "Downloading $DOWNLOAD_URL ..."
curl -fSL "$DOWNLOAD_URL" -o "$TMPDIR/$ARCHIVE"

rm -rf "$TARGET_DIR/include" "$TARGET_DIR/lib" "$TARGET_DIR/extlibs"
if [[ "$ARCHIVE" == *.zip ]]; then
  unzip -q "$TMPDIR/$ARCHIVE" -d "$TARGET_DIR"
else
  tar xzf "$TMPDIR/$ARCHIVE" -C "$TARGET_DIR"
fi

if [ ! -f "$TARGET_DIR/lib/$LIB_NAME" ]; then
  echo "Expected Icebug library not found at $TARGET_DIR/lib/$LIB_NAME" >&2
  exit 1
fi

echo "Installed $ARCHIVE to $TARGET_DIR"
