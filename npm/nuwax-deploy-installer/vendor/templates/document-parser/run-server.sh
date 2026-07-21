#!/usr/bin/env bash
# LaunchAgent wrapper: load OSS secrets then start document-parser.
set -euo pipefail
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/Library/Frameworks/Python.framework/Versions/Current/bin:${PATH:-}"
cd "$(dirname "$0")"
set -a
# shellcheck disable=SC1091
source .document-parser.env
set +a
exec ./document-parser --config ./config.yml server
