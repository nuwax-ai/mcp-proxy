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

echo "==> Building deploy-installer + document-parser + voice-cli ($TARGET)"
cd "$ROOT"
cargo build --release -p deploy-installer -p document-parser -p voice-cli --target "$TARGET"

echo "==> Copying binaries"
mkdir -p "$PKG/vendor/$VENDOR_KEY"
REL="target/$TARGET/release"
cp "$REL/deploy-installer" "$PKG/vendor/$VENDOR_KEY/"
cp "$REL/document-parser" "$PKG/vendor/$VENDOR_KEY/"
cp "$REL/voice-cli" "$PKG/vendor/$VENDOR_KEY/"
# voice-cli macOS shared mode: dylibs must sit next to the binary (@loader_path / @rpath)
for lib in libsherpa-onnx-c-api.dylib libonnxruntime.1.24.4.dylib libonnxruntime.dylib; do
  if [[ -f "$REL/$lib" ]]; then
    cp "$REL/$lib" "$PKG/vendor/$VENDOR_KEY/"
  else
    echo "WARN: missing $REL/$lib (voice-cli may fail to start)" >&2
  fi
done
chmod +x "$PKG/vendor/$VENDOR_KEY/"*

echo "==> Copying templates"
mkdir -p "$PKG/vendor/templates/document-parser"
cp crates/document-parser/deploy/config/config.example.yml \
  "$PKG/vendor/templates/document-parser/config.example.yml"
cp crates/document-parser/deploy/systemd/.document-parser.env.example \
  "$PKG/vendor/templates/document-parser/.document-parser.env.example"
cp crates/document-parser/deploy/launchd/com.nuwax.document-parser.plist \
  "$PKG/vendor/templates/document-parser/com.nuwax.document-parser.plist"

mkdir -p "$PKG/vendor/templates/voice-cli"
cp crates/voice-cli/deploy/config.example.yml \
  "$PKG/vendor/templates/voice-cli/config.example.yml"
cp crates/voice-cli/deploy/launchd/com.nuwax.voice-cli.plist \
  "$PKG/vendor/templates/voice-cli/com.nuwax.voice-cli.plist"

node -e "
const fs = require('fs');
const p = '$PKG/vendor/templates/manifest.json';
const m = JSON.parse(fs.readFileSync(p, 'utf8'));
m.version = '$VERSION';
if (!m.assetVersion) {
  m.assetVersion = '$VERSION'.split('-')[0];
}
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
