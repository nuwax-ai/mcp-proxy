#!/usr/bin/env bash
# Ubuntu 24.04 装 libssl1.1（voice-cli 兜底检测脚本）
#
# 背景: voice-cli 新架构 reqwest 已用 rustls（Cargo.toml:65-73），产物通常不依赖任何 libssl.so。
#   仅 cuda 编译 + 特定链接场景可能命中 libssl1.1，此时运行报:
#   error while loading shared libraries: libssl.so.1.1
# 本脚本先 ldd 检测，命中才从阿里云 Ubuntu 镜像下载 libssl1.1 deb 安装（与系统 OpenSSL 3 共存）。
# rustls 版（默认）会自动 exit 0，无副作用。
#
# 用法: bash install-libssl1.1-ubuntu2404.sh [voice-cli 二进制路径]
set -euo pipefail

BIN="${1:-./voice-cli}"

echo "=== 1) 检查 voice-cli 是否需要 libssl1.1 ==="
if ldd "$BIN" 2>/dev/null | grep -q "libssl.so.1.1"; then
    echo "  ✅ 检测到 voice-cli 链接 libssl1.1，需要安装"
else
    echo "  voice-cli 不依赖 libssl1.1（rustls/CPU/metal 版），无需安装"
    exit 0
fi

if [ -f /usr/lib/x86_64-linux-gnu/libssl.so.1.1 ]; then
    echo "  libssl1.1 已存在，跳过"
    exit 0
fi

echo
echo "=== 2) 从阿里云 Ubuntu 镜像下载 libssl1.1 ==="
URL_BASE=https://mirrors.aliyun.com/ubuntu/pool/main/o/openssl
DEB=$(curl -s "$URL_BASE/" | grep -oE "libssl1.1_[^\"]*_amd64\.deb" | sort -u | tail -1)
[ -n "$DEB" ] || { echo "❌ 未找到 libssl1.1 deb（检查网络）"; exit 1; }
echo "  找到: $DEB"

cd /tmp
wget -q "$URL_BASE/$DEB" -O libssl1.1.deb
echo "  下载完成: $(ls -l libssl1.1.deb | awk '{print $5}') bytes"

echo
echo "=== 3) 安装（与系统 OpenSSL 3 并存）==="
sudo dpkg -i libssl1.1.deb
rm -f libssl1.1.deb

echo
echo "=== 4) 验证 ==="
ls -l /usr/lib/x86_64-linux-gnu/libssl.so.1.1 /usr/lib/x86_64-linux-gnu/libcrypto.so.1.1
ldd "$BIN" 2>/dev/null | grep -E "libssl|libcrypto" || echo "  voice-cli 缺库检查完成"

echo
echo "✅ libssl1.1 安装完成"
