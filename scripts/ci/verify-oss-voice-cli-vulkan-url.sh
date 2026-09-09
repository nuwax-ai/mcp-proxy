#!/usr/bin/env bash
# Verify voice-cli Vulkan linux-x64 tarball on OSS (HTTP + flat layout + tier marker).
#
# Usage:
#   bash scripts/ci/verify-oss-voice-cli-vulkan-url.sh <URL>
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
  .voice-cli-vulkan
)

log() {
  printf '==> %s\n' "$*"
}

log "HEAD $URL"
curl -fsSI "$URL" | sed -n '1,12p'

TMP="$(mktemp -d "${TMPDIR:-/tmp}/verify-voice-vulkan.XXXXXX")"
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
  [[ -s "$EXTRACT_DIR/$f" ]] || {
    echo "ERROR: missing or empty $f after extract" >&2
    exit 1
  }
done

# marker 内容带 vulkan 标识（档位判据，内容错=装错包）
grep -q '^vulkan ' "$EXTRACT_DIR/.voice-cli-vulkan" || {
  echo "ERROR: .voice-cli-vulkan marker content is not 'vulkan <version>'" >&2
  exit 1
}

echo
echo "✅ OSS voice-cli Vulkan URL looks OK: $URL"
