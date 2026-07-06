#!/usr/bin/env bash
# 预下载 sherpa-onnx 预编译 C 库到 docker/sherpa-cache/（Dockerfile.voice-cli 编译期注入用）。
#
# 背景：sherpa-onnx-sys 1.13.3 的 build.rs 默认联网下载预编译 C 库 tar.bz2，
#   github 阻断时容器内编译会卡死。本脚本走 gh-proxy 镜像预下载到本地，
#   Dockerfile 通过 ENV SHERPA_ONNX_ARCHIVE_DIR=/app/docker/sherpa-cache 让 build.rs
#   命中本地 tar、跳过联网（详见 sherpa-onnx-sys build.rs:136-140）。
#
# 用法:
#   bash docker/fetch-sherpa.sh           # 默认 amd64（Docker 构建用）
#   bash docker/fetch-sherpa.sh amd64
#   bash docker/fetch-sherpa.sh arm64
#   bash docker/fetch-sherpa.sh all       # 两个 linux 架构都下（双架构构建用）
#   bash docker/fetch-sherpa.sh osx-arm64 # mac 本地开发（配 OUT_DIR，见下）
#
# 可用环境变量覆盖:
#   SHERPA_PROXY   gh-proxy 镜像地址（默认 https://gh-proxy.com）
#   SHERPA_VERSION 版本号（默认 1.13.3，须与 Cargo.lock 的 sherpa-onnx-sys 一致）
#   OUT_DIR        输出目录（默认 docker/sherpa-cache；mac 本地开发设
#                  ~/.cache/sherpa-onnx-prebuilt，sherpa-onnx-sys build.rs 默认查找路径）
set -euo pipefail

VERSION="${SHERPA_VERSION:-1.13.3}"
PROXY="${SHERPA_PROXY:-https://gh-proxy.com}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${OUT_DIR:-$SCRIPT_DIR/sherpa-cache}"
ARCH="${1:-amd64}"

archive_name() {
    case "$1" in
        amd64)      echo "sherpa-onnx-v${VERSION}-linux-x64-static-lib.tar.bz2" ;;
        arm64)      echo "sherpa-onnx-v${VERSION}-linux-aarch64-static-lib.tar.bz2" ;;
        osx-arm64)  echo "sherpa-onnx-v${VERSION}-osx-arm64-static-lib.tar.bz2" ;;
        osx-x86_64) echo "sherpa-onnx-v${VERSION}-osx-x86_64-static-lib.tar.bz2" ;;
        *) echo "❌ 未知架构: $1（支持 amd64 / arm64 / osx-arm64 / osx-x86_64 / all）" >&2; exit 1 ;;
    esac
}

fetch_one() {
    local arch="$1"
    local name; name="$(archive_name "$arch")"
    local url="${PROXY}/https://github.com/k2-fsa/sherpa-onnx/releases/download/v${VERSION}/${name}"
    local dest="$OUT/$name"

    mkdir -p "$OUT"
    if [ -f "$dest" ]; then
        echo "✅ 已存在，跳过: $name ($(du -h "$dest" | cut -f1))"
        return
    fi

    echo "⬇️  下载 $name"
    echo "   $url"
    if curl -fL --retry 3 --connect-timeout 30 -o "$dest.partial" "$url"; then
        mv "$dest.partial" "$dest"
        echo "✅ 完成: $name ($(du -h "$dest" | cut -f1))"
    else
        rm -f "$dest.partial"
        echo "❌ 下载失败: $name" >&2
        echo "   检查网络 / 换镜像: SHERPA_PROXY=https://<其他镜像> bash $0 $arch" >&2
        exit 1
    fi
}

case "$ARCH" in
    all) fetch_one amd64; fetch_one arm64 ;;
    amd64|arm64|osx-arm64|osx-x86_64) fetch_one "$ARCH" ;;
    *) archive_name "$ARCH" >&2; exit 1 ;;
esac

echo
echo "📦 sherpa-cache 内容:"
ls -lh "$OUT"/*.tar.bz2 2>/dev/null || echo "   （空）"
