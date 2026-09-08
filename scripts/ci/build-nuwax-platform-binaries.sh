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
  x86_64-pc-windows-msvc:windows-x64) ;;
  *)
    echo "unsupported target:vendor-key pair: $TARGET:$VENDOR_KEY" >&2
    echo "supported: aarch64-apple-darwin:darwin-arm64 | x86_64-unknown-linux-gnu:linux-x64 | x86_64-pc-windows-msvc:windows-x64" >&2
    exit 1
    ;;
esac

echo "==> Building deploy-installer + document-parser ($TARGET)"
cd "$ROOT"
cargo build --release -p deploy-installer -p document-parser --target "$TARGET"

# voice-cli：Windows 上尝试构建，失败降级（sherpa/onnx Windows 链未完全验证）
# ——vendor 只含两件套，doctor/voice-cli install 会给出明确提示
VOICE_CLI_BUILT=1
if [[ "$VENDOR_KEY" == windows-* ]]; then
  if ! cargo build --release -p voice-cli --target "$TARGET"; then
    echo "WARN: voice-cli Windows build failed — shipping deploy-installer + document-parser only" >&2
    VOICE_CLI_BUILT=0
  fi
else
  cargo build --release -p voice-cli --target "$TARGET"
fi

echo "==> Copying binaries into vendor/$VENDOR_KEY"
DEST="$PKG/vendor/$VENDOR_KEY"
mkdir -p "$DEST"
REL="target/$TARGET/release"
EXE_SUFFIX=""
[[ "$VENDOR_KEY" == windows-* ]] && EXE_SUFFIX=".exe"
cp "$REL/deploy-installer$EXE_SUFFIX" "$DEST/"
cp "$REL/document-parser$EXE_SUFFIX" "$DEST/"
if [[ "$VOICE_CLI_BUILT" == 1 ]]; then
  cp "$REL/voice-cli$EXE_SUFFIX" "$DEST/"
fi

# voice-cli companion shared libs: must sit next to the binary on all platforms
# （Windows DLL 同目录解析是默认行为；DLL 清单以构建产物为准，glob 拷贝）
if [[ "$VENDOR_KEY" == darwin-* ]]; then
  COMPANION_LIBS=(libsherpa-onnx-c-api.dylib libonnxruntime.1.24.4.dylib libonnxruntime.dylib)
elif [[ "$VENDOR_KEY" == windows-* ]]; then
  COMPANION_LIBS=()
  for dll in "$REL"/*.dll; do
    [[ -f "$dll" ]] && COMPANION_LIBS+=("$(basename "$dll")")
  done
  if [[ "$VOICE_CLI_BUILT" == 1 && ${#COMPANION_LIBS[@]} -eq 0 ]]; then
    echo "WARN: no DLLs found in $REL (voice-cli may fail to start)" >&2
  fi
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

# linux 切片 strip 控制包体积（mac/windows 切片保持原样）
if [[ "$VENDOR_KEY" == linux-* ]]; then
  echo "==> Stripping linux slice"
  strip "$DEST"/*
fi

chmod +x "$DEST"/*

echo "==> vendor/$VENDOR_KEY ready: $(ls "$DEST" | tr '\n' ' ')"
