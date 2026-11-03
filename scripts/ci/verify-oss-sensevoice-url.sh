#!/usr/bin/env bash
# Verify the voice-cli SenseVoice model tarball on OSS (HTTP + archive layout + extract smoke).
#
# Usage:
#   bash scripts/ci/verify-oss-sensevoice-url.sh <URL>
#   bash scripts/ci/verify-oss-sensevoice-url.sh --with-fp32 <URL>   # also expect model.onnx
#
# 校验点（与 pack-voice-cli-sensevoice-models.sh 的产物布局一一对应）：
#   顶层 models/ → models/sensevoice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/
#     ├─ model.int8.onnx  (默认档位，~228MB)
#     ├─ tokens.txt       (sherpa C 端强制校验，漏了 create 直接失败)
#     └─ model.onnx       (仅 --with-fp32 包)
set -euo pipefail

EXPECT_FP32=0
URL=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --with-fp32)
      EXPECT_FP32=1
      shift
      ;;
    -h | --help)
      sed -n '2,14p' "$0"
      exit 0
      ;;
    *)
      URL="$1"
      shift
      ;;
  esac
done

[[ -n "$URL" ]] || {
  echo "Usage: $0 [--with-fp32] <URL>" >&2
  exit 1
}

MODEL_DIR="models/sensevoice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"
MIN_INT8_BYTES=100000000

log() { printf '==> %s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

log "HEAD $URL"
curl -fsSI "$URL" | sed -n '1,12p'

TMP="$(mktemp -d "${TMPDIR:-/tmp}/verify-sensevoice.XXXXXX")"
ARCHIVE="$TMP/archive.tar.gz"
trap 'rm -rf "$TMP"' EXIT

log "download"
curl -fsSL "$URL" -o "$ARCHIVE"
log "size: $(du -h "$ARCHIVE" | awk '{print $1}')"
log "sha256: $(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"

log "list archive (expect top-level models/)"
tar -tzf "$ARCHIVE" | head -20
tar -tzf "$ARCHIVE" | grep -q '^models/' || die "archive must contain top-level models/ directory"
tar -tzf "$ARCHIVE" | grep -q "^${MODEL_DIR}/tokens.txt$" || die "missing ${MODEL_DIR}/tokens.txt"
tar -tzf "$ARCHIVE" | grep -q "^${MODEL_DIR}/model.int8.onnx$" || die "missing ${MODEL_DIR}/model.int8.onnx"
if [[ "$EXPECT_FP32" -eq 1 ]]; then
  tar -tzf "$ARCHIVE" | grep -q "^${MODEL_DIR}/model.onnx$" || die "--with-fp32: missing ${MODEL_DIR}/model.onnx"
fi

log "extract smoke (extract into install_dir → install_dir/models/...)"
EXTRACT_DIR="$TMP/install"
mkdir -p "$EXTRACT_DIR"
tar -xzf "$ARCHIVE" -C "$EXTRACT_DIR"
[[ -s "$EXTRACT_DIR/${MODEL_DIR}/tokens.txt" ]] || die "tokens.txt empty after extract"
int8_size=$(stat -f%z "$EXTRACT_DIR/${MODEL_DIR}/model.int8.onnx" 2>/dev/null || stat -c%s "$EXTRACT_DIR/${MODEL_DIR}/model.int8.onnx")
[[ "$int8_size" -ge "$MIN_INT8_BYTES" ]] || die "model.int8.onnx too small (${int8_size} bytes < ${MIN_INT8_BYTES}) — truncated download?"

echo
echo "✅ OSS SenseVoice URL looks OK: $URL"
echo "   install:  deploy-installer voice-cli install --sensevoice"
echo "   config:   whisper.engine.backend: sensevoice"
