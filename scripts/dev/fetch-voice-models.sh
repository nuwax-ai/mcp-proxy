#!/usr/bin/env bash
# 拉取 voice-cli 模型：STT whisper ggml（modelscope）+ TTS kokoro（gh-proxy）
#
# 背景：HuggingFace 阻断，故 STT 走 modelscope 镜像、TTS 走 gh-proxy。
#   模型放到 crates/voice-cli/models/（对齐 config.yml 的 whisper.models_dir / tts.engine.models_dir）。
#
# 用法:
#   bash scripts/dev/fetch-voice-models.sh                # STT 用 config.yml 的 default_model，含 TTS
#   bash scripts/dev/fetch-voice-models.sh large-v3       # 显式指定 STT 模型（~3GB）
#   bash scripts/dev/fetch-voice-models.sh base           # base 141MB，快速冒烟
#   SKIP_TTS=1 bash scripts/dev/fetch-voice-models.sh     # 只拉 STT
#
# 可用环境变量覆盖:
#   MODELSCOPE  modelscope whisper 仓库（默认 cjc1887415157/whisper.cpp）
#   PROXY       gh-proxy 镜像（默认 https://gh-proxy.com）
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VOICE_DIR="$SCRIPT_DIR/../../crates/voice-cli"
MODELS_DIR="$VOICE_DIR/models"
TTS_DIR="$MODELS_DIR/tts"

MODELSCOPE="${MODELSCOPE:-https://modelscope.cn/models/cjc1887415157/whisper.cpp/resolve/master}"
PROXY="${PROXY:-https://gh-proxy.com}"

# 默认从 config.yml 读 default_model（保证 setup 拉的模型 = run 时要用的模型）
config_default() {
    grep -E '^\s*default_model:' "$VOICE_DIR/config.yml" | head -1 | awk '{print $2}' | tr -d '"'
}
STT_MODEL="${1:-$(config_default)}"
[ -n "$STT_MODEL" ] || { echo "❌ 无法从 config.yml 读 default_model，请显式传参: bash $0 base"; exit 1; }

mkdir -p "$MODELS_DIR" "$TTS_DIR"

echo "=== 1) STT whisper ggml ($STT_MODEL) ← modelscope ==="
STT_FILE="$MODELS_DIR/ggml-$STT_MODEL.bin"
if [ -f "$STT_FILE" ]; then
    echo "  ✅ 已存在: ggml-$STT_MODEL.bin ($(du -h "$STT_FILE" | cut -f1))"
else
    url="$MODELSCOPE/ggml-$STT_MODEL.bin"
    echo "  ⬇️  $url"
    if curl -fL --retry 3 --connect-timeout 30 -o "$STT_FILE.partial" "$url"; then
        mv "$STT_FILE.partial" "$STT_FILE"
        echo "  ✅ 完成: $(du -h "$STT_FILE" | cut -f1)"
    else
        rm -f "$STT_FILE.partial"
        echo "  ❌ STT 下载失败: $STT_MODEL" >&2
        echo "     modelscope 仓库可能不含该模型，试小模型: bash $0 base  (或 tiny/small/medium)" >&2
        echo "     并把 config.yml 的 whisper.default_model 改成对应值" >&2
        exit 1
    fi
fi

if [ "${SKIP_TTS:-0}" = "1" ]; then
    echo "=== 2) TTS kokoro: 跳过（SKIP_TTS=1）==="
    echo
    echo "✅ STT 就绪: $STT_FILE"
    exit 0
fi

echo "=== 2) TTS kokoro-multi-lang-v1_0 ← gh-proxy ==="
KOKORO_DIR="$TTS_DIR/kokoro-multi-lang-v1_0"
if [ -f "$KOKORO_DIR/model.onnx" ]; then
    echo "  ✅ 已存在: $KOKORO_DIR"
else
    url="$PROXY/https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_0.tar.bz2"
    echo "  ⬇️  $url"
    tar_tmp="$TTS_DIR/kokoro.tar.bz2"
    if curl -fSL --retry 3 --connect-timeout 30 -o "$tar_tmp" "$url"; then
        tar xjf "$tar_tmp" -C "$TTS_DIR"
        rm -f "$tar_tmp"
        echo "  ✅ 完成: $KOKORO_DIR"
    else
        rm -f "$tar_tmp"
        echo "  ⚠️  kokoro 下载失败（TTS 可选，不影响 STT）" >&2
        echo "     不用 TTS 的话，确认 config.yml tts.enabled: false 即可" >&2
        exit 1
    fi
fi

echo
echo "✅ voice-cli 模型就绪: $MODELS_DIR"
echo "   STT:  ggml-$STT_MODEL.bin"
echo "   TTS:  $KOKORO_DIR"
