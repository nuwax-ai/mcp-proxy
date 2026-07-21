#!/usr/bin/env bash
# Smoke test nuwax-deploy-installer without uv-init (CI / local).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PKG="$ROOT/npm/nuwax-deploy-installer"
INSTALL="${1:-/tmp/doc-parser-smoke}"

export NUWAX_DEPLOY_ROOT="$PKG/vendor"
export NUWAX_DEPLOY_VERSION="$(node -p "require('$PKG/package.json').version")"
BIN="$PKG/vendor/darwin-arm64/deploy-installer"
TEMPLATES="$PKG/vendor/templates/document-parser"

if [[ ! -x "$BIN" ]]; then
  echo "missing $BIN — run assemble-nuwax-deploy-installer.sh first" >&2
  exit 1
fi

rm -rf "$INSTALL"
mkdir -p "$INSTALL"
cp "$PKG/vendor/darwin-arm64/document-parser" "$INSTALL/"
cp "$TEMPLATES/config.example.yml" "$INSTALL/config.yml"
cp "$TEMPLATES/.document-parser.env.example" "$INSTALL/.document-parser.env"
cp "$TEMPLATES/run-server.sh" "$INSTALL/"
chmod +x "$INSTALL/document-parser" "$INSTALL/run-server.sh"

echo "==> doctor"
"$BIN" doctor

echo "==> service install (dry-run)"
"$BIN" document-parser service install --install-dir "$INSTALL" --dry-run

echo "==> service install (--no-start)"
"$BIN" document-parser service install --install-dir "$INSTALL" --no-start

echo "==> service status (expect stopped)"
STATE="$("$BIN" document-parser service status --install-dir "$INSTALL" 2>&1)"
echo "$STATE"
if echo "$STATE" | grep -Eq '^  state: +running'; then
  echo "ERROR: expected stopped after --no-start, but service is running" >&2
  exit 1
fi

echo "==> service uninstall"
"$BIN" document-parser service uninstall --install-dir "$INSTALL"

echo "✅ smoke test passed"
