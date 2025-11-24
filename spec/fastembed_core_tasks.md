# FastEmbed 核心开发任务清单

- [进行中] 实现 GET /health 接口
  - 路由/方法：GET /health
  - 返回：JSON {status, uptime_ms, model_cache_ready}
  - 计算：uptime_ms = 当前时间 - 服务器启动时间；model_cache_ready 由预热完成后标记
  - 错误语义：正常返回 200；若主循环未就绪可返回 500（简化版可不实现）
  - 验收标准：启动后请求 /health 返回上述字段；状态为 ok；耗时低于 20ms
- [待办] 集成优雅关闭
  - 信号：捕获 SIGINT/SIGTERM
  - 行为：停止接收新连接；继续处理在途请求；等待最多 30s
  - 实现：axum Server 的 with_graceful_shutdown + tokio 信号/取消令牌
  - 拒绝策略：关闭阶段新请求统一返回 503
  - 验收标准：收到信号后 30s 内优雅退出；日志包含开始关闭/完成关闭标记
- [待办] 实现预热策略
  - 时机：服务启动完成后立即执行
  - 行为：根据 fastembed.default_model 调用 get_or_init_model；可选执行一次微型嵌入 ["passage: warmup"]
  - 标志：预热完成后设置 model_cache_ready = true（用于 /health）
  - 验收标准：启动后 3s 内完成预热；/health 的 model_cache_ready 为 true
- [待办] 配置读取与默认值
  - 来源：命令行 --config 或当前目录 config.yml（不存在则生成默认配置）
  - 参数：cache_dir、default_model、max_length、batch_size、normalize（按设计文档默认值）
  - 环境变量：FASTEMBED_CACHE_DIR 可覆盖 cache_dir
  - 验收标准：服务启动日志打印最终配置；缺失项按默认值生效

- [待办] 依赖与工程搭建
  - 修改 fastembed/Cargo.toml，增加：axum、tokio、serde/serde_json、serde_yaml、tracing、clap、fastembed（features: ort-download-binaries,hf-hub-native-tls）
  - （可选）增加 tower-http（CORS/限流/压缩）
  - （可选）使用本地源码：在 workspace 下通过 path 或 [patch.crates-io] 引入 `/Volumes/soddygo/git_work/mcp-proxy/temp/fastembed-rs`
  - 验收标准：`cargo build -p fastembed` 成功；依赖版本统一

- [待办] Axum 服务骨架
  - 在 fastembed/src/main.rs 初始化 Router 与中间件（CORS、DefaultBodyLimit）
  - 注册路由：GET /health、POST /api/embeddings、GET /api/models/available
  - 集成 with_graceful_shutdown
  - 验收标准：服务可启动并响应三条路由；Ctrl-C 优雅退出

- [待办] 配置加载与默认文件
  - 启动参数支持 --port 与 --config；未提供配置文件时生成默认 config.yml（与设计文档一致）
  - 从 YAML 加载 fastembed.cache_dir/default_model/max_length/batch_size/normalize
  - 验收标准：日志打印最终配置；生成的默认文件包含完整字段

- [待办] 集成 fastembed-rs 与预热
  - 通过 `get_or_init_model` 构建会话；在启动后进行预热（可用一次微型嵌入）
  - 预热成功设置 model_cache_ready = true，/health 反映该状态
  - 验收标准：预热在 3s 内完成；/health 返回就绪

- [待办] 构建与运行
  - 构建：`cargo build -p fastembed`
  - 运行：`cargo run -p fastembed -- server --port 8080`
  - 验收标准：端点可访问，日志与退出流程符合预期

