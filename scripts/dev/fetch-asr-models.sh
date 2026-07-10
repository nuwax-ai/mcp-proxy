#!/usr/bin/env bash
# 拉取 sherpa-onnx ASR 模型（FireRedASR2-AED / Fun-ASR-Nano / Qwen3-ASR）到 ./models/<kind>/。
#
# 三模型均走 sherpa-onnx `OfflineRecognizer`（与 TTS 同库），Mac 预编译 lib 含 CoreML EP → 可 GPU。
# 目录布局（sherpa-onnx asr-models release 解压后）：
#   fireredasr2: encoder.int8.onnx + decoder.int8.onnx + tokens.txt
#   funasrnano : encoder_adaptor.int8.onnx + llm.int8.onnx + embedding.int8.onnx + Qwen3-0.6B/
#   qwen3asr   : conv_frontend.onnx + encoder.int8.onnx + decoder.int8.onnx + tokenizer/
# 源：github k2-fsa/sherpa-onnx asr-models release（github 阻断时走 gh-proxy 镜像，自动探活）。
#
# 用法（在 voice-cli 工作目录下执行）:
#   bash scripts/dev/fetch-asr-models.sh fireredasr2
#   bash scripts/dev/fetch-asr-models.sh funasrnano
#   bash scripts/dev/fetch-asr-models.sh qwen3asr
#   bash scripts/dev/fetch-asr-models.sh fireredasr2 /abs/path/to/model_dir   # 自定义输出目录
#
# 环境变量:
#   ASR_PROXY  GitHub 镜像（不设则自动探活 ghproxy.net/gh-proxy.com/mirror.ghproxy.com 择优）
set -euo pipefail

KIND="${1:-}"
if [[ -z "$KIND" ]]; then
    echo "用法: bash $0 <fireredasr2|funasrnano|qwen3asr> [自定义输出目录]" >&2
    exit 1
fi

case "$KIND" in
    fireredasr2)
        MODEL_NAME="sherpa-onnx-fire-red-asr2-zh_en-int8-2026-02-26"
        SUBDIR="fireredasr2"
        ;;
    funasrnano)
        MODEL_NAME="sherpa-onnx-funasr-nano-int8-2025-12-30"
        SUBDIR="funasrnano"
        ;;
    qwen3asr)
        MODEL_NAME="sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25"
        SUBDIR="qwen3asr"
        ;;
    *)
        echo "❌ 未知 kind: $KIND（可选: fireredasr2|funasrnano|qwen3asr）" >&2
        exit 1
        ;;
esac

ARCHIVE="${MODEL_NAME}.tar.bz2"
DEST="${2:-./models/${SUBDIR}/${MODEL_NAME}}"
DEST_DIR="$(dirname "$DEST")"
PROXY="${ASR_PROXY:-}"   # 手动指定镜像；留空自动探活

GH_MIRRORS=(
    "https://ghproxy.net"
    "https://gh-proxy.com"
    "https://mirror.ghproxy.com"
)
GH_TARGET="https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/${ARCHIVE}"

# 选可用 GitHub 镜像（PROXY 已手动指定则跳过）。
resolve_proxy() {
    [ -n "$PROXY" ] && return 0
    echo "🔍 探活 GitHub 镜像（${ARCHIVE}）..."
    for m in "${GH_MIRRORS[@]}"; do
        local sz
        sz=$(curl -sL -m 5 -r 0-2000000 -o /dev/null -w '%{size_download}' "${m}/${GH_TARGET}" 2>/dev/null) || true
        sz="${sz%.*}"; case "$sz" in ''|*[!0-9]*) sz=0 ;; esac
        if [ "$sz" -gt 1000000 ]; then
            PROXY="$m"; echo "  ✅ ${m}"; return 0
        fi
        echo "  ⚠️  ${m}（5s 内 $((sz/1024)) KB，跳过）"
    done
    echo "❌ 所有镜像不可用。手动指定: ASR_PROXY=https://<镜像> bash $0 $KIND" >&2
    echo "   或从 modelscope 搜索对应模型（sherpa 布局）后放到 ${DEST}" >&2
    exit 1
}

mkdir -p "$DEST_DIR"
if [ -d "${DEST}" ] && [ -n "$(ls -A "${DEST}" 2>/dev/null)" ]; then
    echo "✅ 模型目录已存在，跳过: ${DEST}"
    exit 0
fi

resolve_proxy
URL="${PROXY}/${GH_TARGET}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "⬇️  下载 ${ARCHIVE}"
echo "   ${URL}"
if ! curl -fL --retry 3 --connect-timeout 30 -o "${TMP}/${ARCHIVE}" "$URL"; then
    echo "❌ 下载失败。换镜像: ASR_PROXY=https://<其他镜像> bash $0 $KIND" >&2
    exit 1
fi
echo "✅ 下载完成 ($(du -h "${TMP}/${ARCHIVE}" | cut -f1))"

echo "📦 解压到 ${DEST_DIR}/"
tar xjf "${TMP}/${ARCHIVE}" -C "$DEST_DIR"

echo
echo "✅ ${KIND} 模型就绪: ${DEST}"
ls -lh "${DEST}" 2>/dev/null | head -20
echo
echo "config.yml 用法（voice-cli 工作目录下）:"
echo "  whisper:"
echo "    engine:"
echo "      backend: ${KIND}        # sherpa-onnx 批量引擎；流式仍需 whisper"
echo "      sherpa:"
echo "        provider: coreml      # Mac GPU/ANE（留空 = CPU）；Linux 用 cuda"
echo "        ${SUBDIR}:"
echo "          model_dir: ${DEST}"
