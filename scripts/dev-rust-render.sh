#!/bin/bash
# 编译并准备 Rust 侧的环境
# 此脚本由 package.json 中的 dev:rust-render 调用

set -e

echo "[xbxengine] 正在编译 NAPI 核心..."
cargo build -p xbxengine-api --features napi

echo "[xbxengine] 正在编译原生渲染器 (App)..."
cargo build -p xbxengine-app

echo "[xbxengine] Rust 侧构建完成。"
