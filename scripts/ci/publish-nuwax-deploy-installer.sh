#!/usr/bin/env bash
# Assemble + smoke test + npm pack/publish for nuwax-deploy-installer.
#
# Usage:
#   bash scripts/ci/publish-nuwax-deploy-installer.sh [VERSION] [--publish]
#
# Channel is derived from VERSION:
#   0.2.1-beta.1  →  npm --tag beta
#   0.2.1         →  npm --tag latest
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
      sed -n '2,16p' "$0"
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

if [[ "$VERSION" == *"-beta"* || "$VERSION" == *"-alpha"* || "$VERSION" == *"-rc"* ]]; then
  CHANNEL="beta"
else
  CHANNEL="latest"
fi

log() {
  printf '==> %s\n' "$*"
}

log "publish prep for nuwax-deploy-installer@${VERSION} (channel=${CHANNEL})"

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
node -e "const m=require('$PKG_DIR/vendor/templates/manifest.json'); console.log(m.optionalAssets.venv['darwin-arm64'].replaceAll('{version}','${VERSION%%-*}'));"

if [[ "$DO_PUBLISH" -eq 1 ]]; then
  [[ -n "${NPM_TOKEN:-}" ]] || {
    echo "ERROR: set NPM_TOKEN to publish" >&2
    exit 1
  }
  (
    cd "$PKG_DIR"
    npm publish --access public --tag "$CHANNEL"
    if [[ "$CHANNEL" == "latest" ]]; then
      npm dist-tag add "nuwax-deploy-installer@${VERSION}" latest
    fi
    npm dist-tag ls nuwax-deploy-installer || true
  )
  log "published nuwax-deploy-installer@${VERSION} → @${CHANNEL}"
else
  echo
  echo "Local install test:"
  echo "  npm install -g \"$TGZ\""
  echo "  deploy-installer doctor"
  echo
  echo "Publish to npm (channel=${CHANNEL}):"
  echo "  NPM_TOKEN=*** bash scripts/ci/publish-nuwax-deploy-installer.sh ${VERSION} --publish"
  echo
  echo "Preferred: tag CI release"
  if [[ "$CHANNEL" == "beta" ]]; then
    echo "  git tag -a deploy-v${VERSION} -m \"nuwax-deploy-installer ${VERSION}\" && git push origin deploy-v${VERSION}"
  else
    echo "  # after @beta is verified:"
    echo "  git tag -a deploy-v${VERSION} -m \"nuwax-deploy-installer ${VERSION}\" && git push origin deploy-v${VERSION}"
  fi
fi
