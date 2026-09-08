#!/usr/bin/env bash
# Stamp npm/nuwax-deploy-installer with release version + copy templates.
#
# Usage: stamp-nuwax-manifest.sh <version>
#
# Idempotent: safe to run once per platform build or once after merging
# platform artifacts (CI publish job). Needs node.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PKG="$ROOT/npm/nuwax-deploy-installer"
VERSION="${1:?usage: stamp-nuwax-manifest.sh <version>}"

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

MANIFEST_PATH="$PKG/vendor/templates/manifest.json" TARGET_VERSION="$VERSION" node -e "
const fs = require('fs');
const p = process.env.MANIFEST_PATH;
const m = JSON.parse(fs.readFileSync(p, 'utf8'));
m.version = process.env.TARGET_VERSION;
if (!m.assetVersion) {
  m.assetVersion = process.env.TARGET_VERSION.split('-')[0];
}
fs.writeFileSync(p, JSON.stringify(m, null, 2) + '\n');
"

PACKAGE_JSON_PATH="$PKG/package.json" TARGET_VERSION="$VERSION" node -e "
const fs = require('fs');
const p = process.env.PACKAGE_JSON_PATH;
const pkg = JSON.parse(fs.readFileSync(p, 'utf8'));
pkg.version = process.env.TARGET_VERSION;
fs.writeFileSync(p, JSON.stringify(pkg, null, 2) + '\n');
"

echo "==> stamped version $VERSION (manifest.json + package.json)"
