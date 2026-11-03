#!/usr/bin/env bash
# Pack SenseVoice STT models for voice-cli OSS deployment (platform-independent).
#
# 产物: sensevoice-models.tar.gz（**文件名不带版本**——模型版本独立于包版本，
#       与 mineruModels 同款处理，避免每次发版重传）
# 布局: models/sensevoice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/
#         model.int8.onnx (228M, 默认档位) + tokens.txt
#         [--with-fp32] model.onnx (894M)
#       → 顶层是 models/（与 whisper-ggml 包同款约定），安装器解压到 install_dir 即得
#         install_dir/models/sensevoice/<dir>/
#
# Upload (maintainer, manual):
#   oss://nuwa-packages/uploads/voice-cli/models/sensevoice-models.tar.gz
# Public URL:
#   https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli/models/sensevoice-models.tar.gz
#
# Usage:
#   bash scripts/ci/pack-voice-cli-sensevoice-models.sh [--dry-run]
#   bash scripts/ci/pack-voice-cli-sensevoice-models.sh --local-dir /path/to/models/sensevoice
#   bash scripts/ci/pack-voice-cli-sensevoice-models.sh --with-fp32
#
# 模型源: sherpa-onnx github `asr-models` release（github 阻断时走 gh-proxy 镜像，自动探活），
#         与 scripts/dev/fetch-sensevoice-model.sh 同源；--local-dir 可复用本地已拉好的目录。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MODEL_NAME="sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"
ARCHIVE_NAME="sensevoice-models.tar.gz"
OUTPUT_DIR="dist/voice-cli/models"
OSS_PREFIX="uploads/voice-cli/models"
PUBLIC_BASE="https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/${OSS_PREFIX}"

DRY_RUN=0
WITH_FP32=0
LOCAL_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --with-fp32)
      WITH_FP32=1
      shift
      ;;
    --local-dir)
      LOCAL_DIR="${2:-}"
      [[ -n "$LOCAL_DIR" ]] || {
        echo "ERROR: --local-dir requires a path" >&2
        exit 1
      }
      shift 2
      ;;
    -h | --help)
      sed -n '2,22p' "$0"
      exit 0
      ;;
    *)
      echo "ERROR: unknown arg: $1" >&2
      exit 1
      ;;
  esac
done

OUTPUT_PATH="$ROOT/$OUTPUT_DIR/$ARCHIVE_NAME"

log() { printf '==> %s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

print_plan() {
  log "voice-cli SenseVoice models pack"
  echo "  model dir: $MODEL_NAME"
  echo "  files:     model.int8.onnx (228M) + tokens.txt$([[ $WITH_FP32 -eq 1 ]] && echo ' + model.onnx (894M)')"
  echo "  output:    $OUTPUT_PATH"
  echo
  echo "OSS upload:"
  echo "  oss://nuwa-packages/${OSS_PREFIX}/${ARCHIVE_NAME}"
  echo "Public URL:"
  echo "  ${PUBLIC_BASE}/${ARCHIVE_NAME}"
  echo
  echo "manifest.json (npm/deploy-installer/vendor/templates/manifest.json):"
  echo "  \"sensevoiceModels\": {"
  echo "    \"darwin-arm64\": \"${PUBLIC_BASE}/${ARCHIVE_NAME}\","
  echo "    \"darwin-x64\":   \"${PUBLIC_BASE}/${ARCHIVE_NAME}\","
  echo "    \"linux-x64\":    \"${PUBLIC_BASE}/${ARCHIVE_NAME}\","
  echo "    \"linux-arm64\":  \"${PUBLIC_BASE}/${ARCHIVE_NAME}\","
  echo "    \"windows-x64\":  \"${PUBLIC_BASE}/${ARCHIVE_NAME}\""
  echo "  }"
  echo
}

STAGING="$(mktemp -d "${TMPDIR:-/tmp}/sv-models-pack.XXXXXX")"
trap 'rm -rf "$STAGING"' EXIT

resolve_source_dir() {
  # 1) 本地目录优先（复用已拉好的模型，免 1GB 下载）
  if [[ -n "$LOCAL_DIR" ]]; then
    local d="$LOCAL_DIR"
    [[ -d "$d/$MODEL_NAME" ]] && d="$d/$MODEL_NAME"
    [[ -f "$d/tokens.txt" ]] || die "missing tokens.txt under $d"
    [[ -f "$d/model.int8.onnx" || -f "$d/model.onnx" ]] || die "missing model*.onnx under $d"
    echo "$d"
    return
  fi

  # 2) gh-proxy 镜像探活 + 下载 sherpa-onnx asr-models release
  local gh_target="https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/${MODEL_NAME}.tar.bz2"
  local mirrors=("https://ghproxy.net" "https://gh-proxy.com" "https://mirror.ghproxy.com")
  local proxy="" sz
  for m in "${mirrors[@]}"; do
    sz=$(curl -sL -m 5 -r 0-2000000 -o /dev/null -w '%{size_download}' "${m}/${gh_target}" 2>/dev/null || true)
    sz="${sz%.*}"; case "$sz" in '' | *[!0-9]*) sz=0 ;; esac
    if [[ "$sz" -gt 1000000 ]]; then
      proxy="$m"
      log "using GitHub mirror: $m"
      break
    fi
  done
  [[ -n "$proxy" ]] || die "all GitHub mirrors unreachable — use --local-dir"

  local tar_bz2="$STAGING/${MODEL_NAME}.tar.bz2"
  log "download ${MODEL_NAME}.tar.bz2 (~1GB)"
  curl -fL --retry 3 --connect-timeout 30 -o "$tar_bz2" "${proxy}/${gh_target}"
  tar xjf "$tar_bz2" -C "$STAGING"
  echo "$STAGING/$MODEL_NAME"
}

main() {
  print_plan
  if [[ "$DRY_RUN" -eq 1 ]]; then
    log "dry-run: nothing downloaded or written"
    return 0
  fi

  require_cmd() { command -v "$1" >/dev/null 2>&1 || die "missing command: $1"; }
  require_cmd curl
  require_cmd tar

  local src
  src="$(resolve_source_dir)"
  local out_dir="$STAGING/pack/models/sensevoice/$MODEL_NAME"
  mkdir -p "$out_dir"

  # tokens.txt 必带（sherpa C 端强制校验全局 tokens 字段，漏了 create 直接失败）
  cp "$src/tokens.txt" "$out_dir/"
  # int8 默认档位
  if [[ -f "$src/model.int8.onnx" ]]; then
    cp "$src/model.int8.onnx" "$out_dir/"
  else
    log "WARN: source has no model.int8.onnx — packing model.onnx as the only weights"
  fi
  # fp32 可选（voice-cli 的 quantization: fp32 档；默认不打，省 894MB）
  if [[ "$WITH_FP32" -eq 1 && -f "$src/model.onnx" ]]; then
    cp "$src/model.onnx" "$out_dir/"
  fi

  [[ -s "$out_dir/tokens.txt" ]] || die "tokens.txt empty after copy"
  [[ -s "$out_dir/model.int8.onnx" || -s "$out_dir/model.onnx" ]] || die "no onnx weights after copy"

  mkdir -p "$ROOT/$OUTPUT_DIR"
  # 用 gzip -n 保证可复现（不带 mtime/文件名）
  tar -czf "$OUTPUT_PATH" -C "$STAGING/pack" models
  log "packed: $OUTPUT_PATH ($(du -h "$OUTPUT_PATH" | cut -f1))"

  # 元数据（sha256 + 体积），与 whisper-ggml 包同款，便于发布校验
  local meta="${OUTPUT_PATH}.meta.json"
  local sha size_bytes
  sha="$(shasum -a 256 "$OUTPUT_PATH" | awk '{print $1}')"
  size_bytes="$(stat -f%z "$OUTPUT_PATH" 2>/dev/null || stat -c%s "$OUTPUT_PATH")"
  cat >"$meta" <<EOF
{
  "archive": "${ARCHIVE_NAME}",
  "platform": "any",
  "model_dir": "${MODEL_NAME}",
  "sha256": "${sha}",
  "size_bytes": ${size_bytes},
  "files": ["model.int8.onnx", "tokens.txt"$([[ $WITH_FP32 -eq 1 ]] && echo ', "model.onnx"')],
  "oss_object_key": "${OSS_PREFIX}/${ARCHIVE_NAME}",
  "public_url": "${PUBLIC_BASE}/${ARCHIVE_NAME}"
}
EOF
  log "meta: $meta"

  echo
  echo "next:"
  echo "  1) upload to OSS:  ossutil cp $OUTPUT_PATH oss://nuwa-packages/${OSS_PREFIX}/${ARCHIVE_NAME}"
  echo "  2) verify:         bash scripts/ci/verify-oss-sensevoice-url.sh ${PUBLIC_BASE}/${ARCHIVE_NAME}"
  echo "  3) add manifest key sensevoiceModels (see plan above) in"
  echo "     npm/deploy-installer/vendor/templates/manifest.json"
  echo "  4) install:        deploy-installer voice-cli install --sensevoice"
}

main "$@"
