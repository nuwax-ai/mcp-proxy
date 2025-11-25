# FastEmbed 核心开发任务清单

## 阶段 1：基础设施搭建（必须按顺序完成）

- [待办] 任务 1.1：依赖与工程搭建
  - 修改 fastembed/Cargo.toml，增加：
    - Web: axum、tokio、tower-http（CORS/限流/压缩）
    - 序列化: serde、serde_json、serde_yaml
    - CLI: clap（derive 特性）
    - 日志: tracing、tracing-subscriber
    - 嵌入: fastembed = "5"（features: ort-download-binaries, hf-hub-native-tls）
    - 缓存: dashmap、once_cell
  - （可选）使用本地源码：在 workspace 下通过 [patch.crates-io] 引入 `/Volumes/soddygo/git_work/mcp-proxy/temp/fastembed-rs`
  - 验收标准：`cargo build -p fastembed` 成功；依赖版本与 workspace 统一

- [待办] 任务 1.2：配置模块实现
  - 创建 src/config.rs：定义 ServerConfig、FastEmbedConfig、AppConfig 结构体
  - 默认值：cache_dir=".fastembed_cache"、default_model="BGELargeZHV15"、max_length=512、batch_size=256、normalize=true
  - 环境变量支持：FASTEMBED_CACHE_DIR 可覆盖 cache_dir
  - 自动生成默认配置文件：若不存在 config.yml，生成默认配置并打印路径
  - 验收标准：可从 YAML 加载配置；日志打印最终配置；缺失项按默认值生效

- [待办] 任务 1.3：CLI 命令结构设计
  - 创建 src/cli/mod.rs：定义 clap 命令结构
  - 主命令：fastembed（二进制名）
  - 子命令：
    - `server`：启动 HTTP 服务（参数：--port, --config）
    - `models`：模型管理（子子命令：download, list）
  - 验收标准：`cargo run -p fastembed -- --help` 显示命令帮助

## 阶段 2：核心功能模块（可并行开发）

- [待办] 任务 2.1：模型缓存与管理模块
  - 创建 src/models/mod.rs：
    - 实现 MODEL_CACHE（DashMap<EmbeddingModel, Arc<TextEmbedding>>）
    - 实现 parse_model 函数（支持变体名和模型代码）
    - 实现 get_or_init_model 函数（缓存复用逻辑）
    - 实现 list_available_models 函数（本地文件检查，不触发下载）
  - 验收标准：单元测试通过；缓存命中率可验证

- [待办] 任务 2.2：实现 POST /api/embeddings 接口
  - 创建 src/handlers/embeddings.rs：
    - 定义 EmbedRequest、EmbedResponse 结构体
    - 实现 handle_embed 处理器：
      - 参数验证（texts 非空、长度限制）
      - 调用 get_or_init_model 获取模型
      - 批处理逻辑（chunks(batch_size)）
      - 计算耗时并返回
    - 错误处理：400（参数错误）、413（负载过大）、500（模型错误）
  - 验收标准：
    - 可处理单文本和批量文本
    - 返回正确维度的向量（1024）
    - 性能：100 条文本耗时 < 2s

- [待办] 任务 2.3：实现 GET /api/models/available 接口
  - 创建 src/handlers/models.rs：
    - 定义 ModelsResponse 结构体
    - 实现 handle_list_models 处理器：
      - 支持 type 参数筛选（text/image/sparse）
      - 调用 list_available_models 检查本地缓存
      - 不触发网络下载
    - 错误处理：400（参数非法）、500（缓存目录异常）
  - 验收标准：
    - 仅返回已下载的模型
    - 响应时间 < 100ms
    - 离线环境可正常运行

- [待办] 任务 2.4：实现 GET /health 接口
  - 创建 src/handlers/health.rs：
    - 定义 HealthResponse 结构体
    - 计算 uptime_ms（当前时间 - 服务器启动时间）
    - 读取 model_cache_ready 状态（由预热设置）
  - 验收标准：
    - 返回 200 状态码
    - 包含 status、uptime_ms、model_cache_ready 字段
    - 响应时间 < 20ms

## 阶段 3：HTTP 服务集成

- [待办] 任务 3.1：Axum 服务骨架
  - 修改 src/main.rs：
    - 初始化 Router 并注册路由：
      - GET /health
      - POST /api/embeddings
      - GET /api/models/available
    - 添加中间件：
      - CORS（允许所有来源）
      - DefaultBodyLimit（20MB）
      - TraceLayer（请求日志）
    - 集成 AppState（启动时间、配置、缓存就绪标志）
  - 验收标准：服务可启动并响应三条路由；日志输出正常

- [待办] 任务 3.2：实现预热策略
  - 在服务启动后、监听端口前执行：
    - 根据 fastembed.default_model 调用 get_or_init_model
    - 执行一次微型嵌入：embed(["passage: warmup"])
    - 预热成功后设置 AppState.model_cache_ready = true
  - 超时处理：预热超过 30s 则打印警告但继续启动
  - 验收标准：
    - 启动后 3s 内完成预热
    - /health 的 model_cache_ready 为 true
    - 首次请求无模型加载延迟

- [待办] 任务 3.3：集成优雅关闭
  - 实现信号处理：
    - 捕获 SIGINT/SIGTERM
    - 停止接收新连接
    - 等待在途请求完成（最多 30s）
  - 关闭阶段行为：
    - 新请求返回 503 Service Unavailable
    - 日志打印开始关闭/完成关闭标记
  - 使用 axum with_graceful_shutdown + tokio::signal
  - 验收标准：
    - Ctrl-C 后 30s 内优雅退出
    - 在途请求正常完成
    - 日志完整记录关闭过程

## 阶段 4：CLI 模型下载功能（可选，优先级低）

- [待办] 任务 4.1：实现 models download 命令
  - 创建 src/cli/models.rs：
    - 定义 DownloadArgs（type, model, code, cache_dir, progress 等）
    - 实现内置模型下载逻辑（调用 TextEmbedding::try_new 触发下载）
    - 实现 BYO 模式（自定义模型代码 + 文件映射）
    - 下载后校验文件完整性
  - 验收标准：
    - `fastembed models download --model BGELargeZHV15` 成功下载
    - 下载后 /api/models/available 可列出该模型
    - 显示下载进度条

- [待办] 任务 4.2：实现 models list 命令
  - 显示所有已下载的模型（复用 list_available_models）
  - 输出格式：表格（variant, code, dim, type）
  - 验收标准：`fastembed models list` 正确显示本地模型

## 阶段 5：测试与验证

- [待办] 任务 5.1：集成测试
  - 测试场景：
    - 启动服务 → 预热 → 健康检查
    - 发送文本嵌入请求 → 验证向量维度和结果
    - 查询可用模型 → 验证离线工作
    - 优雅关闭 → 验证信号处理
  - 验收标准：所有测试通过

- [待办] 任务 5.2：性能基准测试
  - 使用 criterion 测试：
    - 单文本嵌入耗时
    - 批量（100/500/1000）文本吞吐
    - 并发请求处理能力
  - 验收标准：达到设计文档性能目标

## 构建与运行

- 构建：`cargo build -p fastembed --release`
- 运行服务：`cargo run -p fastembed -- server --port 8080`
- 下载模型：`cargo run -p fastembed -- models download --model BGELargeZHV15`
- 列出模型：`cargo run -p fastembed -- models list`

