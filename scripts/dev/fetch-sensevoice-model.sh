#!/usr/bin/env bash
# 拉取 SenseVoice ONNX 模型（sherpa-onnx 布局）到 ./models/sensevoice/。
#
# transcribe-rs SenseVoiceModel::load 需目录含 `model.int8.onnx` + `tokens.txt`（无自动下载）。
# 规范目录名 `sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17`（中英日韩粤，int8 ~900MB）。
# 源：sherpa-onnx github `asr-models` release（github 阻断时走 gh-proxy 镜像，自动探活）。
#
# 用法（在 voice-cli 工作目录下执行）:
#   bash scripts/dev/fetch-sensevoice-model.sh
#   bash scripts/dev/fetch-sensevoice-model.sh /abs/path/to/model_dir   # 自定义输出目录
#
# 环境变量:
#   SV_PROXY  GitHub 镜像（不设则自动探活 ghproxy.net/gh-proxy.com/mirror.ghproxy.com 择优）
set -euo pipefail

MODEL_NAME="sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"
ARCHIVE="${MODEL_NAME}.tar.bz2"
DEST="${1:-./models/sensevoice/${MODEL_NAME}}"
DEST_DIR="$(dirname "$DEST")"
PROXY="${SV_PROXY:-}"   # 手动指定镜像；留空自动探活

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
    echo "❌ 所有镜像不可用。手动指定: SV_PROXY=https://<镜像> bash $0" >&2
    echo "   或从 modelscope 手拉（搜 SenseVoice，sherpa 布局）后放到 ${DEST}" >&2
    exit 1
}

mkdir -p "$DEST_DIR"
if [ -f "${DEST}/model.int8.onnx" ] && [ -f "${DEST}/tokens.txt" ]; then
    echo "✅ 模型已存在，跳过: ${DEST}"
    exit 0
fi

resolve_proxy
URL="${PROXY}/${GH_TARGET}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "⬇️  下载 ${ARCHIVE}"
echo "   ${URL}"
if ! curl -fL --retry 3 --connect-timeout 30 -o "${TMP}/${ARCHIVE}" "$URL"; then
    echo "❌ 下载失败。换镜像: SV_PROXY=https://<其他镜像> bash $0" >&2
    exit 1
fi
echo "✅ 下载完成 ($(du -h "${TMP}/${ARCHIVE}" | cut -f1))"

echo "📦 解压到 ${DEST_DIR}/"
tar xjf "${TMP}/${ARCHIVE}" -C "$DEST_DIR"

# 校验关键文件：tokens.txt 必有；ONNX 接受 model.onnx（sherpa asr-models 发布的默认名）
# 或 model.<quant>.onnx 任一。transcribe-rs 的 quantization 仅做"文件选择"，
# model.onnx 由 Quantization::FP32 直接命中（int8 找不到 model.int8.onnx 时回退 model.onnx + 告警）。
ONNX_FILE=""
for f in "${DEST}/model.onnx" "${DEST}/model.int8.onnx" "${DEST}/model.fp16.onnx" "${DEST}/model.int4.onnx"; do
    [ -f "$f" ] && ONNX_FILE="$f" && break
done
if [ -z "$ONNX_FILE" ] || [ ! -f "${DEST}/tokens.txt" ]; then
    echo "❌ 解压后缺少 model*.onnx / tokens.txt。实际内容:" >&2
    ls -la "${DEST}" 2>/dev/null || ls -la "${DEST_DIR}" >&2
    exit 1
fi

echo
echo "✅ SenseVoice 模型就绪: ${DEST}"
ls -lh "$ONNX_FILE" "${DEST}/tokens.txt"
# sherpa asr-models 发布是 model.onnx（int8 权重，无后缀）→ 用 quantization: fp32 干净命中
ONNX_BASENAME="$(basename "$ONNX_FILE")"
case "$ONNX_BASENAME" in
    model.onnx)        QREC="fp32" ;;
    model.int8.onnx)   QREC="int8" ;;
    model.fp16.onnx)   QREC="fp16" ;;
    model.int4.onnx)   QREC="int4" ;;
    *)                 QREC="fp32" ;;
esac
echo
echo "config.yml 用法（voice-cli 工作目录下）:"
echo "  whisper:"
echo "    engine:"
echo "      backend: sensevoice"
echo "      sensevoice:"
echo "        model_dir: ${DEST}"
echo "        quantization: ${QREC}   # 文件: ${ONNX_BASENAME}"
