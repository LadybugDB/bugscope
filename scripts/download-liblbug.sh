#!/usr/bin/env bash
# Download prebuilt liblbug shared library from GitHub releases.
# This is required before building the Tauri app.
set -euo pipefail

LBUG_VERSION="0.14.1"
RELEASE_URL="https://github.com/LadybugDB/ladybug/releases/download/v${LBUG_VERSION}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET_DIR="$PROJECT_DIR/src-tauri/liblbug"

# Determine platform
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin)
    ARCHIVE="liblbug-osx-universal.tar.gz"
    ;;
  Linux)
    if [ "$ARCH" = "x86_64" ]; then
      ARCHIVE="liblbug-linux-x86_64.tar.gz"
    elif [ "$ARCH" = "aarch64" ]; then
      ARCHIVE="liblbug-linux-aarch64.tar.gz"
    else
      echo "Unsupported Linux architecture: $ARCH" >&2
      exit 1
    fi
    ;;
  MINGW*|MSYS*|CYGWIN*)
    if [ "$ARCH" = "x86_64" ]; then
      ARCHIVE="liblbug-windows-x86_64.tar.gz"
    elif [ "$ARCH" = "aarch64" ]; then
      ARCHIVE="liblbug-windows-aarch64.tar.gz"
    else
      echo "Unsupported Windows architecture: $ARCH" >&2
      exit 1
    fi
    ;;
  *)
    echo "Unsupported OS: $OS" >&2
    exit 1
    ;;
esac

DOWNLOAD_URL="${RELEASE_URL}/${ARCHIVE}"

# Check if already downloaded
if [ -f "$TARGET_DIR/liblbug.dylib" ] || [ -f "$TARGET_DIR/liblbug.so" ] || [ -f "$TARGET_DIR/liblbug.dll" ]; then
  echo "liblbug already exists in $TARGET_DIR"
  echo "To re-download, remove the directory first: rm -rf $TARGET_DIR"
  exit 0
fi

echo "Downloading liblbug v${LBUG_VERSION} for ${OS}/${ARCH}..."
echo "  URL: $DOWNLOAD_URL"
echo "  Target: $TARGET_DIR"

mkdir -p "$TARGET_DIR"

# Download and extract
TMPFILE="$(mktemp)"
trap "rm -f '$TMPFILE'" EXIT

curl -fSL "$DOWNLOAD_URL" -o "$TMPFILE"
tar xzf "$TMPFILE" -C "$TARGET_DIR"

echo ""
echo "liblbug v${LBUG_VERSION} installed to $TARGET_DIR"
ls -lh "$TARGET_DIR"
