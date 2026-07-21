#!/usr/bin/env bash
# Assemble + smoke test + npm pack for nuwax-deploy-installer (local maintainer flow).
#
# Usage:
#   bash scripts/ci/publish-nuwax-deploy-installer.sh [VERSION] [--publish]
#
# Optional env:
#   NPM_TOKEN   required when passing --publish
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PKG_JSON="$ROOT/npm/nuwax-deploy-installer/package.json"
DO_PUBLISH=0
POSITIONAL=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --publish)
      DO_PUBLISH=1
      shift
      ;;
    -h | --help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *)
      POSITIONAL+=("$1")
      shift
      ;;
  esac
done

VERSION="${POSITIONAL[0]:-$(node -p "require('$PKG_JSON').version")}"
TARGET="${POSITIONAL[1]:-aarch64-apple-darwin}"

log() {
  printf '==> %s\n' "$*"
}

log "publish prep for nuwax-deploy-installer v${VERSION}"

bash "$ROOT/scripts/ci/assemble-nuwax-deploy-installer.sh" "$VERSION" "$TARGET"
bash "$ROOT/scripts/ci/smoke-nuwax-deploy-installer.sh" "/tmp/doc-parser-smoke-${VERSION}"

PKG_DIR="$ROOT/npm/nuwax-deploy-installer"

(
  cd "$PKG_DIR"
  npm pack
)

TGZ="$PKG_DIR/nuwax-deploy-installer-${VERSION}.tgz"
log "packed → $TGZ"
log "manifest venv URL template:"
node -e "const m=require('$PKG_DIR/vendor/templates/manifest.json'); console.log(m.optionalAssets.venv['darwin-arm64'].replaceAll('{version}','$VERSION'));"

if [[ "$DO_PUBLISH" -eq 1 ]]; then
  [[ -n "${NPM_TOKEN:-}" ]] || {
    echo "ERROR: set NPM_TOKEN to publish" >&2
    exit 1
  }
  (
    cd "$PKG_DIR"
    npm publish --access public
  )
  log "published nuwax-deploy-installer@${VERSION} to npm"
else
  echo
  echo "Local install test:"
  echo "  npm install -g \"$TGZ\""
  echo "  deploy-installer doctor"
  echo
  echo "Publish to npm:"
  echo "  NPM_TOKEN=*** bash scripts/ci/publish-nuwax-deploy-installer.sh ${VERSION} --publish"
  echo
  echo "Or tag CI release:"
  echo "  git tag deploy-v${VERSION} && git push origin deploy-v${VERSION}"
fi
