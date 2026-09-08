#!/usr/bin/env bash
# Assemble npm/nuwax-deploy-installer with release binaries and templates.
#
# Usage: assemble-nuwax-deploy-installer.sh [version] [target]
#   e.g. assemble-nuwax-deploy-installer.sh 0.2.8-beta.3 x86_64-unknown-linux-gnu
#
# Thin orchestrator: platform binaries via build-nuwax-platform-binaries.sh,
# templates + version stamping via stamp-nuwax-manifest.sh. Local single-platform
# dev flow keeps the same args and end state as before the split. CI calls the
# two sub-scripts directly (build per-platform job, stamp once in publish job).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PKG="$ROOT/npm/nuwax-deploy-installer"
VERSION="${1:-$(node -p "require('$PKG/package.json').version")}"
TARGET="${2:-aarch64-apple-darwin}"

case "$TARGET" in
  aarch64-apple-darwin) VENDOR_KEY="darwin-arm64" ;;
  x86_64-unknown-linux-gnu) VENDOR_KEY="linux-x64" ;;
  x86_64-pc-windows-msvc) VENDOR_KEY="windows-x64" ;;
  *)
    echo "unsupported target: $TARGET" >&2
    echo "supported: aarch64-apple-darwin | x86_64-unknown-linux-gnu | x86_64-pc-windows-msvc" >&2
    exit 1
    ;;
esac

"$(dirname "$0")/build-nuwax-platform-binaries.sh" "$TARGET" "$VENDOR_KEY"
"$(dirname "$0")/stamp-nuwax-manifest.sh" "$VERSION"

echo "==> Package ready at $PKG (version $VERSION, $VENDOR_KEY)"
