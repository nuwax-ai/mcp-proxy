#!/usr/bin/env bash
# Build one platform slice of nuwax-deploy-installer vendor binaries.
#
# Usage: build-nuwax-platform-binaries.sh <rust-target> <vendor-key>
#   e.g. build-nuwax-platform-binaries.sh x86_64-unknown-linux-gnu linux-x64
#
# Only the binary/so slice goes here (into npm/nuwax-deploy-installer/vendor/<key>/);
# templates + manifest/package.json stamping live in stamp-nuwax-manifest.sh.
# Voice-cli companion libs must sit next to the binary (mac @loader_path/@rpath,
# linux RPATH=$ORIGIN) — same directory layout on both platforms.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PKG="$ROOT/npm/nuwax-deploy-installer"
TARGET="${1:?usage: build-nuwax-platform-binaries.sh <rust-target> <vendor-key>}"
VENDOR_KEY="${2:?usage: build-nuwax-platform-binaries.sh <rust-target> <vendor-key>}"

case "$TARGET:$VENDOR_KEY" in
  aarch64-apple-darwin:darwin-arm64) ;;
  x86_64-unknown-linux-gnu:linux-x64) ;;
  *)
    echo "unsupported target:vendor-key pair: $TARGET:$VENDOR_KEY" >&2
    echo "supported: aarch64-apple-darwin:darwin-arm64 | x86_64-unknown-linux-gnu:linux-x64" >&2
    exit 1
    ;;
esac

echo "==> Building deploy-installer + document-parser + voice-cli ($TARGET)"
cd "$ROOT"
cargo build --release -p deploy-installer -p document-parser -p voice-cli --target "$TARGET"

echo "==> Copying binaries into vendor/$VENDOR_KEY"
DEST="$PKG/vendor/$VENDOR_KEY"
mkdir -p "$DEST"
REL="target/$TARGET/release"
cp "$REL/deploy-installer" "$DEST/"
cp "$REL/document-parser" "$DEST/"
cp "$REL/voice-cli" "$DEST/"

# voice-cli companion shared libs: must sit next to the binary on both platforms
if [[ "$VENDOR_KEY" == darwin-* ]]; then
  COMPANION_LIBS=(libsherpa-onnx-c-api.dylib libonnxruntime.1.24.4.dylib libonnxruntime.dylib)
else
  COMPANION_LIBS=(libsherpa-onnx-c-api.so libsherpa-onnx-cxx-api.so libonnxruntime.so)
fi
for lib in "${COMPANION_LIBS[@]}"; do
  if [[ -f "$REL/$lib" ]]; then
    cp "$REL/$lib" "$DEST/"
  else
    echo "WARN: missing $REL/$lib (voice-cli may fail to start)" >&2
  fi
done

# linux 切片 strip 控制包体积（mac 切片保持原样，与已发布线一致）
if [[ "$VENDOR_KEY" == linux-* ]]; then
  echo "==> Stripping linux slice"
  strip "$DEST"/*
fi

chmod +x "$DEST"/*

echo "==> vendor/$VENDOR_KEY ready: $(ls "$DEST" | tr '\n' ' ')"
