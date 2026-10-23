#!/usr/bin/env bash
# 预下载 sherpa-onnx 预编译 C 库到 docker/sherpa-cache/（Dockerfile.voice-cli 编译期注入用）。
#
# 背景：sherpa-onnx-sys 的 build.rs 默认联网下载预编译 C 库 tar.bz2，
#   github 阻断时容器内编译会卡死。本脚本走 gh-proxy 镜像预下载到本地，
#   Dockerfile 通过 ENV SHERPA_ONNX_ARCHIVE_DIR=/app/docker/sherpa-cache 让 build.rs
#   命中本地 tar、跳过联网（详见 sherpa-onnx-sys build.rs 的 archive 命名表）。
#
# 用法:
#   bash docker/fetch-sherpa.sh                      # 默认 amd64 + shared（voice-cli 各平台均用 shared）
#   bash docker/fetch-sherpa.sh amd64                # 同上，显式架构
#   bash docker/fetch-sherpa.sh arm64 static         # 架构 + 变体（默认 shared）
#   bash docker/fetch-sherpa.sh all                  # linux 双架构（amd64 + arm64）都下
#   bash docker/fetch-sherpa.sh osx-arm64            # mac 本地开发（配 OUT_DIR，见下）
#   bash docker/fetch-sherpa.sh win-x64              # Windows 交叉检查/CI 预缓存
#
# 变体说明（与 sherpa-onnx-sys build.rs 的下载命名一致）:
#   shared  默认。voice-cli 各平台均用 shared（macOS shared 启用 CoreML GPU/ANE；
#           Linux CUDA 预编译包也是 shared）。缺点：分发时需带伴生 .so/.dll
#           （deploy-installer 已处理伴生库拷贝）。
#   static  体积自包含、免伴生库；macOS static 无 CoreML（CPU 回退），TTS/STT
#           sherpa 引擎不用。一般仅在实验性精简分发时选。
#
# 可用环境变量覆盖:
#   SHERPA_PROXY   GitHub 镜像地址（不设则自动探活 ghproxy.net/gh-proxy.com/mirror.ghproxy.com 择优）
#   SHERPA_VERSION 版本号（默认自动读根 Cargo.lock 的 sherpa-onnx-sys 版本，二者永不脱节）
#   SHERPA_VARIANT 变体（shared|static，默认 shared；与第 2 位置参数等价）
#   OUT_DIR        输出目录（默认 docker/sherpa-cache；mac 本地开发设
#                  ~/.cache/sherpa-onnx-prebuilt，sherpa-onnx-sys build.rs 默认查找路径）
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# 版本默认取 Cargo.lock 的 sherpa-onnx-sys（升级 lock 后脚本无需跟着改）
lock_version() {
    awk '$0 == "name = \"sherpa-onnx-sys\"" { getline; gsub(/version = |"/, ""); print; exit }' \
        "$SCRIPT_DIR/../Cargo.lock"
}
VERSION="${SHERPA_VERSION:-$(lock_version)}"
PROXY="${SHERPA_PROXY:-}"   # SHERPA_PROXY 手动指定；留空则自动探活镜像
OUT="${OUT_DIR:-$SCRIPT_DIR/sherpa-cache}"
ARCH="${1:-amd64}"
VARIANT="${2:-${SHERPA_VARIANT:-shared}}"

# GitHub 镜像列表（单个镜像偶发卡死，按序 5s 探活，首个可用即用）
GH_MIRRORS=(
    "https://ghproxy.net"
    "https://gh-proxy.com"
    "https://mirror.ghproxy.com"
)
_proxy_resolved=0

# 选可用 GitHub 镜像（PROXY 已手动指定则跳过）。$1 = 完整 github URL。
resolve_proxy() {
    [ -n "$PROXY" ] && return 0
    local target="$1"
    local name="${1##*/}"
    echo "🔍 探活 GitHub 镜像（${name}）..."
    for m in "${GH_MIRRORS[@]}"; do
        local sz
        sz=$(curl -sL -m 5 -r 0-2000000 -o /dev/null -w '%{size_download}' "${m}/${target}" 2>/dev/null) || true
        sz="${sz%.*}"; case "$sz" in ''|*[!0-9]*) sz=0 ;; esac
        if [ "$sz" -gt 1000000 ]; then
            PROXY="$m"; echo "  ✅ $m"; return 0
        fi
        echo "  ⚠️  ${m}（5s 内 $((sz/1024)) KB，跳过）"
    done
    echo "❌ 所有镜像不可用。手动指定: SHERPA_PROXY=https://<镜像> bash $0" >&2
    exit 1
}

# 包名与 sherpa-onnx-sys build.rs 的 (LinkMode, os, arch) 下载命名表保持一致；
# 特例：linux aarch64 shared 带 -cpu- 段、win shared 固定 MT-Release 运行时。
archive_name() {
    case "${VARIANT}/${1}" in
        # ---- shared（默认，voice-cli 各平台）----
        shared/amd64)      echo "sherpa-onnx-v${VERSION}-linux-x64-shared-lib.tar.bz2" ;;
        shared/arm64)      echo "sherpa-onnx-v${VERSION}-linux-aarch64-shared-cpu-lib.tar.bz2" ;;
        shared/osx-arm64)  echo "sherpa-onnx-v${VERSION}-osx-arm64-shared-lib.tar.bz2" ;;
        shared/osx-x86_64) echo "sherpa-onnx-v${VERSION}-osx-x64-shared-lib.tar.bz2" ;;
        shared/win-x64)    echo "sherpa-onnx-v${VERSION}-win-x64-shared-MT-Release-lib.tar.bz2" ;;
        shared/win-arm64)  echo "sherpa-onnx-v${VERSION}-win-arm64-shared-MT-Release-lib.tar.bz2" ;;
        # ---- static（自包含，macOS 无 CoreML）----
        static/amd64)      echo "sherpa-onnx-v${VERSION}-linux-x64-static-lib.tar.bz2" ;;
        static/arm64)      echo "sherpa-onnx-v${VERSION}-linux-aarch64-static-lib.tar.bz2" ;;
        static/osx-arm64)  echo "sherpa-onnx-v${VERSION}-osx-arm64-static-lib.tar.bz2" ;;
        static/osx-x86_64) echo "sherpa-onnx-v${VERSION}-osx-x86_64-static-lib.tar.bz2" ;;
        static/win-x64)    echo "sherpa-onnx-v${VERSION}-win-x64-static-MT-Release-lib.tar.bz2" ;;
        static/win-arm64)  echo "sherpa-onnx-v${VERSION}-win-arm64-static-MT-Release-lib.tar.bz2" ;;
        *) echo "❌ 未知 架构/变体: ${1}/${VARIANT}（支持 amd64 / arm64 / osx-arm64 / osx-x86_64 / win-x64 / win-arm64 / all）" >&2; exit 1 ;;
    esac
}

fetch_one() {
    local arch="$1"
    local name; name="$(archive_name "$arch")"
    local dest="$OUT/$name"

    mkdir -p "$OUT"
    if [ -f "$dest" ]; then
        echo "✅ 已存在，跳过: $name ($(du -h "$dest" | cut -f1))"
        return
    fi

    local target="https://github.com/k2-fsa/sherpa-onnx/releases/download/v${VERSION}/${name}"
    if [ "$_proxy_resolved" = "0" ]; then resolve_proxy "$target"; _proxy_resolved=1; fi
    local url="${PROXY}/${target}"

    echo "⬇️  下载 $name"
    echo "   $url"
    if curl -fL --retry 3 --connect-timeout 30 -o "$dest.partial" "$url"; then
        mv "$dest.partial" "$dest"
        echo "✅ 完成: $name ($(du -h "$dest" | cut -f1))"
    else
        rm -f "$dest.partial"
        echo "❌ 下载失败: $name" >&2
        echo "   检查网络 / 换镜像: SHERPA_PROXY=https://<其他镜像> bash $0 $arch $VARIANT" >&2
        exit 1
    fi
}

case "$ARCH" in
    all) fetch_one amd64; fetch_one arm64 ;;
    amd64|arm64|osx-arm64|osx-x86_64|win-x64|win-arm64) fetch_one "$ARCH" ;;
    *) archive_name "$ARCH" >&2; exit 1 ;;
esac

echo
echo "📦 sherpa-cache 内容:"
ls -lh "$OUT"/*.tar.bz2 2>/dev/null || echo "   （空）"
