# Rust 项目技术文档

## 项目概述

这是一个基于 Rust 的文档解析器项目，支持多种文档格式的解析和处理。

## 架构设计

### 核心组件

1. **格式检测器** - 自动识别文档格式
2. **解析引擎** - 支持多种格式的解析
3. **存储服务** - 管理解析结果和元数据
4. **任务队列** - 异步处理文档解析任务

### 技术栈

- **语言**: Rust 2021 Edition
- **异步运行时**: Tokio
- **数据库**: Sled (嵌入式)
- **Web 框架**: Axum
- **序列化**: Serde

## API 接口

### 文档上传

```http
POST /api/v1/documents/upload
Content-Type: multipart/form-data

file: [binary data]
format: "auto"
```

### 解析状态查询

```http
GET /api/v1/documents/{id}/status
```

### 解析结果获取

```http
GET /api/v1/documents/{id}/content
Accept: application/json
```

## 配置说明

### 环境变量

```bash
# 服务器配置
SERVER_PORT=8087
SERVER_HOST=0.0.0.0

# 日志配置
LOG_LEVEL=info
LOG_PATH=./logs/app.log

# 存储配置
SLED_PATH=./data/sled
SLED_CACHE_CAPACITY=1048576
```

## 部署指南

### 开发环境

```bash
# 克隆项目
git clone <repository-url>
cd document-parser

# 安装依赖
cargo install

# 运行测试
cargo test

# 启动服务
cargo run
```

### 生产环境

```bash
# 构建发布版本
cargo build --release

# 运行服务
./target/release/document-parser
```

## 性能指标

### 解析速度

| 文档类型 | 平均解析时间 | 内存使用 |
|----------|--------------|----------|
| Markdown | 50ms        | 2MB      |
| Word     | 200ms       | 10MB     |
| PDF      | 500ms       | 25MB     |

### 并发能力

- 最大并发解析任务：10
- 队列容量：100
- 超时设置：30秒

## 总结

这个技术文档包含了：
- 项目架构说明
- API 接口定义
- 配置和部署指南
- 性能指标数据
- 代码示例

用于测试复杂 Markdown 内容的解析能力。
