# mcp-proxy convert 兜底元数据 —— 需求与设计

> **文档层级**：Spec（做什么）+ Plan（如何实现），不包含具体 Task 拆解
> **状态**：已实现
> **日期**：2026-07-28
> **范围**：URL 形式的 SSE / Streamable HTTP `mcp-proxy convert` 发现阶段兜底；
> 共享 crate `mcp-proxy-args`；本地快照导出验证
> **调用方说明**：CLI 参数与示例见
> [`MCP_CONVERT_FALLBACK_CLI.md`](./MCP_CONVERT_FALLBACK_CLI.md)

---

## 1. 背景

真实消费方之一是 rcoder。端到端链路如下：

```text
POST /computer/chat
  └── agent_config.context_servers
        { command, args, env, ... }

rcoder 宿主机
  → gRPC / 容器内 agent_runner
  → ACP NewSession / LoadSession
       └── McpServer::Stdio { command, args, env }

Agent 子进程
  → mcp-proxy convert <url>
```

URL 形式的远程 MCP 通常按以下顺序工作：

1. `initialize` 获取协议版本、capabilities 和 serverInfo；
2. `tools/list` 获取工具列表；
3. `tools/call` 调用具体工具。

上游 MCP 如果在启动或运行期间发生短暂网络抖动，Agent 可能因为
`initialize` 或 `tools/list` 失败，将整个 MCP 判断为不可用并跳过。即使网络随后恢复，
本次 Agent 会话也可能不再拥有该 MCP 的工具。

本设计通过预先提供发现阶段快照，让 Agent 在短暂故障期间仍能看到 MCP 和工具：

```text
真实上游正常
  → 完全走正常流程

真实上游 initialize 或 tools/list 发生任意失败
  → 如果配置了完整 fallback
       → initialize / tools/list 使用快照
       → tools/call 仍返回后端不可用
       → 后台继续重连

真实上游恢复
  → 切回真实上游
```

fallback 只用于“发现阶段保活”，不伪造工具当前可调用。

---

## 2. 目标

### 2.1 功能目标

1. 为 URL 形式的 SSE / Streamable HTTP `convert` 增加可选发现快照；
2. 上游正常时始终使用真实数据；
3. 正常发现流程发生任何错误时均可使用 fallback，只有下游主动取消除外；
4. fallback 期间保持 stdio 服务存活并继续后台重连；
5. 上游恢复后自动切回真实上游；
6. 提供文件参数，避免大 JSON 触发 OS argv 长度限制；
7. 提供轻量共享 crate，供 rcoder 严格识别 `mcp-proxy convert` 后，将内联 JSON
   原子写入内容寻址缓存并重组为文件参数；
8. 提供一次性 export 参数，方便从健康上游生成可直接回灌的测试快照。

### 2.2 非目标

- 不伪造 `tools/call` 成功结果；
- 不为快照增加自定义业务 JSON 外壳；
- 不改造 `mcp-proxy proxy`；
- 不为 LocalCommand 提供 fallback；
- 不自动猜测一个参数值究竟是 JSON 还是文件路径；
- 不自动猜测上游使用 SSE 还是 Streamable HTTP；
- 不在共享 crate 中依赖 rmcp、clap、tokio 或服务端运行栈；
- 不在共享 crate 中实现缓存清理策略；
- 不在本仓库实现 rcoder 的最终 ACP 接线代码。

---

## 3. 适用范围与模式分流

`convert` 配置解析后可能得到：

| 模式 | fallback 行为 |
|------|---------------|
| DirectUrl | 支持 |
| RemoteService（SSE / Streamable HTTP） | 支持 |
| LocalCommand | 不支持；warn 后忽略全部 import 参数 |

处理顺序必须是：

```text
先解析 convert 配置来源
  ├─ LocalCommand
  │    ├─ 如果出现 import 参数，记录 warn
  │    ├─ 不读取 import 文件
  │    ├─ 不解析 import JSON
  │    └─ 按原有 run_command_mode 执行
  └─ DirectUrl / RemoteService
       └─ 加载并校验 fallback
```

LocalCommand 下 import 内容即使不存在或非法也不会导致启动失败，因为这些参数在该模式下
不适用且不会被消费。

---

## 4. CLI 设计

### 4.1 Import 参数

| 参数 | 类型 | 说明 |
|------|------|------|
| `--import-initialize` | `String` | 内联 MCP `InitializeResult` |
| `--import-initialize-file` | `PathBuf` | 从文件读取 `InitializeResult`；与上一参数互斥 |
| `--import-tools` | `String` | 内联 MCP `ListToolsResult` |
| `--import-tools-file` | `PathBuf` | 从文件读取 `ListToolsResult`；与上一参数互斥 |

initialize 和 tools 分开承载，原因是：

- 对齐 MCP 的两次 RPC；
- 不创建非官方包装结构；
- 两类结果可以独立演进；
- tools 通常明显更大，可以单独使用文件。

### 4.2 Export 参数

| 参数 | 类型 | 说明 |
|------|------|------|
| `--export-initialize` | `PathBuf` 或 `-` | 从真实上游导出 `InitializeResult` |
| `--export-tools` | `PathBuf` 或 `-` | 从真实上游导出完整 `ListToolsResult` |

export 是一次性本地采集/验证模式：

- 只从真实上游采集，绝不导出 import 或内存 fallback；
- 导出未经过 `ToolFilter` 过滤的原始上游发现结果；
- 成功导出后退出，不启动长期 stdio 服务和 watchdog；
- 可以单独导出一侧，也可以同时导出两侧；
- 两个 export 参数不能同时为 `-`；
- export 与任意 import 参数互斥；
- `export-tools` 必须遍历 `nextCursor`，合并全部分页；
- 导出文件必须能被对应 import 参数直接消费；
- 写文件使用原子替换，任一采集或写入错误返回非零退出码。

### 4.3 Import 启用条件

fallback 参数组整体可选。一旦出现任意 import 参数，就必须形成完整配置：

```text
initialize_ok =
  (--import-initialize XOR --import-initialize-file)
  AND 内容可读取
  AND 可反序列化为当前协议实现的 InitializeResult

tools_ok =
  (--import-tools XOR --import-tools-file)
  AND 内容可读取
  AND 可反序列化为当前协议实现的 ListToolsResult

fallback_enabled = initialize_ok AND tools_ok
```

| 输入 | DirectUrl / RemoteService 行为 |
|------|-------------------------------|
| 四个 import 参数都不存在 | `Ok(None)`，保持现状 |
| initialize 和 tools 两侧均合法 | `Ok(Some(fallback))` |
| 只提供一侧 | fail fast，返回参数错误 |
| 同侧 inline 和 file 同时存在 | 模式感知的参数互斥错误 |
| 文件读取失败 | fail fast，错误包含文件路径上下文 |
| JSON 非法或类型不匹配 | fail fast，错误包含来源上下文 |
| `capabilities.tools` 不存在 | fail fast |

不能在远程 URL 模式下静默关闭用户明确配置的 fallback，否则调用方会误以为容错已经生效。

四个 import 字段在 clap 层只负责接收，不使用 `conflicts_with` 提前终止。完整性和互斥校验
必须在配置来源解析之后执行，这样 LocalCommand 才能真正做到 warn 后忽略，而不会在进入
模式分流前被 clap 拒绝。

### 4.4 协议要求

上游完全不可达时，自动探测无法确定 URL 使用 SSE 还是 Streamable HTTP。为了保证
fallback 在冷启动故障时可用，启用 import fallback 时必须在访问上游之前确定协议：

- CLI 显式提供 `--protocol sse|stream`；或
- RemoteService 配置已经明确协议。

如果出现 import 参数但协议无法预先确定，应在连接上游之前 fail fast，并提示补充
`--protocol`。

export 必须连接健康上游，因此允许继续使用现有自动协议探测，也允许显式指定协议。

---

## 5. JSON 数据约束

import/export 都使用 rmcp 对应的官方 MCP 结果形状。

InitializeResult：

```json
{
  "protocolVersion": "2024-11-05",
  "capabilities": {
    "tools": {}
  },
  "serverInfo": {
    "name": "example-mcp",
    "version": "1.0.0"
  }
}
```

ListToolsResult：

```json
{
  "tools": [
    {
      "name": "search",
      "description": "搜索",
      "inputSchema": {
        "type": "object",
        "properties": {
          "q": { "type": "string" }
        },
        "required": ["q"]
      }
    }
  ]
}
```

约束：

- 字段使用官方驼峰命名；
- `capabilities.tools` 必须存在；
- 静态 tools 快照必须包含全部分页，`nextCursor` 为 `null` 或省略；
- mcp-proxy 根据最终协议分别反序列化为 SSE 与 Stream 实现所使用的 rmcp 类型；
- 不承诺不同 rmcp 版本之间所有扩展字段无损互转；
- 当前协议实现不支持的 protocolVersion 或字段类型应尽早报错。

---

## 6. 运行时状态设计

### 6.1 发现数据优先级

```text
1. 当前可用上游的实时响应
2. 本次运行中最近一次成功获取的真实快照
3. import 提供的静态快照
```

每个协议 handler 保存一个 `DiscoveryState`，其中同时包含当前最佳
`ServerInfo` 和可选的完整 `ListToolsResult`。import 负责初始填充；获取到配套的真实
initialize/tools 数据后，通过一次 `ArcSwap` 更新原子替换整个状态。单独成功完成首屏
`tools/list` 时，只更新同一状态内的 tools 并保留 info。这样所有 handler clone 都能看到
一致的发现数据，缓存本身也表达了优先级，不需要长期并存 imported 与
last_successful 两套状态。

该状态不需要 DashMap。更新使用 `ArcSwap::rcu`，避免持有 map guard 或跨异步边界持锁。

### 6.2 冷启动

```text
启动远程 URL convert
  → 校验 fallback 与协议
  → 按现有策略连接真实上游
       ├─ 成功
       │    ├─ 用真实 peer_info 创建 handler
       │    ├─ 获取并缓存真实 tools
       │    └─ 启动 stdio + watchdog
       └─ initialize 或 tools/list 任意失败
            ├─ fallback_enabled = false
            │    └─ 保持现状：启动失败
            └─ fallback_enabled = true
                 ├─ 用 import initialize 创建 disconnected handler
                 ├─ 注入 import tools
                 ├─ 启动 stdio
                 └─ 立即启动后台重连
```

### 6.3 运行中断线

```text
上游断线
  → handler 将 backend 标记为 unavailable
  → 健康检查继续只检查真实上游
  → initialize / tools/list 使用最近可用快照
  → tools/call 返回 backend unavailable
  → watchdog 后台重连
```

### 6.4 上游恢复

```text
重连成功
  → 原子替换 backend
  → 刷新真实 InitializeResult
  → 拉取完整 tools/list 并刷新真实 tools 快照
  → 后续发现请求和工具调用重新走真实上游
```

SSE 和 Stream 都必须使用上述、所有 clone 可见的原子 `DiscoveryState`，避免 backend
已经切换、但 initialize 与 tools 仍来自不同代上游的短暂不一致。

### 6.5 哪些失败允许使用 fallback

所有真实 `initialize` / `tools/list` 错误都允许 fallback，例如：

- 连接失败；
- 握手超时；
- 请求超时；
- transport 已关闭；
- 重连期间没有 backend；
- `tools/list` 的网络/传输错误。
- 鉴权失败、InvalidParams、协议错误或 RPC 业务拒绝。

唯一例外是下游 Agent 主动取消：取消请求必须返回取消错误，不能用缓存伪装成一次成功
响应。健康检查也始终直连真实 peer，不经过 fallback。

### 6.6 工具过滤

真实 tools 与所有 fallback tools 都必须经过同一个 `ToolFilter`：

```text
选择数据来源
  → 取得 ListToolsResult
  → allow/deny filter
  → 返回 Agent
```

缓存保存未过滤的官方结果，过滤在响应阶段执行，避免运行时配置与缓存内容耦合。

### 6.7 重连与进程生命周期

- `--retries=0`：持续重连，推荐用于 fallback；
- `--retries=N`：限制一次连续故障期间的重连次数；任一次重连成功后计数和退避都清零；
- watchdog 停止不能导致已经启动的 fallback stdio 服务退出；
- stdio EOF、显式取消或不可恢复的内部错误才结束代理进程；
- 上游健康判断不能读取 fallback，否则会把“发现可用”误判为“后端健康”。
- 单次连接建立设置 15 秒上限，完整发现/导出设置 30 秒上限，避免网络请求永久挂起
  fallback 启动；RemoteService 的 `timeout` 还会作为更严格的 HTTP connect/read timeout。

---

## 7. SSE 与 Stream 实现边界

当前 SSE 与 Stream 使用不同 rmcp 版本，不能在 `mcp-proxy-args` 中暴露 rmcp 类型。

| 模块 | 责任 |
|------|------|
| `mcp-proxy-args` | 读取 JSON 字符串、argv 解析、内容寻址落盘；不理解 MCP 类型 |
| `mcp-proxy` CLI | 模式分流、协议确定、将 JSON 交给对应协议实现反序列化 |
| `mcp-sse-proxy` | SSE ServerInfo/tools 缓存、断线 fallback、重连刷新 |
| `mcp-streamable-proxy` | Stream ServerInfo/tools 缓存、断线 fallback、重连刷新 |

SSE 和 Stream handler 应提供对称能力：

```rust
new_disconnected(..., fallback_info, fallback_tools)
swap_backend(...)
refresh_real_discovery(...)
is_backend_available()
```

具体类型可以不同，但对外行为、日志和测试场景必须一致。

---

## 8. 共享 crate `mcp-proxy-args`

### 8.1 目标

新建：

```text
crates/mcp-proxy-args/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── flags.rs
    ├── load.rs
    ├── rewrite.rs
    └── cache.rs
```

消费方：

1. mcp-proxy：复用 flag 名称和 inline/file 加载逻辑；
2. rcoder：ACP 前识别目标命令，将 inline JSON 写入文件并改写 argv。

允许依赖：

- `serde_json`；
- `thiserror` 或 `anyhow`；
- `sha2`。
- `tempfile`。

禁止依赖：

- rmcp；
- clap；
- tokio；
- axum；
- mcp-sse-proxy；
- mcp-streamable-proxy。

所有生产代码禁止 `unwrap()` / `expect()`，错误应附带参数名、文件路径或操作阶段上下文。
不使用 unsafe。

### 8.2 Flag 名称

供 clap 使用的名称不带 `--`：

```rust
pub const IMPORT_INITIALIZE: &str = "import-initialize";
pub const IMPORT_INITIALIZE_FILE: &str = "import-initialize-file";
pub const IMPORT_TOOLS: &str = "import-tools";
pub const IMPORT_TOOLS_FILE: &str = "import-tools-file";
pub const EXPORT_INITIALIZE: &str = "export-initialize";
pub const EXPORT_TOOLS: &str = "export-tools";
```

argv parser 负责识别 `--{name}` 和 `--{name}=value`。CLI 与 lib 必须有测试确保 flag
名称一致，避免两边漂移。

### 8.3 Load API

```rust
pub struct RawFallbackImportSpec {
    pub initialize_inline: Option<String>,
    pub initialize_file: Option<PathBuf>,
    pub tools_inline: Option<String>,
    pub tools_file: Option<PathBuf>,
}

pub enum ImportSource {
    Inline,
    File(PathBuf),
}

pub struct LoadedJson {
    pub json: String,
    pub source: ImportSource,
}

pub struct LoadedFallbackJson {
    pub initialize: LoadedJson,
    pub tools: LoadedJson,
}

/// 四项都没有时返回 Ok(None)。
/// 出现任意一项后，负责完整性、互斥、读取和 JSON 语法校验。
/// 不校验具体 MCP schema。
pub fn try_load_fallback(
    spec: &RawFallbackImportSpec,
) -> Result<Option<LoadedFallbackJson>, LoadError>;
```

mcp-proxy 在确定 DirectUrl/RemoteService 和协议后调用该 API，再把 JSON 反序列化为对应
rmcp 类型。LocalCommand 不调用该 API。

### 8.4 严格命令识别

共享 lib 只能改写真正的 `mcp-proxy convert`：

```rust
pub fn is_mcp_proxy_convert(command: &str, args: &[String]) -> bool;
```

判断规则：

1. 对 `command` 取 basename；
2. basename 必须严格等于 `mcp-proxy`，Windows 可兼容 `mcp-proxy.exe`；
3. `convert` 必须处于 CLI 子命令位置，不允许简单扫描任意参数；
4. 第一版至少支持 rcoder 当前标准形式：

```text
command = "mcp-proxy"
args    = ["convert", "<url>", ...]
```

5. 如需兼容 `-v convert` / `--quiet convert`，只跳过已知全局参数；
6. JSON 值、URL、文件名或其他参数中出现字符串 `convert` 不得误判。

以下命令必须原样返回且无文件副作用：

```text
node ...
bunx ...
mcp-proxy proxy ...
mcp-proxy health ...
other-mcp-proxy convert ...
```

### 8.5 Rewrite API

```rust
pub struct RewriteOptions {
    /// 必须对最终 Agent 子进程可见。
    pub cache_dir: PathBuf,
    pub file_prefix: String,
}

pub struct RewriteResult {
    pub args: Vec<String>,
    /// 本次实际新建的文件；已存在并复用的文件不包含在内。
    pub created_files: Vec<PathBuf>,
    pub changed: bool,
}

pub fn rewrite_convert_import_args_to_files(
    command: &str,
    args: &[String],
    options: &RewriteOptions,
) -> Result<RewriteResult, RewriteError>;
```

行为：

```text
不是 mcp-proxy convert
  → changed=false
  → args 原样返回
  → 不创建目录、不写文件

是 mcp-proxy convert
  → 完整解析所有 import 参数
  → 校验重复、缺值、inline/file 冲突、两侧完整性
  → 读取并校验所有 inline/file 来源的 JSON 语法
  → 已是 *-file 的参数原样保留
  → inline 内容按原始 JSON 字节计算 SHA-256
  → 原子写入内容寻址缓存
  → 将 inline 参数改写为 *-file <path>
  → 保留其他参数的顺序和值
```

支持输入形式：

```text
--import-tools <json>
--import-tools=<json>
```

输出统一规范化为：

```text
--import-tools-file
<path>
```

重复 flag、flag 缺值、同侧冲突或只提供一侧均返回 `Err`。

LocalCommand 是 mcp-proxy CLI 解析 `--config` 内容后才能确定的模式。共享 lib 保持轻量，
只做命令和 argv 级识别，不解析 mcp-proxy 业务配置；最终是否忽略 import 由 CLI 决定。

### 8.6 内容寻址缓存

固定使用内容寻址，不提供关闭开关：

```text
{cache_dir}/{prefix}-initialize-{sha256_hex}.json
{cache_dir}/{prefix}-tools-{sha256_hex}.json
```

要求：

- 对收到的原始 JSON UTF-8 字节计算 SHA-256；
- hash 文件名使用完整 64 字符 SHA-256 hex；
- 相同内容复用同一路径；
- 先完整解析所有参数，再执行任何写入；
- 在目标目录创建临时文件；
- 使用同目录 `NamedTempFile`，完整写入并 `sync_all` 后通过
  `persist_noclobber` 原子发布；
- 并发发现目标已存在时复用；
- 不覆盖 hash 不匹配的已有文件；
- Debian 下缓存目录权限为 `0700`、最终文件权限为 `0600`；
- 错误必须包含目标路径和操作上下文。

rcoder 默认使用 `std::env::temp_dir().join("mcp-proxy-import-cache")`；Debian 容器中即
`/tmp/mcp-proxy-import-cache`。临时文件只承担原子写入，最终内容寻址文件不会随
`NamedTempFile` drop 删除，而是保留到容器临时目录生命周期结束。共享 crate 不主动
清理缓存。

`mcp-proxy` 启动时一次性读取并反序列化文件到内存；command 启动完成后不再依赖文件
持续存在，因此后续系统清理不会影响当前进程。

### 8.7 rcoder 调用顺序

```text
1. rewrite_convert_import_args_to_files
2. enhance_mcp_proxy_args
3. 构造 ACP McpServer::Stdio
```

示例：

```rust
use mcp_proxy_args::{
    RewriteOptions,
    rewrite_convert_import_args_to_files,
};

fn prepare_mcp_stdio(
    command: String,
    args: Vec<String>,
    cache_dir: PathBuf,
) -> anyhow::Result<(String, Vec<String>)> {
    let rewritten = rewrite_convert_import_args_to_files(
        &command,
        &args,
        &RewriteOptions {
            cache_dir,
            file_prefix: "mcp-proxy-import".to_string(),
        },
    )?;

    let final_args = enhance_mcp_proxy_args(&command, rewritten.args);
    Ok((command, final_args))
}
```

`cache_dir` 必须在 rcoder 写文件的位置和 Agent 最终执行 `mcp-proxy` 的环境中指向同一可见
路径。

---

## 9. Export 实现

export 复用正常连接能力，但不复用 fallback 响应：

```text
解析 export 参数
  → 校验未同时出现 import
  → 从 CLI 或 RemoteService 配置取得明确协议
  → 连接真实上游
  → 读取 peer_info / InitializeResult
  → tools/list 循环读取全部分页
  → 序列化为官方 JSON
  → 原子写入文件或输出单份 JSON 到 stdout
  → exit 0
```

约束：

- 同时指定 initialize 和 tools export 时，任一失败则整体返回失败；
- 两个目标都是文件时，可分别原子提交；
- 若要求严格的跨文件事务不现实，错误日志必须指出哪个文件已成功写入；
- stdout 只允许输出一份 JSON，日志继续写 stderr；
- export 输出不经过 `ToolFilter`；
- export 不启动 stdio server；
- export 不启动 watchdog；
- export 不读取 import；
- export 结果应通过一次反向反序列化测试，确保可被 import 消费。

---

## 10. 日志

| 场景 | 级别与内容 |
|------|------------|
| 未配置 import | debug：fallback disabled |
| LocalCommand 带 import | warn：fallback import ignored for local command mode |
| fallback 加载成功 | info：enabled，记录 inline/file 来源，不打印完整 JSON |
| 初始上游失败并启用 fallback | warn/info：using fallback metadata |
| tools/list 使用 fallback | warn/debug：来源为 last-real 或 imported |
| tools/call 后端不可用 | warn/error：backend unavailable，可重试 |
| 上游恢复 | info：backend reconnected, switched to live metadata |
| export 成功 | info：类型、工具数量、目标路径；stdout 模式日志只写 stderr |
| rcoder rewrite 跳过非目标命令 | debug，不创建文件 |
| rcoder rewrite 完成 | info/debug：改写类型和文件路径，不打印 JSON |

日志不得打印：

- auth/header 完整值；
- import JSON 全文；
- 工具 schema 全文。

可以记录工具数量和截断后的工具名列表。

---

## 11. 错误处理

遵循 Fail Fast：

- 远程模式下，用户明确提供的 fallback 配置不完整或非法时立即失败；
- export 参数冲突或导出失败时立即失败；
- rewrite 在写文件前完成所有可完成的校验；
- 不使用 `unwrap()` / `expect()`；
- I/O、JSON、协议转换错误增加上下文；
- 明确记录真实发现错误；鉴权、协议和 RPC 错误也允许使用发现快照；
- LocalCommand 是明确例外：import 不适用，因此 warn 后忽略。

建议错误类型按职责拆分：

```text
LoadError
  ├─ IncompletePair
  ├─ ConflictingSources
  ├─ ReadFile
  └─ InvalidJsonSyntax

RewriteError
  ├─ MissingValue
  ├─ DuplicateFlag
  ├─ IncompletePair
  ├─ ConflictingSources
  ├─ Load
  ├─ NonUtf8CachePath
  └─ CacheWrite

FallbackParseError
  ├─ InvalidInitialize
  ├─ InvalidTools
  ├─ MissingToolsCapability
  └─ UnsupportedProtocolVersion
```

---

## 12. 测试与验收

### 12.1 CLI 参数与模式

- [ ] 四个 import 参数都不存在：行为与当前版本一致；
- [ ] 完整 inline、完整 file、混合来源均可启用；
- [ ] 只提供一侧：fail fast；
- [ ] 同侧 inline/file 冲突：远程模式的模式感知校验拒绝；
- [ ] 文件不存在、JSON 非法、schema 不匹配：错误包含来源；
- [ ] fallback 已配置但协议未知：提示显式指定 `--protocol`；
- [ ] LocalCommand + 任意 import：warn、忽略、不读取文件；
- [ ] import 与 export 同时出现：参数错误；
- [ ] 两个 export 同时为 stdout：参数错误。

### 12.2 冷启动与恢复

- [ ] 上游正常 + fallback：返回真实 initialize/tools；
- [ ] 上游启动失败 + fallback：stdio 可完成 initialize/tools/list；
- [ ] fallback 期间 tools/call 返回后端不可用；
- [ ] 上游恢复：切回真实 initialize/tools/call；
- [ ] fallback 未配置时保持现有失败行为；
- [ ] 有限 retries 耗尽后 fallback stdio 不退出；
- [ ] 健康检查始终命中真实 backend。

### 12.3 SSE / Stream 对称性

- [ ] SSE 和 Stream 均支持 disconnected fallback 启动；
- [ ] 两条路径重连时都刷新真实 ServerInfo 和 tools；
- [ ] 两条路径都对任意真实发现错误 fallback，并排除下游主动取消；
- [ ] 两条路径都应用 ToolFilter；
- [ ] 不同 rmcp 版本不能解析的快照明确失败。

### 12.4 Export

- [ ] 单独导出 initialize；
- [ ] 单独导出 tools；
- [ ] 同时导出两个文件；
- [ ] tools 分页被完整合并；
- [ ] stdout 只输出 JSON，日志在 stderr；
- [ ] 导出文件可直接被 import 消费；
- [ ] 上游失败时不会导出 fallback。

### 12.5 `mcp-proxy-args`

- [ ] `mcp-proxy convert` 标准形式被识别；
- [ ] 绝对路径和 Windows basename 按规则识别；
- [ ] `mcp-proxy proxy`、node、bunx 等原样返回且无文件副作用；
- [ ] 参数值中出现 `convert` 不会误判；
- [ ] 支持 `--flag value` 和 `--flag=value`；
- [ ] inline 被改写为 file，其他参数保持顺序；
- [ ] 已是 file 的参数保持；
- [ ] 重复、缺值、冲突、单侧配置返回错误；
- [ ] 相同 JSON 二次调用复用相同路径；
- [ ] 并发写入不会产生半文件；
- [ ] 写入失败不会返回部分改写后的 args。

---

## 13. 涉及模块

| 模块 | 改动 |
|------|------|
| `crates/mcp-proxy-args/` | 新建 flags/load/parse/rewrite/cache |
| 根 `Cargo.toml` | workspace member 与 dependency |
| `crates/mcp-proxy/.../support/args.rs` | 新增 6 个 CLI 参数 |
| `crates/mcp-proxy/.../cli_impl/convert_cmd.rs` | 模式分流、LocalCommand 忽略、import/export 校验 |
| `crates/mcp-proxy/.../core/convert.rs` | fallback 协议要求、export 分支 |
| `crates/mcp-proxy/.../core/sse.rs` | 冷启动 fallback、watchdog 生命周期 |
| `crates/mcp-proxy/.../core/stream.rs` | 冷启动 fallback、watchdog 生命周期 |
| `crates/mcp-sse-proxy/.../sse_handler.rs` | 可更新 info/tools 快照与断线 fallback |
| `crates/mcp-streamable-proxy/.../proxy_handler.rs` | tools 快照与断线 fallback |
| rcoder（独立 PR） | ACP 前调用 rewrite，再调用现有 enhance |

---

## 14. 决策记录

| 决策 | 原因 |
|------|------|
| 功能落在 `convert`，不落在 `proxy` | Agent 使用 URL 远程 MCP 的方向是 URL → stdio |
| 只支持 DirectUrl / RemoteService | LocalCommand 不受远程网络抖动影响 |
| LocalCommand 忽略 import 并 warn | 保持兼容，不让不适用参数阻断本地命令 |
| 正常上游永远优先 | fallback 只处理失败路径 |
| initialize/tools 双全才启用 | 避免 Agent 得到半套发现信息 |
| 远程模式非法配置 fail fast | 防止静默失去容错能力 |
| fallback 前必须确定协议 | 上游完全不可达时无法自动识别 SSE/Stream |
| 不伪造 tools/call | 工具是否真实执行必须由上游决定 |
| 同时支持 inline/file | inline 便于测试，file 规避 argv 限制 |
| 新建轻量 `mcp-proxy-args` | mcp-proxy 与 rcoder 共用语义，不引入服务端依赖 |
| 严格识别 `mcp-proxy convert` | 避免修改其他 MCP command |
| 固定内容寻址和原子写入 | 支持复用并避免并发半文件 |
| export 为一次性真实上游采集 | 方便本地生成并回灌验证快照 |
| 缓存清理由 rcoder 管理 | 共享 lib 不掌握 session/workspace 生命周期 |
