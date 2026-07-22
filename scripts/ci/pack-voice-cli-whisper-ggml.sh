#!/usr/bin/env bash
# Pack Whisper ggml models for voice-cli OSS deployment (macOS / any host with curl).
#
# Default: whisper-ggml-large-v3-{VERSION}.tar.gz  (models/ggml-large-v3.bin only)
# Optional: whisper-ggml-all-{VERSION}.tar.gz       (--all: tiny/base/small/medium/large-v3)
#
# Upload (maintainer, manual):
#   oss://nuwa-packages/uploads/voice-cli/whisper-ggml-large-v3-{VERSION}.tar.gz
# Public URL:
#   https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli/whisper-ggml-large-v3-{VERSION}.tar.gz
#
# Usage:
#   bash scripts/ci/pack-voice-cli-whisper-ggml.sh [VERSION]
#   bash scripts/ci/pack-voice-cli-whisper-ggml.sh --all 0.2.1
#   bash scripts/ci/pack-voice-cli-whisper-ggml.sh --dry-run
#   bash scripts/ci/pack-voice-cli-whisper-ggml.sh --local-dir ./models 0.2.1
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PKG_JSON="$ROOT/npm/nuwax-deploy-installer/package.json"
MODELSCOPE_BASE="https://modelscope.cn/models/cjc1887415157/whisper.cpp/resolve/master"

DRY_RUN=0
PACK_ALL=0
LOCAL_DIR=""
POSITIONAL=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --all)
      PACK_ALL=1
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
      sed -n '2,20p' "$0"
      exit 0
      ;;
    *)
      POSITIONAL+=("$1")
      shift
      ;;
  esac
done

VERSION="${POSITIONAL[0]:-$(node -p "require('$PKG_JSON').version")}"
VERSION="${VERSION%%-*}"
OUTPUT_DIR="dist/voice-cli/v${VERSION}"

if [[ "$PACK_ALL" -eq 1 ]]; then
  ARCHIVE_NAME="whisper-ggml-all-${VERSION}.tar.gz"
  MODELS=(tiny base small medium large-v3)
else
  ARCHIVE_NAME="whisper-ggml-large-v3-${VERSION}.tar.gz"
  MODELS=(large-v3)
fi

OUTPUT_PATH="$ROOT/$OUTPUT_DIR/$ARCHIVE_NAME"
OSS_PREFIX="uploads/voice-cli"
PUBLIC_BASE="https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/${OSS_PREFIX}"

log() {
  printf '==> %s\n' "$*"
}

die() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

print_plan() {
  log "voice-cli Whisper ggml pack"
  echo "  version:   $VERSION"
  echo "  mode:      $([[ $PACK_ALL -eq 1 ]] && echo all || echo large-v3)"
  echo "  models:    ${MODELS[*]}"
  echo "  output:    $OUTPUT_PATH"
  echo
  echo "OSS upload:"
  echo "  oss://nuwa-packages/${OSS_PREFIX}/${ARCHIVE_NAME}"
  echo "Public URL:"
  echo "  ${PUBLIC_BASE}/${ARCHIVE_NAME}"
  echo
}

fetch_or_copy_model() {
  local name="$1"
  local staging_models="$2"
  local dst="$staging_models/ggml-${name}.bin"
  local src_local=""

  if [[ -n "$LOCAL_DIR" ]]; then
    src_local="$LOCAL_DIR/ggml-${name}.bin"
    [[ -f "$src_local" ]] || src_local="$LOCAL_DIR/models/ggml-${name}.bin"
    [[ -f "$src_local" ]] || die "missing local ggml-${name}.bin under $LOCAL_DIR"
    cp "$src_local" "$dst"
    log "copied ggml-${name}.bin from $src_local"
    return
  fi

  local url="${MODELSCOPE_BASE}/ggml-${name}.bin"
  log "download ggml-${name}.bin"
  curl -fL --retry 3 --retry-delay 5 "$url" -o "$dst"
}

write_metadata() {
  local staging="$1"
  local meta="${OUTPUT_PATH}.meta.json"
  local sha size_bytes
  sha="$(shasum -a 256 "$OUTPUT_PATH" | awk '{print $1}')"
  size_bytes="$(stat -f%z "$OUTPUT_PATH" 2>/dev/null || stat -c%s "$OUTPUT_PATH")"

  cat >"$meta" <<EOF
{
  "version": "${VERSION}",
  "platform": "darwin-arm64",
  "archive": "${ARCHIVE_NAME}",
  "pack": "$([[ $PACK_ALL -eq 1 ]] && echo all || echo large-v3)",
  "sha256": "${sha}",
  "size_bytes": ${size_bytes},
  "models": [$(printf '"%s",' "${MODELS[@]}" | sed 's/,$//')],
  "oss_object_key": "${OSS_PREFIX}/${ARCHIVE_NAME}",
  "public_url": "${PUBLIC_BASE}/${ARCHIVE_NAME}"
}
EOF
  log "metadata → $meta"
}

require_cmd curl
require_cmd node
require_cmd tar
require_cmd shasum

print_plan

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "(dry-run: prerequisites OK, exiting before download/pack)"
  exit 0
fi

STAGING="$(mktemp -d "${TMPDIR:-/tmp}/voice-whisper-pack.XXXXXX")"
cleanup() {
  rm -rf "$STAGING"
}
trap cleanup EXIT

mkdir -p "$STAGING/models"
for m in "${MODELS[@]}"; do
  fetch_or_copy_model "$m" "$STAGING/models"
done

[[ -f "$STAGING/models/ggml-large-v3.bin" ]] || die "archive must contain models/ggml-large-v3.bin"

mkdir -p "$ROOT/$OUTPUT_DIR"
log "archive → $OUTPUT_PATH"
COPYFILE_DISABLE=1 tar -czf "$OUTPUT_PATH" -C "$STAGING" models

log "archive size: $(du -h "$OUTPUT_PATH" | awk '{print $1}')"
write_metadata "$STAGING"

echo
echo "✅ pack complete"
echo "Next:"
echo "  1. Upload: $OUTPUT_PATH"
echo "     → oss://nuwa-packages/${OSS_PREFIX}/${ARCHIVE_NAME}"
echo "  2. Verify: bash scripts/ci/verify-oss-whisper-url.sh \\"
echo "       ${PUBLIC_BASE}/${ARCHIVE_NAME}"
