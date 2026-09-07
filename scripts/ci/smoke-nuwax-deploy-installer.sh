#!/usr/bin/env bash
# Smoke test nuwax-deploy-installer without uv-init (CI / local).
# Platform-aware: picks the matching vendor/<key> slice for the current OS/arch.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PKG="$ROOT/npm/nuwax-deploy-installer"
INSTALL="${1:-/tmp/doc-parser-smoke}"

export NUWAX_DEPLOY_ROOT="$PKG/vendor"
export NUWAX_DEPLOY_VERSION="$(node -p "require('$PKG/package.json').version")"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)  VENDOR_KEY="darwin-arm64" ;;
  Darwin-x86_64) VENDOR_KEY="darwin-x64" ;;
  Linux-x86_64)  VENDOR_KEY="linux-x64" ;;
  Linux-aarch64) VENDOR_KEY="linux-arm64" ;;
  *)
    echo "smoke: unsupported platform $(uname -s)-$(uname -m)" >&2
    exit 1
    ;;
esac

BIN="$PKG/vendor/$VENDOR_KEY/deploy-installer"
TEMPLATES="$PKG/vendor/templates/document-parser"

if [[ ! -x "$BIN" ]]; then
  echo "missing $BIN — run assemble-nuwax-deploy-installer.sh first" >&2
  exit 1
fi

rm -rf "$INSTALL"
mkdir -p "$INSTALL"
cp "$PKG/vendor/$VENDOR_KEY/document-parser" "$INSTALL/"
cp "$TEMPLATES/config.example.yml" "$INSTALL/config.yml"
cp "$TEMPLATES/.document-parser.env.example" "$INSTALL/.document-parser.env"
chmod +x "$INSTALL/document-parser"

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

echo "✅ smoke test passed ($VENDOR_KEY)"
