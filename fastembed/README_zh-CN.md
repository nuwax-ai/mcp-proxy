# FastEmbed

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# FastEmbed

使用 FastEmbed 库的高性能文本嵌入 HTTP 服务，用于高效的文本向量化。

## 概述

`fastembed` 是一个基于 Rust 构建的高性能文本嵌入服务，提供 HTTP API 用于使用 FastEmbed 进行文本向量化。

## 功能特性

- **FastEmbed 集成**: 使用 FastEmbed 5.0 和 ONNX 运行时
- **HTTP API**: 用于文本嵌入的 RESTful API
- **并发处理**: 使用 DashMap 进行高效的并发操作
- **OpenAPI 文档**: 自动生成的 API 文档
- **多种模型**: 支持各种嵌入模型

## 快速开始

### 安装

```bash
# 从源码构建
cargo build --release -p fastembed

# 二进制文件位置
ls target/release/fastembed
```

### 使用

```bash
# 启动服务器（默认端口 8080）
fastembed server

# 指定自定义端口
fastembed server --port 8081
```

### API 使用

```bash
# 生成嵌入向量
curl -X POST http://localhost:8080/embed \
  -H "Content-Type: application/json" \
  -d '{
    "texts": ["Hello world", "Fast embedding"],
    "model": "BAAI/bge-small-en-v1.5"
  }'
```

## 支持的模型

- `BAAI/bge-small-en-v1.5` - 快速英语模型（384 维）
- `BAAI/bge-base-en-v1.5` - 平衡英语模型（768 维）
- `BAAI/bge-large-en-v1.5` - 高质量英语模型（1024 维）

## 开发

```bash
# 构建
cargo build -p fastembed

# 测试
cargo test -p fastembed
```

## 许可证

MIT OR Apache-2.0

## 贡献

欢迎提交 Issue 和 Pull Request！
