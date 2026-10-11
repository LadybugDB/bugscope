#!/usr/bin/env bash
# Install Apache Arrow 25 on macOS CI from a pinned Homebrew formula.
#
# The icebug prebuilt (scripts/download_icebug.sh, ICEBUG_VERSION 13.2)
# links `@rpath`-style `libarrow.2500.dylib` (Arrow 25.x), but Homebrew core
# only ever carries the latest release (26.x as of Oct 2026) and has no
# versioned `apache-arrow@25` formula. Installing latest would give
# compile-time headers and a runtime dylib that libnetworkit cannot load
# (and scripts/stage_macos_bundle.sh would fail to find libarrow.2500).
#
# So we pour the last 25.0.1 bottle (commit 8b54558 in Homebrew/homebrew-core,
# "apache-arrow: update 25.0.1_9 bottle") via a local formula file. Its
# `cellar: :any` bottles (arm64_tahoe / arm64_sequoia) install like a normal
# keg under /opt/homebrew/opt/apache-arrow, so vendor_arrow.sh and
# stage_macos_bundle.sh work unmodified. Build-only deps (boost, llvm@22,
# ...) still resolve from current core.
#
# When the icebug prebuilt moves to a newer Arrow, update the pin below
# (and the apt pins in .github/workflows/build.yml) to match.
set -euo pipefail

FORMULA_SHA="8b5455835733a8476e7cb60f59b8d187437ac43b" # last apache-arrow 25.0.1 bottle
FORMULA_URL="https://raw.githubusercontent.com/Homebrew/homebrew-core/${FORMULA_SHA}/Formula/a/apache-arrow.rb"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "install-arrow25-macos.sh is macOS-only" >&2
  exit 1
fi

if [ -f /opt/homebrew/opt/apache-arrow/lib/libarrow.2500.dylib ]; then
  echo "Arrow 25 already installed in /opt/homebrew/opt/apache-arrow"
  exit 0
fi

# Modern Homebrew only installs formulae that live in a tap, so stage the
# pinned formula in a throwaway local tap (fully supported, unlike
# `brew install /path/to/file.rb`, which brew now rejects).
TAP_DIR="$(brew --repository)/Library/Taps/local/homebrew-arrow"
mkdir -p "$TAP_DIR/Formula"

echo "Fetching pinned apache-arrow formula (${FORMULA_SHA}) ..."
curl -fSL "$FORMULA_URL" -o "$TAP_DIR/Formula/apache-arrow.rb"
grep -q '25\.0\.1' "$TAP_DIR/Formula/apache-arrow.rb" || {
  echo "pinned formula is no longer Arrow 25.0.1; update FORMULA_SHA" >&2
  exit 1
}

# Pour the bottle (fail fast instead of a surprise hour-long source build
# if the runner image moves to an unbottled macOS version).
brew install --force-bottle local/arrow/apache-arrow

[ -f /opt/homebrew/opt/apache-arrow/lib/libarrow.2500.dylib ] || {
  echo "expected libarrow.2500.dylib missing after install" >&2
  exit 1
}
echo "Installed Apache Arrow 25 ($(otool -L /opt/homebrew/opt/apache-arrow/lib/libarrow.2500.dylib | head -n1))"
