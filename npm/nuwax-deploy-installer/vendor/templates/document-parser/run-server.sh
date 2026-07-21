#!/usr/bin/env bash
# LaunchAgent wrapper: load OSS secrets then start document-parser.
set -euo pipefail
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/Library/Frameworks/Python.framework/Versions/Current/bin:${PATH:-}"
# launchd may omit HOME; Python/HuggingFace caches and MPS need a real home
if [[ -z "${HOME:-}" ]]; then
  HOME="$(cd ~ && pwd)"
  export HOME
fi
export TMPDIR="${TMPDIR:-/tmp}"
cd "$(dirname "$0")"
set -a
# shellcheck disable=SC1091
source .document-parser.env
set +a
exec ./document-parser --config ./config.yml server
