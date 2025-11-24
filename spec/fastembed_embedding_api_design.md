# FastEmbed 文本向量 API 设计（BGELargeZHV15）

## 1. 背景与目标
- 目标：提供一个 HTTP 接口，接收文本，返回其向量嵌入；可通过请求参数指定模型（默认与示例使用 BGELargeZHV15）。
- 适用场景：检索增强、语义搜索、向量数据库入库前的批量嵌入。
- 技术选型：
  - 嵌入计算：`fastembed`（基于 ONNX Runtime 与 `tokenizers`）。
  - Web 服务：`axum`（与现有工作区风格一致）。
  - 可观测性：`tracing` + （可选）`opentelemetry`。

## 2. 模型选择与参数规范
- 目标模型：`EmbeddingModel::BGELargeZHV15`
  - 维度（dim）：1024
  - 模型代码（model_code）：`Xenova/bge-large-zh-v1.5`
  - 说明：中文大模型 v1.5，适合中文检索与语义匹配。
- 请求参数 `model` 支持两种标识：
  - 变体名：`BGELargeZHV15`
  - 模型代码：`Xenova/bge-large-zh-v1.5`
- 文本前缀建议：遵循 BGE 系列习惯，`query:` 与 `passage:` 前缀有助于优化效果（非强制）。

## 3. 依赖与初始化
- 依赖（工程搭建）：
  - Web 服务：`axum`、`tokio`、`serde/serde_json`、`tracing`（可选 `opentelemetry`）、`serde_yaml`（配置）
  - CLI/配置：`clap`（server 子命令）
  - 嵌入：`fastembed = "5"`（features：`ort-download-binaries`,`hf-hub-native-tls`）
  - 可选：`tower-http` 用于 CORS/限流/Body 限制

  Cargo.toml 依赖示例：
  ```toml
  [dependencies]
  axum = "0.7"
  tokio = { version = "1", features = ["macros","rt-multi-thread"] }
  serde = { version = "1", features = ["derive"] }
  serde_json = "1"
  serde_yaml = "0.9"
  tracing = "0.1"
  clap = { version = "4", features = ["derive"] }
  fastembed = { version = "5", features = ["ort-download-binaries","hf-hub-native-tls"] }
  ```

  本地源码路径（可选）：
  - fastembed-rs 源码位置：`/Volumes/soddygo/git_work/mcp-proxy/temp/fastembed-rs`
  - 工作区开发可通过 path 依赖或临时覆盖：
  ```toml
  [patch.crates-io]
  fastembed = { path = "temp/fastembed-rs" }
  ```
  - 或直接以 workspace 成员方式引入。
  - `fastembed = "5"`
  - 默认特性：`ort-download-binaries`（自动下载ONNX runtime），`hf-hub-native-tls`（下载模型与分词器文件）。
- 初始化选项：
  - 使用 `TextInitOptions` 创建 `TextEmbedding`；可设置：`execution_providers`、`cache_dir`、`show_download_progress`、`max_length`。
  - 默认 `max_length = 512`；可按需要覆盖。
- 缓存目录：
  - 环境变量：`FASTEMBED_CACHE_DIR`，默认 `.fastembed_cache`。
- 执行提供者（EP）：
  - 默认 CPU；如需要 GPU/特定 EP，可通过 `execution_providers` 注入（例如 CUDA、CoreML 等，具体取决于 ORT 版本与平台）。

## 4. HTTP API 设计
### 4.1 路由与方法
- `POST /api/embeddings`

### 4.2 请求体（JSON）
```
{
  "model": "BGELargeZHV15",            // 可选，默认 BGELargeZHV15；也支持 "Xenova/bge-large-zh-v1.5"
  "texts": ["query: ...", "passage: ..."],
  "batch_size": 256,                     // 可选；默认 256；对非量化模型适用
  "max_length": 512,                     // 可选；若不指定使用模型默认
  "normalize": true                      // 可选；是否对输出进行 L2 归一化（默认 true）
}
```

### 4.3 响应体（JSON）
```
{
  "model": {
    "variant": "BGELargeZHV15",
    "code": "Xenova/bge-large-zh-v1.5",
    "dim": 1024
  },
  "count": 2,
  "embeddings": [
    [0.00123, -0.00456, ...],
    [0.00078,  0.00234, ...]
  ],
  "elapsed_ms": 12
}
```

### 4.4 错误响应
- 400 Bad Request：参数缺失/非法（例如 `texts` 为空或过长、模型不支持）。
- 413 Payload Too Large：单次提交文本数过多或文本长度超限。
- 500 Internal Server Error：模型加载/推理失败、底层依赖异常。
```
{
  "error": "INVALID_MODEL",
  "message": "Unknown model: BGELargeZHV15",
  "status": 400
}
```

### 4.5 可用模型查询（离线安装）
- 路由与方法：
  - `GET /api/models/available`
- 设计原则：仅返回“本地已安装（离线已下载且文件完整）”的模型；避免在查询时触发网络下载。
- 可选查询参数（Query）：
  - `type`: `text` | `image` | `sparse`，默认 `text`。用于筛选模型类别。
- 可用性判定（本地缓存检查）：
  - 基于 `FASTEMBED_CACHE_DIR`（默认 `.fastembed_cache`；或使用服务配置的 `cache_dir`）进行文件存在性检查：
    - 文本模型：必须存在 `model_file` 与分词器文件（`tokenizer.json`、`config.json`、`special_tokens_map.json`、`tokenizer_config.json`）；若 `additional_files` 非空，也需存在。
    - 稀疏文本模型：同上（以 `SparseModel` 的 `model_file` 与分词器文件为准）。
    - 图像模型：必须存在 `model_file` 与 `preprocessor_config.json`；若 `additional_files` 非空，也需存在。
  - 注：具体路径结构依赖 HF hub 的缓存实现（`hf-hub`），服务端应通过与下载时一致的 `cache_dir` 与文件名进行存在性校验（不进行在线拉取）。
- 响应体（JSON）：
```
{
  "type": "text",
  "count": 2,
  "models": [
    { "variant": "BGELargeZHV15", "code": "Xenova/bge-large-zh-v1.5", "dim": 1024 },
    { "variant": "BGESmallZHV15", "code": "Xenova/bge-small-zh-v1.5", "dim": 512 }
  ]
}
```
- 错误响应示例：
  - `400 Bad Request`：`type` 参数非法。
  - `500 Internal Server Error`：缓存目录不可访问/文件读取异常。
- 典型实现思路（伪代码）：
```rust
use fastembed::{EmbeddingModel, ImageEmbeddingModel, SparseModel};
use std::path::PathBuf;

fn list_available_text_models(cache_dir: PathBuf) -> anyhow::Result<Vec<(EmbeddingModel, ModelInfo<EmbeddingModel>)>> {
    fastembed::TextEmbedding::list_supported_models()
        .into_iter()
        .filter(|minfo| files_exist(cache_dir.clone(), minfo))
        .map(|minfo| Ok((minfo.model.clone(), minfo)))
        .collect()
}

fn files_exist(cache_dir: PathBuf, minfo: &ModelInfo<EmbeddingModel>) -> bool {
    // 按实际缓存结构检查 model_file / additional_files 与 tokenizer 相关文件
    // 不进行网络请求；仅本地文件存在性判断
    check_model_file(cache_dir.clone(), &minfo.model_code, &minfo.model_file)
        && minfo.additional_files.iter().all(|f| check_model_file(cache_dir.clone(), &minfo.model_code, f))
        && check_tokenizer_files(cache_dir, &minfo.model_code)
}
```

## 5. 路由与中间件
- 框架：`axum`
- 建议中间件：
  - CORS：允许常见跨域请求（GET/POST/PUT/DELETE/PATCH）。
  - Body 限制：`DefaultBodyLimit` 控制最大请求大小（例如 20MB）。
  - 可观测性：`opentelemetry` 中间件，为每次请求生成 `trace_id` 与耗时头（如 `x-request-id`、`x-server-time`）。

## 6. 模型会话缓存与线程模型
- 会话缓存：
  - 使用 `dashmap` 维护 `model_key → Arc<TextEmbedding>` 的缓存，避免重复拉取与初始化。
  - `model_key` 支持两种格式：变体名（如 `BGELargeZHV15`）与模型代码（如 `Xenova/bge-large-zh-v1.5`）；内部统一映射到 `EmbeddingModel`。
- 并发：
  - `TextEmbedding::embed` 内部是 CPU 并行安全的（会话在多个请求间共享）。
  - 通过 `available_parallelism()` 自动设置 ORT `intra_threads`；必要时允许通过环境变量覆盖。

## 7. 批处理策略与吞吐
- 默认 `batch_size = 256`（非量化模型）。
- 动态量化模型（名称以 `Q` 结尾，如 `BGELargeENV15Q`）：需禁用真实批处理（`batch_size = texts.len()`），否则不同批量产生的嵌入不可直接比较；`BGELargeZHV15` 不受此限制。
- 大批量处理：对 `texts` 进行 `chunks(batch_size)` 切分，逐批推理后拼接结果，保证稳定吞吐与内存占用。

## 8. 伪代码（服务侧实现示例）
```rust
use fastembed::{TextEmbedding, TextInitOptions, EmbeddingModel};
use once_cell::sync::Lazy;
use dashmap::DashMap;
use std::sync::Arc;

static MODEL_CACHE: Lazy<DashMap<EmbeddingModel, Arc<TextEmbedding>>> = Lazy::new(|| DashMap::new());

fn parse_model(user_input: &str) -> Result<EmbeddingModel, String> {
    // 允许两种输入：变体名与模型代码
    match user_input {
        // 变体名直映射
        "BGELargeZHV15" => Ok(EmbeddingModel::BGELargeZHV15),
        // 模型代码用 FromStr
        other => other.parse::<EmbeddingModel>().map_err(|e| e),
    }
}

fn get_or_init_model(em: EmbeddingModel, max_length: Option<usize>) -> anyhow::Result<Arc<TextEmbedding>> {
    if let Some(existing) = MODEL_CACHE.get(&em) { return Ok(existing.clone()); }
    let mut options = TextInitOptions::new(em);
    if let Some(n) = max_length { options.max_length = n; }
    let model = TextEmbedding::try_new(options)?;
    let arc = Arc::new(model);
    MODEL_CACHE.insert(em, arc.clone());
    Ok(arc)
}

pub async fn handle_embed(req: EmbedRequest) -> Result<EmbedResponse, AppError> {
    let em = parse_model(req.model.as_deref().unwrap_or("BGELargeZHV15"))
        .map_err(|_| AppError::Validation("INVALID_MODEL".into()))?;
    let model = get_or_init_model(em, req.max_length)?;
    let mut model_ref = Arc::clone(&model);
    let vectors = model_ref.embed(req.texts, req.batch_size)?;
    Ok(EmbedResponse { /* 填充响应 */ })
}
```

## 9. 监控与日志
- 指标：
  - QPS、平均耗时、错误率、模型加载耗时、一次请求的文本数分布等。
- 日志：
  - 记录模型标识、文本数量、批大小、耗时、异常详情（避免敏感信息）。
- Trace：
  - 通过 `opentelemetry` 上报到 Jaeger 或其它后端；关键 span：下载模型、构造会话、分词、推理、导出向量。

## 10. 安全与限流
- 限流：基于 IP 或 API Key，防止滥用；可选在中间件层实现。
- 大小限制：`texts.len()` 上限（例如 1024）；单文本长度上限（例如 8k tokens 以内，配合 `max_length` 截断）。
- 入参校验：拒绝空文本、超长文本、非法编码；明确返回错误码。

## 11. 兼容性与可移植性
- 平台：macOS/Linux；依赖 ORT 二进制的下载与可用性。
- 缓存：`FASTEMBED_CACHE_DIR` 可指向持久化磁盘；CI/CD 环节可预热模型以降低首请求延迟。

## 12. 后续规划
- 模型列举接口：`GET /api/models` 返回 `TextEmbedding::list_supported_models()` 的子集（含 `variant/code/dim`）。
- 多模型支持：允许在请求中切换到 `EmbeddingModel::BGESmallZHV15` 等其它中文模型或英文模型。
- 向量后处理：统一 L2 归一化（若模型未内置），或曝光开关供调用方选择。
- 错误细化：区分下载失败（HF 网络）、文件损坏、会话初始化失败、分词失败、推理失败等。

## 13. CLI（clap）模型离线下载
- 可行性：可通过命令行指定模型，离线下载所需文件并写入缓存目录，供服务端作为“可用模型”使用。
  - 对于内置支持的模型（如 `BGELargeZHV15`、`BGESmallZHV15` 等），可直接用变体名或模型代码触发下载。
  - 对于未内置但在 Hugging Face 存在的仓库（例如 `BAAI/bge-base-zh-v1.5`），可使用“用户自定义（BYO）”模式，显式指定文件名映射后下载对应文件。

### 13.1 命令设计
- 二进制示例：`fastembed-cli`
- 子命令：`models download`
- 语法：
```
fastembed-cli models download \
  --type text \
  --model BGELargeZHV15 \
  --cache-dir .fastembed_cache \
  --progress true
```
- 或使用模型代码：
```
fastembed-cli models download \
  --type text \
  --code Xenova/bge-large-zh-v1.5 \
  --cache-dir .fastembed_cache
```
- BYO（未内置模型，如 `BAAI/bge-base-zh-v1.5`）：
```
fastembed-cli models download \
  --type text \
  --code BAAI/bge-base-zh-v1.5 \
  --onnx onnx/model.onnx \
  --tokenizer tokenizer.json \
  --config config.json \
  --special-tokens special_tokens_map.json \
  --tokenizer-config tokenizer_config.json \
  --cache-dir .fastembed_cache
```

### 13.2 clap 参数结构示例
```rust
#[derive(clap::Subcommand)]
pub enum ModelsCmd {
    Download(DownloadArgs),
}

#[derive(clap::Args)]
pub struct DownloadArgs {
    /// text | image | sparse
    #[arg(long, default_value = "text")]
    pub r#type: String,

    /// 内置模型变体名，如 BGELargeZHV15（可选）
    #[arg(long)]
    pub model: Option<String>,

    /// Hugging Face 模型代码，如 Xenova/bge-large-zh-v1.5 或 BAAI/bge-base-zh-v1.5（可选）
    #[arg(long)]
    pub code: Option<String>,

    /// BYO 文件名映射（文本模型）
    #[arg(long)]
    pub onnx: Option<String>,
    #[arg(long)]
    pub tokenizer: Option<String>,
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long, alias = "special_tokens")]
    pub special_tokens_map: Option<String>,
    #[arg(long)]
    pub tokenizer_config: Option<String>,

    /// 缓存目录
    #[arg(long, default_value = ".fastembed_cache")]
    pub cache_dir: std::path::PathBuf,

    /// 显示下载进度
    #[arg(long, default_value_t = true)]
    pub progress: bool,
}
```

### 13.3 下载与校验流程
- 内置模型路径：
  - `TextEmbedding::try_new(TextInitOptions)` 会通过 `hf-hub` 自动拉取 `model_file` 与分词器四件套；如 `additional_files` 非空也会拉取。
- 模型代码路径：
  - 若 `code` 对应内置模型，则与上同；否则进入 BYO 模式：调用 `hf-hub` 下载用户指定的 `onnx` 与分词器文件名；再进行存在性校验。
- 校验点：
  - `model_file` 存在；分词器文件完整；`additional_files` 存在（如有）；（图像模型需 `preprocessor_config.json`）。
- 失败处理：
  - 返回明确的错误信息（缺文件名、拉取失败、文件损坏等），不标记为可用模型。

### 13.4 与可用模型接口的联动
- 下载完成并校验通过后，`GET /api/models/available` 即可列出该模型（按 `type` 分类）。
- BYO 下载的模型以 `code` + 文件映射作为唯一标识，供服务端缓存判断。

### 13.5 可行性结论
- 使用 `clap` 提供命令行模型下载是可行的：
  - 内置模型（如中文 `BGELargeZHV15/Small`）可直接下载并使用。
  - `BAAI/bge-base-zh-v1.5` 这类未内置仓库可通过 BYO 模式下载 ONNX 与分词器文件后使用（需正确提供文件名映射）。

## 14. CLI Server（axum HTTP 服务）
- 功能：通过命令行启动内置的 axum HTTP 服务，提供本设计文档中的接口（`POST /api/embeddings`、`GET /api/models/available`）。
- 可执行命令：`fastembed-cli server`
- 选项：
  - `--port <u16>`：指定监听端口，默认 `8080`
  - `--config <path>`：指定配置文件路径，默认当前目录 `./config.yml`
- 配置文件策略：
  - 若 `--config` 指定路径存在则按文件加载；
  - 若未指定且当前目录没有 `config.yml`，启动前自动生成默认配置文件，并打印生成路径；
  - 默认配置内容示例（YAML）：
    ```yaml
    server:
      host: "0.0.0.0"
      port: 8080
    fastembed:
      cache_dir: ".fastembed_cache"
      default_model: "BGELargeZHV15"   # 也可以使用模型代码 Xenova/bge-large-zh-v1.5
      max_length: 512
      batch_size: 256
      normalize: true
    ```
- 端点暴露：
  - `GET /health`（健康检查）
  - `POST /api/embeddings`（文本向量化）
  - `GET /api/models/available`（可用模型查询，离线校验）
- 示例：
  - 指定端口与配置：
    ```bash
    fastembed-cli server --port 8081 --config ./config.yml
    ```
  - 使用默认端口，自动生成默认配置：
    ```bash
    fastembed-cli server
    # 输出：未发现 ./config.yml，已生成默认配置到 ./config.yml，正在监听 0.0.0.0:8080
    ```
- 运行时行为：
  - 优先从配置文件读取 `fastembed.cache_dir/default_model/max_length/batch_size/normalize` 等；
  - 如配置缺失，使用代码内默认值（与上方 YAML 一致）；
  - 启动时初始化 `MODEL_CACHE`（仅在首次处理某模型请求时真正加载，会话随后复用）。
  
  ## 15. 健康检查 /health、优雅关闭与预热策略
  - 健康检查：
    - 路由与方法：`GET /health`
    - 返回示例：`{"status":"ok","uptime_ms":1234,"model_cache_ready":true}`
    - 判定：进程正常、HTTP 监听成功、主循环存活；`model_cache_ready` 表示默认模型是否已完成预热。
  - 优雅关闭：
    - 监听系统信号（SIGINT/SIGTERM），停止接收新连接；等待正在处理的请求完成或超时（如 30s）后再关闭。
    - 关闭阶段拒绝新请求并返回 503；刷新日志与指标后退出。（实现上可基于 `axum`/`tokio` 的 `with_graceful_shutdown` 模式。）
  - 预热策略：
    - 启动后根据配置 `fastembed.default_model` 进行一次会话预热：调用 `get_or_init_model` 构造 `TextEmbedding`；可选执行一次极小的嵌入，如 `["passage: warmup"]`，以加载 ORT 会话与分词器。
    - 预热完成后将 `MODEL_CACHE` 标记就绪，`/health` 的 `model_cache_ready` 为 true。

