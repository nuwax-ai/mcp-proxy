# FastEmbed

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# FastEmbed

基于 [fastembed-rs](https://crates.io/crates/fastembed) + ONNX Runtime 的高性能**本地**嵌入 HTTP 服务，完全在设备上运行 —— 支持文本、图像、稀疏嵌入，可选 GPU 加速。

## 功能特性

- **多类型嵌入**：一个端点支持稠密 `text`、稠密 `image`、`sparse`（SPLADE / BGE-M3）
- **40+ 文本模型**自动从 fastembed-rs 发现，零维护成本
- **GPU 加速**：CoreML（macOS）/ CUDA（Linux）/ DirectML（Windows），按 device 配置
- **本地优先**：模型缓存到磁盘，数据不出进程
- **OSS 模型分发**：启动时从 OSS/HTTP URL 拉取预打包模型，完全跳过 HuggingFace 下载
- **并发缓存**：按 (类型, 模型) 的 `DashMap` 缓存，惰性初始化，每个模型独立锁
- **并发限流**：信号量控制，默认 2× CPU 核数（可配），防止耗尽 tokio 阻塞线程池
- **OpenAPI 文档**：Swagger UI 位于 `/swagger-ui`

## 快速开始

> 注意：`crates/fastembed` 已是 workspace 成员，但**不在** `default-members` 中（ort 编译期需联网下载、编译耗时数分钟）。裸 `cargo build`/`test` 不碰它，需显式构建/测试：
>
> ```bash
> cargo build -p fastembed-server --release
> ```

```bash
# 启动服务器（默认端口 8068）
fastembed server

# 指定端口 + 从 OSS 预拉取模型
fastembed server --port 8081 \
    --model-url "https://your-bucket.oss.example.com/models/bge-large-zh-v1.5.tar.gz"
```

### 预下载模型

```bash
# 从 HuggingFace 下载（变体名或 HF 代码）
fastembed models download --type text --model AllMiniLML6V2

# 从 OSS / HTTP URL 拉取模型包（tar.gz）
fastembed models pull \
    --url "https://your-bucket.oss.example.com/models/bge-large-zh-v1.5.tar.gz" \
    --cache-dir .fastembed_cache

# 列出已下载模型
fastembed models list --type text
```

## API

### `POST /api/embeddings`

```bash
# 文本（默认）
curl -X POST http://localhost:8068/api/embeddings \
  -H "Content-Type: application/json" \
  -d '{
    "type": "text",
    "model": "AllMiniLML6V2",
    "texts": ["query: 你好世界", "passage: 本地向量化"]
  }'

# 图像（images 字段传本地图片路径）
curl -X POST http://localhost:8068/api/embeddings \
  -H "Content-Type: application/json" \
  -d '{
    "type": "image",
    "model": "ClipVitB32",
    "images": ["/path/to/a.jpg", "/path/to/b.png"]
  }'

# 稀疏（每条输入返回 {indices, values}）
curl -X POST http://localhost:8068/api/embeddings \
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
  port: 8068
fastembed:
  cache_dir: .fastembed_cache
  default_model: BGELargeZHV15        # text
  default_image_model: ClipVitB32     # image
  default_sparse_model: SPLADEPPV1    # sparse
  batch_size: 256
  device: auto                        # auto | cpu | coreml | cuda | directml
  pool_size: 1                        # 实例池大小（并发推理上限；>1 代价 N× 内存）
  model_url:                          # 可选：启动时从 OSS/HTTP 拉取模型包的 URL
```

### 环境变量覆盖

| 变量 | 覆盖 |
|------|------|
| `FASTEMBED_HOST` / `FASTEMBED_PORT` | 服务监听 |
| `FASTEMBED_CACHE_DIR` | 缓存目录 |
| `FASTEMBED_MODEL` | 默认文本模型 |
| `FASTEMBED_IMAGE_MODEL` | 默认图像模型 |
| `FASTEMBED_SPARSE_MODEL` | 默认稀疏模型 |
| `FASTEMBED_DEVICE` | 计算设备（auto/cpu/coreml/cuda/directml） |
| `FASTEMBED_BATCH_SIZE` | 批大小 |
| `FASTEMBED_POOL_SIZE` | 实例池大小（并发数；>1 = N× 内存） |
| `FASTEMBED_MODEL_URL` | 启动时模型包下载 URL |
| `FASTEMBED_MAX_CONCURRENT` | 最大并发嵌入请求数（默认 2× CPU 核数） |

### 命令行覆盖（最高优先级）

```bash
fastembed server --port 8081 --model-url <URL> --cache-dir /data/cache
```

优先级：**命令行** > **环境变量** > **配置文件** > **默认值**。

## 设计说明

- **预热范围**：启动时预热三种类型的默认模型（text + image + sparse）。文本模型做一次微型推理验证；image/sparse 仅加载 ONNX 会话。失败只记 warn 不阻止启动。
- **并发初始化**：每个 (类型, 模型) 对拥有独立的初始化锁 —— 不同模型可并行加载。仅同一模型的并发首次加载会被串行化，避免 ort 冲突。运行时推理无锁。
- **实例池**：`pool_size` 控制单模型并发推理上限。`=1`（默认）单实例串行（CPU 推理通常最优）；`>1` 创建 N 个独立 ONNX 会话允许 N 路并发（代价 N× 内存）。
- **并发限流**：嵌入请求由信号量控制（默认 2× CPU 核数，最小 4，最大 64）。超限请求排队等待，避免耗尽 tokio 阻塞线程池。
- **请求超时**：每个嵌入请求有 120 秒硬超时，超时返回 HTTP 504。
- **错误分类**：image 类型传入不存在的图片路径返回 **400**（客户端错误）；模型初始化 / 推理失败返回 **500**。
- **模型目录**：首次使用时从 `fastembed-rs` API 自动填充 40+ 文本模型，零手动维护。通过 `model_url` 预下载的模型直接放入 hf-hub 缓存格式，fastembed 发现文件已存在即跳过 HuggingFace 下载。
- **维度语义**：响应中 `dim=0` 表示稀疏模型或目录外未知模型（稠密目录内模型才有真实维度）。
- **配置文件位置**：默认读取工作目录下的 `./config.yml`。生产部署建议用环境变量覆盖或挂载配置文件。
- **进度条**：`models download` 和 `models pull` 显示下载进度；服务运行时请求触发的懒加载不打印进度条，避免污染日志。
- **BYO 模式**：CLI 的 `--onnx/--tokenizer/...` 参数已保留但**暂未实现**，传了会立即报错（避免被静默忽略）。

## 代码结构

```
src/
├── main.rs               CLI 入口
├── config.rs             AppConfig、Device 枚举、env 覆盖宏
├── cli/
│   ├── mod.rs            CLI 参数定义（server、models download/list/pull）
│   └── models.rs         download/list/pull 命令实现
├── handlers/
│   ├── embeddings.rs     POST /api/embeddings（信号量 + 超时 + spawn_blocking）
│   ├── health.rs         GET /health
│   └── models.rs         GET /api/models/available
├── server/
│   └── mod.rs            Axum 路由、AppState（Semaphore, AtomicBool）、预热、优雅关闭
└── models/
    ├── mod.rs            EmbeddingType、InitializedModel、resolve、get_or_init_model、GPU EP、测试
    ├── pool.rs           ModelPool<T>（round-robin）、MODEL_CACHE、INIT_LOCKS（每个模型独立锁）
    ├── catalog.rs        ModelEntry、动态目录（从 fastembed-rs API）、ModelInfo、本地模型扫描
    └── download.rs       download_model_from_url、download_file（进度条）、extract_tar_gz
```

## 开发

```bash
cargo build -p fastembed-server
cargo test  -p fastembed-server
cargo clippy -p fastembed-server --all-targets
```

## 许可证

MIT OR Apache-2.0
