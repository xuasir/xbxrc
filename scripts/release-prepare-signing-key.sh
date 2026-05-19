#!/usr/bin/env bash
# 将 GitHub Secret 中的 minisign 私钥写入文件，并以「路径」形式供 tauri build 使用。
# 避免在 Windows 上把多行密钥内容塞进环境变量触发 base64 解析错误（如 Invalid symbol 37 / %）。
set -euo pipefail

if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  echo "错误: TAURI_SIGNING_PRIVATE_KEY 未设置"
  exit 1
fi

if [[ -f "$TAURI_SIGNING_PRIVATE_KEY" ]]; then
  echo "TAURI_SIGNING_PRIVATE_KEY 已是文件路径: $TAURI_SIGNING_PRIVATE_KEY"
  echo "TAURI_SIGNING_PRIVATE_KEY=$TAURI_SIGNING_PRIVATE_KEY" >> "$GITHUB_ENV"
  exit 0
fi

mkdir -p "$HOME/.tauri"
key_file="$HOME/.tauri/xbxrc.key"
printf '%s' "$TAURI_SIGNING_PRIVATE_KEY" > "$key_file"
chmod 600 "$key_file"
echo "已写入签名私钥: $key_file"
echo "TAURI_SIGNING_PRIVATE_KEY=$key_file" >> "$GITHUB_ENV"
