#!/usr/bin/env bash
# Build a relocatable document-parser Python venv tarball for macOS Apple Silicon.
#
# Output (default):
#   dist/document-parser/v{VERSION}/venv-macos-arm64-{VERSION}.tar.gz
#
# Upload to OSS (maintainer, manual):
#   oss://nuwa-packages/uploads/document-parser/venv-macos-arm64-{VERSION}.tar.gz
# Public URL (manifest template):
#   https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/venv-macos-arm64-{VERSION}.tar.gz
#
# Usage:
#   bash scripts/ci/pack-document-parser-venv-macos-arm64.sh [VERSION] [OUTPUT_DIR]
#   bash scripts/ci/pack-document-parser-venv-macos-arm64.sh 0.2.1
#   bash scripts/ci/pack-document-parser-venv-macos-arm64.sh --dry-run
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PKG_JSON="$ROOT/npm/nuwax-deploy-installer/package.json"

DRY_RUN=0
SKIP_VERIFY=0
POSITIONAL=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --skip-verify)
      SKIP_VERIFY=1
      shift
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
if [[ -n "${POSITIONAL[1]:-}" ]]; then
  OUTPUT_DIR="${POSITIONAL[1]}"
else
  OUTPUT_DIR="dist/document-parser/v${VERSION}"
fi
ARCHIVE_NAME="venv-macos-arm64-${VERSION}.tar.gz"
OUTPUT_PATH="$ROOT/$OUTPUT_DIR/$ARCHIVE_NAME"

MINERU_VERSION="3.4.4"

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

assert_macos_arm64() {
  [[ "$(uname -s)" == "Darwin" ]] || die "this script must run on macOS (got $(uname -s))"
  [[ "$(uname -m)" == "arm64" ]] || die "this script must run on Apple Silicon (got $(uname -m))"
}

print_plan() {
  log "document-parser prebuilt venv pack (macOS ARM64)"
  echo "  version:     $VERSION"
  echo "  output:      $OUTPUT_PATH"
  echo "  mineru:      $MINERU_VERSION"
  echo "  relocatable: yes (uv venv --relocatable)"
  echo
  echo "OSS upload path (after you upload manually):"
  echo "  oss://nuwa-packages/uploads/document-parser/${ARCHIVE_NAME}"
  echo "Public URL:"
  echo "  https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/${ARCHIVE_NAME}"
  echo
}

verify_venv() {
  local staging="$1"
  local py="$staging/venv/bin/python"
  [[ -x "$py" ]] || die "venv python missing: $py"

  log "verify: mineru"
  "$staging/venv/bin/mineru" --version

  log "verify: torch / MPS"
  "$py" -c "import torch; print('torch', torch.__version__, '| cuda', torch.cuda.is_available(), '| mps', torch.backends.mps.is_available())"

  log "verify: markitdown import"
  "$py" -c "import markitdown; print('markitdown OK')"
}

create_venv() {
  local staging="$1"
  local py="$staging/venv/bin/python"

  log "create relocatable venv (Python 3.12)"
  (
    cd "$staging"
    if [[ -x /opt/homebrew/bin/python3.12 ]]; then
      uv venv --relocatable --python /opt/homebrew/bin/python3.12 venv
    elif [[ -x /usr/local/bin/python3.12 ]]; then
      uv venv --relocatable --python /usr/local/bin/python3.12 venv
    else
      uv venv --relocatable --python 3.12 venv
    fi
  )

  local py_version
  py_version="$("$py" --version | awk '{print $2}')"
  case "$py_version" in
    3.1[0-3]*) ;;
    *) die "Python $py_version does not satisfy mineru (need 3.10–3.13)" ;;
  esac
  echo "  python: $py_version"

  log "install mineru[core]==${MINERU_VERSION}"
  uv pip install "mineru[core]==${MINERU_VERSION}" --python "$py"

  log "install markitdown"
  uv pip install markitdown --python "$py"

  log "pin huggingface-hub<1.0 (mineru dependency conflict)"
  uv pip install "huggingface-hub>=0.34,<1.0" --python "$py"
}

write_metadata() {
  local staging="$1"
  local meta="${OUTPUT_PATH}.meta.json"
  local sha size_bytes mps_ok
  sha="$(shasum -a 256 "$OUTPUT_PATH" | awk '{print $1}')"
  size_bytes="$(stat -f%z "$OUTPUT_PATH")"
  mps_ok="$("$staging/venv/bin/python" -c "import torch; print('true' if torch.backends.mps.is_available() else 'false')")"

  cat >"$meta" <<EOF
{
  "version": "${VERSION}",
  "platform": "darwin-arm64",
  "archive": "${ARCHIVE_NAME}",
  "sha256": "${sha}",
  "size_bytes": ${size_bytes},
  "mineru": "${MINERU_VERSION}",
  "torch_mps_available_at_build": ${mps_ok},
  "oss_object_key": "uploads/document-parser/${ARCHIVE_NAME}",
  "public_url": "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/${ARCHIVE_NAME}"
}
EOF
  log "metadata → $meta"
}

assert_macos_arm64
require_cmd uv
require_cmd node
require_cmd tar
require_cmd shasum

print_plan

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "(dry-run: prerequisites OK, exiting before venv build)"
  exit 0
fi

STAGING="$(mktemp -d "${TMPDIR:-/tmp}/doc-parser-venv-pack.XXXXXX")"
cleanup() {
  rm -rf "$STAGING"
}
trap cleanup EXIT

create_venv "$STAGING"

if [[ "$SKIP_VERIFY" -eq 0 ]]; then
  verify_venv "$STAGING"
else
  log "skip verify (--skip-verify)"
fi

mkdir -p "$ROOT/$OUTPUT_DIR"
log "archive → $OUTPUT_PATH"
# Avoid macOS xattr/resource forks in tarball
COPYFILE_DISABLE=1 tar -czf "$OUTPUT_PATH" -C "$STAGING" venv

log "archive size: $(du -h "$OUTPUT_PATH" | awk '{print $1}')"
write_metadata "$STAGING"

echo
echo "✅ pack complete"
echo "Next:"
echo "  1. Upload: $OUTPUT_PATH"
echo "     → oss://nuwa-packages/uploads/document-parser/${ARCHIVE_NAME}"
echo "  2. Verify: bash scripts/ci/verify-oss-venv-url.sh \\"
echo "       https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/${ARCHIVE_NAME}"
echo "  3. Publish npm: bash scripts/ci/publish-nuwax-deploy-installer.sh ${VERSION}"
