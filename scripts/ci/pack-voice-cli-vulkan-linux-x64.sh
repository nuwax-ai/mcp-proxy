#!/usr/bin/env bash
# Pack voice-cli (Vulkan build) + sherpa CPU shared libs for Linux x86_64 OSS deployment.
#
# Input: directory containing (flat):
#   voice-cli(编译机 cargo build --features vulkan 产物，whisper/ggml-vulkan
#   静态链入二进制) libsherpa-onnx-c-api.so libonnxruntime.so
#
# Output:
#   dist/voice-cli/v{VERSION}/voice-cli-vulkan-linux-x64-{VERSION}.tar.gz
#   （内含档位 marker .voice-cli-vulkan——vulkan 二进制与 CPU 版按文件不可
#   区分，marker 是 install/upgrade 的档位唯一判据）
#
# Usage:
#   bash scripts/ci/pack-voice-cli-vulkan-linux-x64.sh [VERSION]
#   bash scripts/ci/pack-voice-cli-vulkan-linux-x64.sh --src dist/voice-cli/v0.2.12/linux-x64-vulkan 0.2.12
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OSS_PREFIX="uploads/voice-cli"
PUBLIC_BASE="https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/${OSS_PREFIX}"
MARKER=".voice-cli-vulkan"

log() {
  printf '==> %s\n' "$*"
}

die() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

SRC_DIR=""
POSITIONAL=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --src)
      SRC_DIR="${2:-}"
      [[ -n "$SRC_DIR" ]] || {
        echo "ERROR: --src requires a path" >&2
        exit 1
      }
      shift 2
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

# 默认版本取 manifest assetVersion（**不是** package.json——仓库内 package.json
# 的 version 在发版 stamp 前是旧值，如 0.2.1；安装器按 assetVersion 拼 OSS 文件名，
# 打错版本号的包上传后 404、auto 档全体静默回退 CPU）
MANIFEST_JSON="$ROOT/npm/nuwax-deploy-installer/vendor/templates/manifest.json"
ASSET_VERSION="$(MANIFEST_JSON="$MANIFEST_JSON" node -e \
  "console.log(JSON.parse(require('fs').readFileSync(process.env.MANIFEST_JSON,'utf8')).assetVersion || '')")"
VERSION="${POSITIONAL[0]:-$ASSET_VERSION}"
VERSION="${VERSION%%-*}"
[[ -n "$ASSET_VERSION" ]] || die "manifest.json has no assetVersion"
[[ "$VERSION" == "$ASSET_VERSION" ]] || \
  die "version $VERSION != manifest assetVersion $ASSET_VERSION (bump assetVersion first)"
ARCHIVE_NAME="voice-cli-vulkan-linux-x64-${VERSION}.tar.gz"
OUTPUT_DIR="$ROOT/dist/voice-cli/v${VERSION}"
OUTPUT_PATH="$OUTPUT_DIR/$ARCHIVE_NAME"

if [[ -z "$SRC_DIR" ]]; then
  SRC_DIR="$OUTPUT_DIR/linux-x64-vulkan"
fi

REQUIRED=(voice-cli libsherpa-onnx-c-api.so libonnxruntime.so)

[[ -d "$SRC_DIR" ]] || die "source dir not found: $SRC_DIR"
for f in "${REQUIRED[@]}"; do
  [[ -f "$SRC_DIR/$f" ]] || die "missing $SRC_DIR/$f"
done

log "voice-cli Vulkan linux-x64 pack"
echo "  version: $VERSION"
echo "  source:  $SRC_DIR"
echo "  output:  $OUTPUT_PATH"
echo "  OSS:     oss://nuwa-packages/${OSS_PREFIX}/v${VERSION}/${ARCHIVE_NAME}"
echo

mkdir -p "$OUTPUT_DIR"
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/voice-vulkan-pack.XXXXXX")"
cleanup() {
  rm -rf "$STAGING"
}
trap cleanup EXIT

for f in "${REQUIRED[@]}"; do
  cp "$SRC_DIR/$f" "$STAGING/"
  chmod +x "$STAGING/$f" 2>/dev/null || true
done

# 档位 marker：非空才有效（present 判定要求 len>0），内容带版本便于人工排查
echo "vulkan ${VERSION}" >"$STAGING/$MARKER"

# tar 成员 = REQUIRED + marker（与 assets.rs VOICE_CLI_VULKAN_BUNDLE_FILES 一一对应）
TAR_MEMBERS=("${REQUIRED[@]}" "$MARKER")

log "archive → $OUTPUT_PATH"
COPYFILE_DISABLE=1 tar -czf "$OUTPUT_PATH" -C "$STAGING" "${TAR_MEMBERS[@]}"

SHA="$(shasum -a 256 "$OUTPUT_PATH" | awk '{print $1}')"
SIZE="$(stat -f%z "$OUTPUT_PATH" 2>/dev/null || stat -c%s "$OUTPUT_PATH")"

FILES_JSON=$(printf '"%s",' "${TAR_MEMBERS[@]}" | sed 's/,$//')

cat >"${OUTPUT_PATH}.meta.json" <<EOF
{
  "version": "${VERSION}",
  "platform": "linux-x64-vulkan",
  "archive": "${ARCHIVE_NAME}",
  "sha256": "${SHA}",
  "size_bytes": ${SIZE},
  "files": [${FILES_JSON}],
  "oss_object_key": "${OSS_PREFIX}/v${VERSION}/${ARCHIVE_NAME}",
  "public_url": "${PUBLIC_BASE}/v${VERSION}/${ARCHIVE_NAME}"
}
EOF

log "size: $(du -h "$OUTPUT_PATH" | awk '{print $1}')"
log "metadata → ${OUTPUT_PATH}.meta.json"
echo
echo "✅ pack complete"
