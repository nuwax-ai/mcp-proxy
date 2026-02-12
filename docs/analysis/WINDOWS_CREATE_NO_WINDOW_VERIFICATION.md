# Windows CREATE_NO_WINDOW 处理确认报告

## 检查日期
2026-02-12

## 检查范围
mcp-proxy 代码库中所有启动子进程的地方

---

## ✅ 检查结果：全部已正确处理

### 文件 1: `mcp-proxy/src/client/core/command.rs`
- **状态**: ✅ 已处理
- **场景**: 本地命令模式（stdio CLI）
- **实现方式**: `process-wrap` + `CreationFlags`

```rust
#[cfg(windows)]
{
    use process_wrap::CreationFlags;
    // CREATE_NO_WINDOW = 0x08000000
    // 隐藏控制台窗口，避免在 GUI 应用（如 Tauri）中显示 CMD 窗口
    wrapped_cmd.wrap(CreationFlags(0x08000000));
    wrapped_cmd.wrap(JobObject);
}
```

**优点**:
- ✅ 使用 `process-wrap` 统一 API
- ✅ 同时使用 `JobObject` 管理进程树
- ✅ 位置正确（在 CommandWrap::with_new 闭包之后）
- ✅ 跨平台条件编译

---

### 文件 2: `mcp-sse-proxy/src/server_builder.rs`
- **状态**: ✅ 已处理
- **场景**: SSE 协议代理启动 MCP 服务子进程
- **实现方式**: `process-wrap::tokio` + `CreationFlags`

```rust
#[cfg(windows)]
{
    use process_wrap::tokio::CreationFlags;
    // CREATE_NO_WINDOW = 0x08000000
    // 隐藏控制台窗口，避免在 GUI 应用（如 Tauri）中显示 CMD 窗口
    wrapped_cmd.wrap(CreationFlags(0x08000000));
    wrapped_cmd.wrap(JobObject);
}
```

**优点**:
- ✅ 使用 async 版本的 `process-wrap::tokio`
- ✅ 同时使用 `JobObject` 管理进程树
- ✅ 位置正确（在 TokioCommandWrap::with_new 闭包之后）
- ✅ 详细的中英文注释

---

### 文件 3: `mcp-streamable-proxy/src/server_builder.rs`
- **状态**: ✅ 已处理（v0.1.39 修复）
- **场景**: Streamable HTTP 协议代理启动 MCP 服务子进程
- **实现方式**: 标准库 `CommandExt` + `creation_flags`

```rust
// Windows: 隐藏控制台窗口，避免在 GUI 应用（如 Tauri）中显示 CMD 窗口
// 注意：必须在所有 env/args 配置之后设置，确保不被覆盖
#[cfg(windows)]
{
    use std::os::windows::process::CommandExt;
    // CREATE_NO_WINDOW = 0x08000000
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}
```

**优点**:
- ✅ 使用标准库 API（无需额外依赖）
- ✅ 位置正确（在所有 env/args 之后）
- ✅ 明确的警告注释说明顺序重要性
- ✅ 定义为局部常量，代码清晰

**修复历史**:
- v0.1.38 之前: CREATE_NO_WINDOW 设置在 env/args 之前（可能失效）
- v0.1.39: 修复为在所有配置之后设置

---

## 代码覆盖率

### mcp-proxy 相关子进程启动点

| 组件 | 文件 | 启动场景 | Windows 处理 |
|------|------|---------|------------|
| **mcp-proxy CLI** | client/core/command.rs | 启动本地 MCP 服务（stdio 模式） | ✅ CreationFlags |
| **mcp-sse-proxy** | mcp-sse-proxy/src/server_builder.rs | 启动 MCP 服务子进程（SSE 模式） | ✅ CreationFlags |
| **mcp-streamable-proxy** | mcp-streamable-proxy/src/server_builder.rs | 启动 MCP 服务子进程（HTTP 模式） | ✅ creation_flags |

### 其他组件（不在检查范围）

| 组件 | 说明 | 是否需要处理 |
|------|------|------------|
| document-parser | Python 子进程（uv、MinerU） | ⚠️ 建议检查 |
| voice-cli | Python 子进程（TTS、Whisper） | ⚠️ 建议检查 |

---

## 实现模式对比

### 模式 A: process-wrap + CreationFlags
**使用位置**: mcp-proxy, mcp-sse-proxy

```rust
use process_wrap::tokio::{TokioCommandWrap, CreationFlags, JobObject, KillOnDrop};

let mut wrapped_cmd = TokioCommandWrap::with_new(command, |cmd| {
    cmd.args(&args);
    cmd.envs(&env);
});

#[cfg(windows)]
{
    wrapped_cmd.wrap(CreationFlags(0x08000000));
    wrapped_cmd.wrap(JobObject);
}

wrapped_cmd.wrap(KillOnDrop);
let process = TokioChildProcess::new(wrapped_cmd)?;
```

**优点**:
- 统一的 API 风格
- 自动进程树管理（JobObject）
- 自动清理（KillOnDrop）

---

### 模式 B: 标准库 CommandExt
**使用位置**: mcp-streamable-proxy

```rust
use tokio::process::Command;

let mut cmd = Command::new(command);
cmd.args(&args);
cmd.envs(&env);

#[cfg(windows)]
{
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

let child = cmd.spawn()?;
```

**优点**:
- 无需额外依赖
- 标准库稳定性
- 更直接的控制

---

## 测试建议

### 手动测试步骤（Windows）

1. **编译最新版本**:
   ```bash
   cargo build --release -p mcp-stdio-proxy
   ```

2. **测试场景 A - CLI 模式**:
   ```bash
   # 启动一个本地 MCP 服务
   mcp-proxy convert <local-mcp-service> --verbose
   ```
   **预期**: 无 CMD 窗口弹出

3. **测试场景 B - Server 模式**:
   ```bash
   # 启动 mcp-proxy 服务器
   mcp-proxy server --port 18099
   ```
   **预期**: 启动 MCP 服务插件时无 CMD 窗口弹出

4. **测试场景 C - Tauri 集成**:
   - 从 nuwax-agent 启动 mcp-proxy
   **预期**: 无 CMD 窗口（这个需要 nuwax-agent 也正确设置）

### 自动化测试（建议添加）

```rust
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::os::windows::process::CommandExt;
    
    #[tokio::test]
    async fn test_no_window_flag_is_set() {
        // 测试 CREATE_NO_WINDOW 标志是否生效
        // 可以通过检查进程树中是否有 conhost.exe 来验证
    }
}
```

---

## 结论

### ✅ 所有子进程启动点已正确处理

1. **mcp-proxy/client/core/command.rs** - ✅ 已处理
2. **mcp-sse-proxy/src/server_builder.rs** - ✅ 已处理  
3. **mcp-streamable-proxy/src/server_builder.rs** - ✅ 已处理（v0.1.39）

### 关键要点

1. **CREATE_NO_WINDOW 值**: 所有实现都使用 `0x08000000`
2. **设置顺序**: 所有实现都在 env/args 之后设置
3. **条件编译**: 所有实现都使用 `#[cfg(windows)]`
4. **注释完整**: 所有实现都包含说明目的的注释

### 仍需关注

1. **nuwax-agent 侧**: 启动 mcp-proxy.cmd 本身时需要设置 CREATE_NO_WINDOW
2. **document-parser**: Python 子进程可能需要相同处理
3. **voice-cli**: Python 子进程可能需要相同处理
4. **测试验证**: 需要在 Windows 环境实际测试确认效果

---

## 版本信息

- **检查版本**: v0.1.39
- **修复文件**: mcp-streamable-proxy/src/server_builder.rs
- **修复内容**: 将 CREATE_NO_WINDOW 移到最后设置
- **向后兼容**: 是（仅影响 Windows 平台行为）

---

## 签署

**检查人员**: Claude (Sonnet 4.5)  
**检查日期**: 2026-02-12  
**检查方法**: 代码审查 + 全局搜索 + Agent 验证  
**结论**: ✅ 所有 mcp-proxy 相关子进程启动点已正确处理 Windows CREATE_NO_WINDOW
