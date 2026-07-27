#!/usr/bin/env bash
# Verify a prebuilt document-parser venv tarball on OSS (HTTP + archive layout).
#
# Usage:
#   bash scripts/ci/verify-oss-venv-url.sh <URL>
#   bash scripts/ci/verify-oss-venv-url.sh --extract <URL>   # also smoke-test python imports
#
set -euo pipefail

EXTRACT=0
URL=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --extract)
      EXTRACT=1
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
  echo "Usage: $0 [--extract] <URL>" >&2
  exit 1
}

log() {
  printf '==> %s\n' "$*"
}

log "HEAD $URL"
curl -fsSI "$URL" | sed -n '1,12p'

TMP="$(mktemp -d "${TMPDIR:-/tmp}/verify-venv.XXXXXX")"
ARCHIVE="$TMP/archive.tar.gz"
cleanup() {
  rm -rf "$TMP"
}
trap cleanup EXIT

log "download"
curl -fsSL "$URL" -o "$ARCHIVE"
log "size: $(du -h "$ARCHIVE" | awk '{print $1}')"
log "sha256: $(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"

log "list archive (expect top-level venv/)"
tar -tzf "$ARCHIVE" | head -20
if ! tar -tzf "$ARCHIVE" | grep -q '^venv/'; then
  echo "ERROR: archive must contain top-level venv/ directory" >&2
  exit 1
fi

if [[ "$EXTRACT" -eq 1 ]]; then
  log "extract + import smoke test"
  EXTRACT_DIR="$TMP/install"
  mkdir -p "$EXTRACT_DIR"
  tar -xzf "$ARCHIVE" -C "$EXTRACT_DIR"
  PY="$EXTRACT_DIR/venv/bin/python"
  [[ -x "$PY" ]] || {
    echo "ERROR: missing $PY after extract" >&2
    exit 1
  }
  "$EXTRACT_DIR/venv/bin/mineru" --version
  "$PY" -c "import torch, markitdown; print('torch', torch.__version__, '| mps', torch.backends.mps.is_available())"
fi

echo
echo "✅ OSS venv URL looks OK: $URL"
