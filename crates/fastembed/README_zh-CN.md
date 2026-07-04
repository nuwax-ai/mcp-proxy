# FastEmbed

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# FastEmbed

基于 [fastembed-rs](https://crates.io/crates/fastembed) + ONNX Runtime 的高性能**本地**嵌入 HTTP 服务，完全在设备上运行 —— 支持文本、图像、稀疏嵌入，可选 GPU 加速。

## 功能特性

- **多类型嵌入**：一个端点支持稠密 `text`、稠密 `image`、`sparse`（SPLADE / BGE-M3）
- **GPU 加速**：CoreML（macOS）/ CUDA（Linux）/ DirectML（Windows），按 device 配置
- **本地优先**：模型缓存到磁盘，数据不出进程
- **并发缓存**：按 (类型, 模型) 的 `DashMap` 缓存，惰性初始化
- **OpenAPI 文档**：Swagger UI 位于 `/swagger-ui`

## 快速开始

> 注意：`crates/fastembed` 已是 workspace 成员，但**不在** `default-members` 中（ort 编译期需联网下载、编译耗时数分钟）。裸 `cargo build`/`test` 不碰它，需显式构建/测试：
>
> ```bash
> cargo build -p fastembed-server --release
> ```

```bash
# 启动服务器（默认端口 8080）
fastembed server

# 指定自定义端口
fastembed server --port 8081
```

### 预下载模型

```bash
# 文本模型（变体名或 HF 代码）
fastembed models download --type text --model AllMiniLML6V2

# 图像模型
fastembed models download --type image --model ClipVitB32

# 稀疏模型
fastembed models download --type sparse --model SPLADEPPV1

# 列出已下载模型
fastembed models list --type text
```

## API

### `POST /api/embeddings`

```bash
# 文本（默认）
curl -X POST http://localhost:8080/api/embeddings \
  -H "Content-Type: application/json" \
  -d '{
    "type": "text",
    "model": "AllMiniLML6V2",
    "texts": ["query: 你好世界", "passage: 本地向量化"]
  }'

# 图像（images 字段传本地图片路径）
curl -X POST http://localhost:8080/api/embeddings \
  -H "Content-Type: application/json" \
  -d '{
    "type": "image",
    "model": "ClipVitB32",
    "images": ["/path/to/a.jpg", "/path/to/b.png"]
  }'

# 稀疏（每条输入返回 {indices, values}）
curl -X POST http://localhost:8080/api/embeddings \
  -H "Content-Type: application/json" \
  -d '{
    "type": "sparse",
    "model": "SPLADEPPV1",
    "texts": ["稀疏检索文本"]
  }'
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `type` | `text` \| `image` \| `sparse` | 默认 `text` |
| `model` | string | 变体名或 HF 代码；缺省时按类型取配置默认值 |
| `texts` | string[] | `text`/`sparse` 的文本输入 |
| `images` | string[] | `image` 类型的本地图片路径 |
| `batch_size` | int | 可选，缺省取配置 `batch_size` |

响应：`embeddings`（稠密，text/image）**或** `sparse_embeddings`（稀疏），附带 `model` 信息与 `elapsed_ms`。

### 其他端点

- `GET /health` —— 服务状态 + 预热就绪
- `GET /api/models/available?type=text|image|sparse` —— 已下载到本地的模型
- `GET /swagger-ui` —— 交互式 API 文档

## 配置

`config.yml`（首次运行自动生成）：

```yaml
server:
  host: 0.0.0.0
  port: 8080
fastembed:
  cache_dir: .fastembed_cache
  default_model: BGELargeZHV15        # text
  default_image_model: ClipVitB32     # image
  default_sparse_model: SPLADEPPV1    # sparse
  batch_size: 256
  device: auto                        # auto | cpu | coreml | cuda | directml
  pool_size: 1                        # 实例池大小（并发推理上限；>1 代价 N× 内存）
```

### 环境变量覆盖

| 变量 | 覆盖 |
|------|------|
| `FASTEMBED_HOST` / `FASTEMBED_PORT` | 服务监听 |
| `FASTEMBED_CACHE_DIR` | 缓存目录 |
| `FASTEMBED_MODEL` | 默认文本模型 |
| `FASTEMBED_IMAGE_MODEL` | 默认图像模型 |
| `FASTEMBED_SPARSE_MODEL` | 默认稀疏模型 |
| `FASTEMBED_DEVICE` | 计算设备 |
| `FASTEMBED_BATCH_SIZE` | 批大小 |
| `FASTEMBED_POOL_SIZE` | 实例池大小（并发数；>1 = N× 内存） |

## 支持的模型

**文本**（Xenova ONNX 命名空间）：`BGELargeZHV15`（Xenova/bge-large-zh-v1.5，1024 维）、`BGESmallZHV15`（512 维）、`BGEBaseENV15`（768 维）、`BGESmallENV15`（384 维）、`BGELargeENV15`（1024 维）、`AllMiniLML6V2`（384 维）、`AllMiniLML12V2`（384 维）。也接受 fastembed `EmbeddingModel::from_str` 能识别的任何模型（维度报为 0）。

**图像**：`ClipVitB32`（512 维）、`Resnet50`（2048 维）、`UnicomVitB16`（768 维）、`UnicomVitB32`（512 维）、`NomicEmbedVisionV15`（768 维）。

**稀疏**：`SPLADEPPV1`（Qdrant/Splade_PP_en_v1）、`BGEM3`（BAAI/bge-m3）。

## 开发

```bash
cargo build -p fastembed-server
cargo test  -p fastembed-server
cargo clippy -p fastembed-server --all-targets
```

## 许可证

MIT OR Apache-2.0
