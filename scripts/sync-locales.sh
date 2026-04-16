#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_DIR="$ROOT_DIR/locales"

TARGET_CRATES=(
  "mcp-common"
  "mcp-proxy"
  "document-parser"
  "voice-cli"
  "oss-client"
)

LOCALE_FILES=(
  "en.yml"
  "zh-CN.yml"
  "zh-TW.yml"
)

for crate in "${TARGET_CRATES[@]}"; do
  target_dir="$ROOT_DIR/$crate/locales"
  mkdir -p "$target_dir"
  for locale_file in "${LOCALE_FILES[@]}"; do
    cp "$SOURCE_DIR/$locale_file" "$target_dir/$locale_file"
  done
  echo "synced locales -> $target_dir"
done

