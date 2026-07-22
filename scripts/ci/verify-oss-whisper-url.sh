#!/usr/bin/env bash
# Verify a voice-cli Whisper ggml tarball on OSS (HTTP + archive layout).
#
# Usage:
#   bash scripts/ci/verify-oss-whisper-url.sh <URL>
#   bash scripts/ci/verify-oss-whisper-url.sh --all <URL>   # expect all five ggml bins
#
set -euo pipefail

EXPECT_ALL=0
URL=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --all)
      EXPECT_ALL=1
      shift
      ;;
    -h | --help)
      sed -n '2,8p' "$0"
      exit 0
      ;;
    *)
      URL="$1"
      shift
      ;;
  esac
done

[[ -n "$URL" ]] || {
  echo "Usage: $0 [--all] <URL>" >&2
  exit 1
}

log() {
  printf '==> %s\n' "$*"
}

log "HEAD $URL"
curl -fsSI "$URL" | sed -n '1,12p'

TMP="$(mktemp -d "${TMPDIR:-/tmp}/verify-whisper.XXXXXX")"
ARCHIVE="$TMP/archive.tar.gz"
cleanup() {
  rm -rf "$TMP"
}
trap cleanup EXIT

log "download"
curl -fsSL "$URL" -o "$ARCHIVE"
log "size: $(du -h "$ARCHIVE" | awk '{print $1}')"
log "sha256: $(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"

log "list archive (expect top-level models/)"
tar -tzf "$ARCHIVE" | head -20
if ! tar -tzf "$ARCHIVE" | grep -q '^models/'; then
  echo "ERROR: archive must contain top-level models/ directory" >&2
  exit 1
fi
if ! tar -tzf "$ARCHIVE" | grep -q 'models/ggml-large-v3.bin$'; then
  echo "ERROR: archive must contain models/ggml-large-v3.bin" >&2
  exit 1
fi

if [[ "$EXPECT_ALL" -eq 1 ]]; then
  for m in tiny base small medium large-v3; do
    if ! tar -tzf "$ARCHIVE" | grep -q "models/ggml-${m}.bin$"; then
      echo "ERROR: --all pack missing models/ggml-${m}.bin" >&2
      exit 1
    fi
  done
fi

log "extract smoke"
EXTRACT_DIR="$TMP/install"
mkdir -p "$EXTRACT_DIR"
tar -xzf "$ARCHIVE" -C "$EXTRACT_DIR"
[[ -f "$EXTRACT_DIR/models/ggml-large-v3.bin" ]] || {
  echo "ERROR: missing ggml-large-v3.bin after extract" >&2
  exit 1
}

echo
echo "✅ OSS Whisper URL looks OK: $URL"
