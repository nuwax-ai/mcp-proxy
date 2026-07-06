#!/usr/bin/env bash
# 检查 mac 本地开发所需的系统依赖：rust/cargo、ffmpeg、uv
# 缺失时打印安装命令；任一缺失则 exit 1（Fail Fast，避免后续 setup 失败难定位）
set -euo pipefail

missing=0

check() {
    local name="$1" cmd="$2" hint="$3"
    if command -v "$cmd" >/dev/null 2>&1; then
        echo "  ✅ $name: $(command -v "$cmd")"
    else
        echo "  ❌ $name 缺失。安装: $hint"
        missing=1
    fi
}

echo "🔍 检查系统依赖（mac 本地开发）"
check "rust/cargo" cargo "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
check "ffmpeg"     ffmpeg "brew install ffmpeg"
check "uv"         uv     "curl -LsSf https://astral.sh/uv/install.sh | sh"

if [ "$missing" -ne 0 ]; then
    echo
    echo "❌ 有依赖缺失，先装好再 make dev-setup"
    exit 1
fi

echo
echo "✅ 系统依赖齐全"
