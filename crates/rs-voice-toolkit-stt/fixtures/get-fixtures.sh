#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

MODELS_DIR="${SCRIPT_DIR}/models"
AUDIO_DIR="${SCRIPT_DIR}/audio"

MODEL_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin?download=true"
MODEL_OUT="${MODELS_DIR}/ggml-tiny.bin"

SAMPLE_WAV_URL="https://raw.githubusercontent.com/ggerganov/whisper.cpp/master/samples/jfk.wav"
SAMPLE_WAV_OUT="${AUDIO_DIR}/jfk.wav"

mkdir -p "${MODELS_DIR}" "${AUDIO_DIR}"

download() {
  local url="$1"
  local out="$2"
  if command -v curl >/dev/null 2>&1; then
    echo "Downloading ${url} -> ${out}"
    curl -L --fail --retry 3 --connect-timeout 10 -o "$out" "$url"
  elif command -v wget >/dev/null 2>&1; then
    echo "Downloading ${url} -> ${out}"
    wget -O "$out" "$url"
  else
    echo "Error: need curl or wget to download files" >&2
    exit 1
  fi
}

if [ ! -f "${MODEL_OUT}" ]; then
  download "${MODEL_URL}" "${MODEL_OUT}"
else
  echo "Model already exists: ${MODEL_OUT}"
fi

if [ ! -f "${SAMPLE_WAV_OUT}" ]; then
  download "${SAMPLE_WAV_URL}" "${SAMPLE_WAV_OUT}"
else
  echo "Sample already exists: ${SAMPLE_WAV_OUT}"
fi

echo
echo "Fixtures ready:"
echo "  Model : ${MODEL_OUT}"
echo "  Audio : ${SAMPLE_WAV_OUT}"
echo
echo "Run integration test (自动跳过若 fixtures 缺失):"
echo "  cargo test -p rs-voice-toolkit-stt --lib -- --nocapture"
echo

