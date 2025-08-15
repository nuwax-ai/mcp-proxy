#!/bin/bash

# Document Parser 测试服务器启动脚本

echo "🚀 启动 Document Parser 测试服务器..."
echo "📍 工作目录: $(pwd)"
echo "🔧 配置文件: config.yml"
echo "🌐 服务地址: http://localhost:8087"
echo ""

# 检查配置文件是否存在
if [ ! -f "config.yml" ]; then
    echo "❌ 错误: 找不到配置文件 config.yml"
    exit 1
fi

# 检查 Cargo.toml 是否存在
if [ ! -f "Cargo.toml" ]; then
    echo "❌ 错误: 找不到 Cargo.toml 文件"
    exit 1
fi

# 创建必要的目录
echo "📁 创建必要的目录..."
mkdir -p logs
mkdir -p data
mkdir -p /tmp/mineru
mkdir -p /tmp/markitdown

# 设置环境变量
export RUST_LOG=info
export RUST_BACKTRACE=1

echo "🔨 编译并启动服务器..."
echo "💡 提示: 使用 Ctrl+C 停止服务器"
echo "📖 API测试文件: test_api.rest"
echo ""

# 启动服务器
cargo run --bin document-parser