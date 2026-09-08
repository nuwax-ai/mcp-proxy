#!/usr/bin/env bash
# Smoke test nuwax-deploy-installer without uv-init (CI / local).
# Platform-aware: picks the matching vendor/<key> slice for the current OS/arch.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PKG="$ROOT/npm/nuwax-deploy-installer"
INSTALL="${1:-/tmp/doc-parser-smoke}"

export NUWAX_DEPLOY_ROOT="$PKG/vendor"
export NUWAX_DEPLOY_VERSION="$(PKG_JSON="$PKG/package.json" node -p "require(process.env.PKG_JSON).version")"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)  VENDOR_KEY="darwin-arm64" ;;
  Darwin-x86_64) VENDOR_KEY="darwin-x64" ;;
  Linux-x86_64)  VENDOR_KEY="linux-x64" ;;
  Linux-aarch64) VENDOR_KEY="linux-arm64" ;;
  MINGW64*-x86_64|MINGW32*-x86_64|MSYS*-x86_64|CYGWIN*-x86_64)
                 VENDOR_KEY="windows-x64" ;;
  *)
    echo "smoke: unsupported platform $(uname -s)-$(uname -m)" >&2
    exit 1
    ;;
esac

BIN_SUFFIX=""
[[ "$VENDOR_KEY" == windows-* ]] && BIN_SUFFIX=".exe"
BIN="$PKG/vendor/$VENDOR_KEY/deploy-installer$BIN_SUFFIX"
TEMPLATES="$PKG/vendor/templates/document-parser"

if [[ ! -x "$BIN" ]]; then
  echo "missing $BIN — run assemble-nuwax-deploy-installer.sh first" >&2
  exit 1
fi

rm -rf "$INSTALL"
mkdir -p "$INSTALL"
cp "$PKG/vendor/$VENDOR_KEY/document-parser$BIN_SUFFIX" "$INSTALL/"
cp "$TEMPLATES/config.example.yml" "$INSTALL/config.yml"
cp "$TEMPLATES/.document-parser.env.example" "$INSTALL/.document-parser.env"
chmod +x "$INSTALL/document-parser"

echo "==> doctor"
"$BIN" doctor

# voice-cli 冒烟：vendor 目录内直接跑 --version（clap 内建、零模型加载零 GPU）
# ——伴生库同目录，链接期问题（如 Windows DLL 缺失）在此当场暴露
if [[ -x "$PKG/vendor/$VENDOR_KEY/voice-cli$BIN_SUFFIX" ]]; then
  echo "==> voice-cli --version"
  "$PKG/vendor/$VENDOR_KEY/voice-cli$BIN_SUFFIX" --version
else
  echo "==> voice-cli not bundled in this slice — skipping version smoke"
fi

echo "==> service install (dry-run)"
"$BIN" document-parser service install --install-dir "$INSTALL" --dry-run

# CI runner（无交互会话/受限环境）可用 SMOKE_SKIP_SERVICE_LIFECYCLE=1 跳过
# 注册类步骤；完整生命周期由 Windows 实机验证覆盖
if [[ -z "${SMOKE_SKIP_SERVICE_LIFECYCLE:-}" ]]; then
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
fi

echo "✅ smoke test passed ($VENDOR_KEY)"
