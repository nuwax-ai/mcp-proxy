# mcp-proxy convert —— 兜底元数据 CLI 参数说明

> **读者**：使用或组装 `mcp-proxy convert` 命令的开发者（含 TypeScript）
> **状态**：已实现（最终以 `--help` 为准）
> **完整设计**：见 [`MCP_CONVERT_FALLBACK_METADATA_DESIGN.md`](./MCP_CONVERT_FALLBACK_METADATA_DESIGN.md)

本文只说明 `convert` 子命令新增的兜底参数、适用范围和运行时行为。

---

## 1. 解决什么问题

`mcp-proxy convert <url>` 将远程 SSE 或 Streamable HTTP MCP 转换为本地 stdio，供 Agent 使用。

正常情况下，Agent 会依次执行：

1. `initialize`：获取协议版本、服务信息和 capabilities；
2. `tools/list`：获取工具列表；
3. `tools/call`：调用具体工具。

如果远程 MCP 在启动或运行期间发生短暂网络抖动，Agent 可能因为 `initialize` 或
`tools/list` 失败而将整个 MCP 判断为不可用并跳过。即使网络随后恢复，本次 Agent
会话也可能不再拥有这个 MCP 的工具。

新增参数允许预先提供一份官方 MCP 形状的 `InitializeResult` 和
`ListToolsResult` 快照：

- 上游正常时，始终使用真实上游，不使用快照覆盖真实数据；
- 上游真实发现流程发生任何失败时，都可用快照应答
  `initialize` / `tools/list`；
- 下游 Agent 主动取消请求时直接返回取消错误，不使用快照；
- `tools/call` 不会伪造成功；上游不可用时仍返回可重试错误；
- 后台继续重连，上游恢复后重新使用真实上游。

这是一项“发现阶段保活”能力，不表示上游工具当前一定可调用。

---

## 2. 适用范围

兜底参数只适用于以下 URL 形式的远程 MCP：

- `mcp-proxy convert <url>`；
- `--config` / `--config-file` 最终解析出的 RemoteService；
- 远程协议为 SSE 或 Streamable HTTP。

如果 `--config` / `--config-file` 最终解析为 LocalCommand：

- import 参数会被忽略；
- 记录一条 warn 日志；
- 不读取 import 文件，也不解析 import JSON；
- LocalCommand 按原有流程执行。

`mcp-proxy proxy` 不在本功能范围内。

---

## 3. 新增参数

```bash
mcp-proxy convert <URL> [选项...]
```

| 参数 | 值类型 | 必填 | 说明 |
|------|--------|------|------|
| `--import-initialize` | JSON 字符串 | 否 | 内联 MCP `InitializeResult` |
| `--import-initialize-file` | 文件路径 | 否 | 从文件读取 `InitializeResult`；与上一参数互斥 |
| `--import-tools` | JSON 字符串 | 否 | 内联 MCP `ListToolsResult` |
| `--import-tools-file` | 文件路径 | 否 | 从文件读取 `ListToolsResult`；与上一参数互斥 |
| `--export-initialize` | 文件路径或 `-` | 否 | 从真实上游导出 `InitializeResult`；`-` 表示 stdout |
| `--export-tools` | 文件路径或 `-` | 否 | 从真实上游导出完整 `ListToolsResult`；`-` 表示 stdout |

fallback 参数组整体可选：

- 四个参数都不传时，行为与当前版本完全一致；
- 一旦传入任意 import 参数，initialize 和 tools 两侧都必须配置完整且内容合法；
- initialize 和 tools 可以混用来源，例如 initialize 内联、tools 使用文件；
- 生产环境建议始终使用 `*-file`，避免命令行参数长度限制。

`--export-*` 是独立的一次性采集模式，用于本地测试和生成可回灌的快照，不参与
`fallback_enabled` 判断。

---

## 4. 启用与校验规则

```text
initialize_ok =
  恰好提供 --import-initialize / --import-initialize-file 中的一个
  并且内容可读取、可解析

tools_ok =
  恰好提供 --import-tools / --import-tools-file 中的一个
  并且内容可读取、可解析

fallback_enabled = initialize_ok AND tools_ok
```

| 情况 | 行为 |
|------|------|
| 两侧都没传 | 不启用兜底，保持现状 |
| 只配置 initialize 或只配置 tools | 参数错误，启动失败 |
| 同侧同时配置 inline 与 file | 远程模式下参数互斥错误，启动失败 |
| 文件不存在或不可读 | 参数加载错误，启动失败 |
| JSON 非法或不是对应 MCP 类型 | 参数解析错误，启动失败 |
| 两侧均合法 | 启用兜底 |
| LocalCommand + 任意 import 参数 | warn 后忽略，不读取、不解析 |

对于远程 URL 模式，错误配置不会静默降级。这样可以避免调用方误以为兜底已生效。
互斥和完整性校验在识别配置模式后执行，因此 LocalCommand 不会被不适用的 import
参数阻断。

---

## 5. 协议要求

上游在进程启动时完全不可达时，自动探测无法判断 URL 使用 SSE 还是 Streamable HTTP。
为了保证兜底可以在这种情况下启动，启用 fallback 时必须能在访问上游之前确定协议：

- 通过命令行显式提供 `--protocol sse` 或 `--protocol stream`；或
- RemoteService 配置中已经明确协议。

如果启用了 fallback，但协议既未显式指定、也无法从配置确定，则参数校验失败。

生产示例：

```bash
mcp-proxy convert https://example.com/mcp \
  --protocol stream \
  --import-initialize-file /data/mcp/example.initialize.json \
  --import-tools-file /data/mcp/example.tools.json
```

---

## 6. 运行时行为

```text
启动 convert
  → 解析 URL / RemoteService / LocalCommand
       ├─ LocalCommand
       │    └─ warn 并忽略 import，走原有本地命令流程
       └─ URL / RemoteService
            → 校验并加载 import
            → 连接真实上游
                 ├─ 成功
                 │    └─ initialize / tools/list / tools/call 均走真实上游
                 └─ initialize 或 tools/list 任意失败
                      ├─ fallback 已启用
                      │    ├─ 用快照启动 stdio
                      │    ├─ initialize / tools/list 返回快照
                      │    ├─ tools/call 返回后端不可用错误
                      │    └─ 后台继续重连
                      └─ fallback 未启用
                           └─ 保持现状：启动失败

运行中上游断开
  ├─ fallback 已启用
  │    ├─ initialize / tools/list 返回最近可用快照
  │    ├─ tools/call 返回后端不可用错误
  │    └─ 后台继续重连
  └─ fallback 未启用
       └─ 保持现状

上游恢复
  → 切换回真实上游
  → 后续 tools/list / tools/call 使用真实结果
```

发现数据的优先级是：

```text
当前可用上游的真实数据
  > 本次运行中最近一次成功获取的真实数据
  > import 提供的静态快照
```

健康检查始终访问真实上游，不会被 fallback 快照伪装成健康。

`--retries=0` 表示持续重连，也是兜底场景的推荐配置。若配置有限重试次数，它限制
一次连续故障期间的失败次数；重连成功后计数与退避会清零。重试耗尽后可以停止重连，
但已启动的 fallback stdio 服务不应因此退出。

---

## 7. JSON 格式

内联参数值和对应文件内容使用相同的官方 MCP JSON 形状，不增加自定义外壳。

### 7.1 InitializeResult

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

启用 tools fallback 时，`capabilities.tools` 必须存在，可以是空对象 `{}`。

### 7.2 ListToolsResult

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

字段名保持 MCP 官方驼峰形式，例如 `protocolVersion`、`serverInfo` 和
`inputSchema`。

如果上游 `tools/list` 使用分页，生成快照的一方应先拉取所有分页，再保存完整工具列表；
静态快照应使用 `nextCursor = null` 或省略该字段。

---

## 8. 使用示例

### 8.1 小数据测试：内联 JSON

```bash
mcp-proxy convert https://example.com/mcp \
  --protocol stream \
  --import-initialize '{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"example-mcp","version":"1.0.0"}}' \
  --import-tools '{"tools":[{"name":"search","description":"搜索","inputSchema":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}}]}'
```

### 8.2 生产：文件路径

```bash
mcp-proxy convert https://example.com/mcp \
  --protocol stream \
  --import-initialize-file /data/mcp/example.initialize.json \
  --import-tools-file /data/mcp/example.tools.json
```

### 8.3 TypeScript argv

```typescript
import { spawn } from "node:child_process";

const child = spawn(
  "mcp-proxy",
  [
    "convert",
    "https://example.com/mcp",
    "--protocol",
    "stream",
    "--import-initialize-file",
    initPath,
    "--import-tools-file",
    toolsPath,
  ],
  { stdio: "pipe" },
);
```

应使用参数数组，不要把整条命令拼成 shell 字符串。大 JSON 应先写入文件，再传短路径。
rcoder 场景中的自动落盘和 argv 改写由共享库 `mcp-proxy-args` 完成，详见完整设计文档。
在 Debian 容器中默认写入 `/tmp/mcp-proxy-import-cache`：最终内容寻址文件保留到容器
生命周期结束，`mcp-proxy` 启动时读取到内存后不再依赖文件持续存在。

---

## 9. 导出快照用于本地验证

```bash
mcp-proxy convert https://example.com/mcp \
  --protocol stream \
  --export-initialize ./example.initialize.json \
  --export-tools ./example.tools.json
```

导出模式的行为固定为：

- 只从当前真实上游采集，不使用 import 或内存 fallback 数据；
- 导出未经过 `--allow-tools` / `--deny-tools` 过滤的原始上游发现结果；
- 成功连接后导出指定内容并退出，成功退出码为 `0`；
- 任一连接、RPC、分页或写文件操作失败时返回非零退出码；
- `--export-tools` 必须沿 `nextCursor` 拉取全部分页，再写出完整工具列表；
- 写出的 JSON 必须能够被对应的 `--import-*` / `--import-*-file` 直接消费；
- 可以只导出 initialize、只导出 tools，或同时导出两者；
- 两个 export 参数不能同时使用 `-`，避免 stdout 中连续出现两个无边界 JSON；
- export 模式不启动长期 stdio 代理，也不启动后台重连 watchdog；
- export 与任意 import 参数同时出现时视为参数冲突并报错。

单独输出到 stdout：

```bash
mcp-proxy convert https://example.com/mcp \
  --protocol stream \
  --export-tools -
```

---

## 10. 与现有参数的关系

- `--auth`、`-H`：仍用于访问真实上游；
- `--allow-tools`、`--deny-tools`：同时过滤真实工具列表和 fallback 工具列表；
- `--ping-interval`、`--ping-timeout`：仍只检查真实上游；
- `--retries`：控制后台重连次数，`0` 表示持续重连；
- `--protocol`：fallback 启动前必须能够明确 SSE 或 Stream；
- 日志参数语义不变。

---

## 11. 常见问题

**Q：只传 `--import-tools-file` 可以吗？**
A：不可以。fallback 是一套完整的发现快照，initialize 和 tools 必须同时提供。

**Q：上游正常时还会读取 fallback 文件吗？**
A：远程 URL 模式会在启动时读取并校验，但不会用它覆盖真实上游响应。

**Q：LocalCommand 会读取 fallback 文件吗？**
A：不会。LocalCommand 会记录 warn 并忽略全部 import 参数。

**Q：为什么 fallback 时建议显式传 `--protocol`？**
A：上游完全不可达时无法自动判断传输协议，必须提前知道应该启动 SSE 还是 Stream 路径。

**Q：工具调用会不会用快照伪造成功？**
A：不会。上游不可用时 `tools/call` 返回错误，由 Agent 决定是否重试。

**Q：网络恢复后会继续使用旧快照吗？**
A：不会。上游恢复后切换回真实上游，真实响应始终优先。

**Q：如何获得可用于 import 的快照？**
A：对健康的真实 MCP 使用 `--export-initialize` 和 `--export-tools`。导出成功后可直接回灌验证。
