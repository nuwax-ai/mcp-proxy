#!/usr/bin/env bash
# Assemble npm/nuwax-deploy-installer with release binaries and templates.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PKG="$ROOT/npm/nuwax-deploy-installer"
VERSION="${1:-$(node -p "require('$PKG/package.json').version")}"
TARGET="${2:-aarch64-apple-darwin}"
VENDOR_KEY="darwin-arm64"

if [[ "$TARGET" != "aarch64-apple-darwin" ]]; then
  echo "Only aarch64-apple-darwin is supported in phase 1 (got: $TARGET)" >&2
  exit 1
fi

echo "==> Building deploy-installer + document-parser ($TARGET)"
cd "$ROOT"
cargo build --release -p deploy-installer -p document-parser --target "$TARGET"

echo "==> Copying binaries"
mkdir -p "$PKG/vendor/$VENDOR_KEY"
cp "target/$TARGET/release/deploy-installer" "$PKG/vendor/$VENDOR_KEY/"
cp "target/$TARGET/release/document-parser" "$PKG/vendor/$VENDOR_KEY/"
chmod +x "$PKG/vendor/$VENDOR_KEY/"*

echo "==> Copying templates"
mkdir -p "$PKG/vendor/templates/document-parser"
cp crates/document-parser/deploy/config/config.example.yml \
  "$PKG/vendor/templates/document-parser/config.example.yml"
cp crates/document-parser/deploy/systemd/.document-parser.env.example \
  "$PKG/vendor/templates/document-parser/.document-parser.env.example"
cp crates/document-parser/deploy/launchd/run-server.sh \
  "$PKG/vendor/templates/document-parser/run-server.sh"
cp crates/document-parser/deploy/launchd/com.nuwax.document-parser.plist \
  "$PKG/vendor/templates/document-parser/com.nuwax.document-parser.plist"
chmod +x "$PKG/vendor/templates/document-parser/run-server.sh"

node -e "
const fs = require('fs');
const p = '$PKG/vendor/templates/manifest.json';
const m = JSON.parse(fs.readFileSync(p, 'utf8'));
m.version = '$VERSION';
fs.writeFileSync(p, JSON.stringify(m, null, 2) + '\n');
"

node -e "
const fs = require('fs');
const p = '$PKG/package.json';
const pkg = JSON.parse(fs.readFileSync(p, 'utf8'));
pkg.version = '$VERSION';
fs.writeFileSync(p, JSON.stringify(pkg, null, 2) + '\n');
"

echo "==> Package ready at $PKG (version $VERSION)"
