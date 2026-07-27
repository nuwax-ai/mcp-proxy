#!/usr/bin/env bash
# Verify voice-cli CUDA linux-x64 tarball on OSS (HTTP + flat layout).
#
# Usage:
#   bash scripts/ci/verify-oss-voice-cli-cuda-url.sh <URL>
#
set -euo pipefail

URL="${1:-}"
[[ -n "$URL" ]] || {
  echo "Usage: $0 <URL>" >&2
  exit 1
}

REQUIRED=(
  voice-cli
  libsherpa-onnx-c-api.so
  libonnxruntime.so
  libonnxruntime_providers_cuda.so
  libonnxruntime_providers_shared.so
)

log() {
  printf '==> %s\n' "$*"
}

log "HEAD $URL"
curl -fsSI "$URL" | sed -n '1,12p'

TMP="$(mktemp -d "${TMPDIR:-/tmp}/verify-voice-cuda.XXXXXX")"
ARCHIVE="$TMP/archive.tar.gz"
cleanup() {
  rm -rf "$TMP"
}
trap cleanup EXIT

log "download"
curl -fsSL "$URL" -o "$ARCHIVE"
log "size: $(du -h "$ARCHIVE" | awk '{print $1}')"
log "sha256: $(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"

for f in "${REQUIRED[@]}"; do
  if ! tar -tzf "$ARCHIVE" | grep -qx "$f"; then
    echo "ERROR: archive must contain top-level $f" >&2
    exit 1
  fi
done

EXTRACT_DIR="$TMP/install"
mkdir -p "$EXTRACT_DIR"
tar -xzf "$ARCHIVE" -C "$EXTRACT_DIR"
for f in "${REQUIRED[@]}"; do
  [[ -f "$EXTRACT_DIR/$f" ]] || {
    echo "ERROR: missing $f after extract" >&2
    exit 1
  }
done

echo
echo "✅ OSS voice-cli CUDA URL looks OK: $URL"
