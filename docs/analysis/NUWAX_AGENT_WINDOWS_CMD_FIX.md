# nuwax-agent Windows CMD 窗口隐藏完整解决方案

## 问题总结

### 现象
在 Windows 上运行 nuwax-agent (Tauri 应用) 时，会弹出多个 CMD 命令行窗口，影响用户体验。

### 受影响的服务
所有通过 npm 全局安装的 Node.js 服务（以 `.cmd` 文件形式存在）：

1. **mcp-proxy.cmd** - MCP 代理服务
   ```
   C:\Users\MECHREVO\.local\bin\mcp-proxy.cmd
   ```

2. **nuwax-file-server.cmd** - 文件服务器
   ```
   C:\Users\MECHREVO\.local\bin\nuwax-file-server.cmd
   ```

3. **其他潜在的 npm 全局包**
   - `nuwaxcode`
   - `claude-code-acp-ts`
   - 任何未来通过 `npm install -g` 安装的服务

### 根本原因

#### 技术细节
Windows 上的 npm 全局包会生成批处理包装文件：
```
~/.local/bin/
├── package-name.cmd       # Windows 批处理文件
├── package-name           # Unix shell 脚本（Windows 不使用）
└── package-name.ps1       # PowerShell 脚本（可选）
```

当 Rust 代码使用 `std::process::Command` 或 `tokio::process::Command` 启动这些 `.cmd` 文件时：

**默认行为**：
```rust
Command::new("C:\\...\\mcp-proxy.cmd").spawn()?;
// ❌ 会弹出 CMD 窗口
```

**需要显式设置**：
```rust
#[cfg(windows)]
{
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}
// ✅ 隐藏 CMD 窗口
```

#### 层级关系说明

```
┌─────────────────────────────────────┐
│ nuwax-agent.exe (Tauri GUI 应用)   │
└─────────────────┬───────────────────┘
                  │
      ┌───────────┴───────────┐
      │                       │
      ▼                       ▼
┌─────────────────┐   ┌─────────────────┐
│ mcp-proxy.cmd   │   │ file-server.cmd │  ❌ 问题1: nuwax-agent 启动时未隐藏
└─────┬───────────┘   └─────────────────┘
      │
      ▼
┌─────────────────┐
│ MCP 服务子进程  │  ✅ 已修复 (mcp-proxy v0.1.39)
└─────────────────┘
```

**结论**：
- **mcp-proxy v0.1.39** 修复了：mcp-proxy 启动 MCP 子进程时的 CMD 窗口
- **nuwax-agent 需要修复**：启动 npm 全局包时的 CMD 窗口

---

## 解决方案

### 方案 1: 统一的进程启动工具函数 ✅ 推荐

#### 1.1 创建通用工具模块

**文件**: `nuwax-agent-core/src/utils/process.rs` (新建)

```rust
//! 进程启动工具函数
//! 
//! 提供跨平台的进程启动功能，Windows 上自动隐藏 CMD 窗口

use std::process::Command as StdCommand;
use tokio::process::Command as TokioCommand;

/// 创建隐藏 CMD 窗口的同步 Command（Windows）
/// 
/// # 用法
/// ```rust
/// let mut cmd = create_hidden_command("C:\\path\\to\\script.cmd");
/// cmd.env("PORT", "8080");
/// cmd.args(&["--production"]);
/// cmd.spawn()?;
/// ```
#[cfg(windows)]
pub fn create_hidden_command(program: impl AsRef<std::ffi::OsStr>) -> StdCommand {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    
    let mut cmd = StdCommand::new(program);
    
    // 注意：必须在所有 env/args 调用之后才能确保标志生效
    // 这里先设置，但调用者不应该再次修改
    cmd.creation_flags(CREATE_NO_WINDOW);
    
    cmd
}

#[cfg(not(windows))]
pub fn create_hidden_command(program: impl AsRef<std::ffi::OsStr>) -> StdCommand {
    StdCommand::new(program)
}

/// 创建隐藏 CMD 窗口的异步 Command（Windows）
#[cfg(windows)]
pub fn create_hidden_tokio_command(program: impl AsRef<std::ffi::OsStr>) -> TokioCommand {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    
    let mut cmd = TokioCommand::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    
    cmd
}

#[cfg(not(windows))]
pub fn create_hidden_tokio_command(program: impl AsRef<std::ffi::OsStr>) -> TokioCommand {
    TokioCommand::new(program)
}
```

#### 1.2 使用方式

**在服务启动代码中**：

```rust
use crate::utils::process::create_hidden_tokio_command;

// FileServer 启动
pub async fn start_file_server(config: &FileServerConfig) -> Result<Child> {
    let cmd_path = "C:\\Users\\...\\nuwax-file-server.cmd";
    
    let mut cmd = create_hidden_tokio_command(cmd_path);
    cmd.env("PORT", &config.port.to_string());
    cmd.env("NODE_ENV", "production");
    cmd.args(&["--production"]);
    
    // 启动进程（CMD 窗口已自动隐藏）
    let child = cmd.spawn()?;
    
    Ok(child)
}

// McpProxy 启动
pub async fn start_mcp_proxy(config: &McpProxyConfig) -> Result<Child> {
    let cmd_path = "C:\\Users\\...\\mcp-proxy.cmd";
    
    let mut cmd = create_hidden_tokio_command(cmd_path);
    cmd.env("MCP_PORT", &config.port.to_string());
    
    let child = cmd.spawn()?;
    
    Ok(child)
}
```

---

### 方案 2: 增强版 - 带日志捕获 ✅ 强烈推荐

由于 CMD 窗口隐藏后无法看到错误信息，需要捕获子进程的 stdout/stderr：

**文件**: `nuwax-agent-core/src/utils/process.rs`

```rust
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command as TokioCommand};
use std::process::Stdio;
use anyhow::Result;

/// 启动服务并捕获输出日志
/// 
/// # 特性
/// - Windows: 自动隐藏 CMD 窗口
/// - 捕获 stdout/stderr 并记录到日志
/// - 返回子进程句柄
/// 
/// # 参数
/// - `cmd_path`: 可执行文件路径（如 `C:\...\mcp-proxy.cmd`）
/// - `service_name`: 服务名称（用于日志标识）
/// - `env_vars`: 环境变量 `Vec<(String, String)>`
/// - `args`: 命令行参数 `Vec<String>`
pub async fn start_service_with_logging(
    cmd_path: impl AsRef<std::ffi::OsStr>,
    service_name: &str,
    env_vars: Option<Vec<(String, String)>>,
    args: Option<Vec<String>>,
) -> Result<Child> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    
    let mut cmd = TokioCommand::new(&cmd_path);
    
    // 设置环境变量
    if let Some(envs) = env_vars {
        for (key, value) in envs {
            cmd.env(key, value);
        }
    }
    
    // 设置参数
    if let Some(args) = args {
        cmd.args(&args);
    }
    
    // 捕获 stdout/stderr
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    
    // Windows: 隐藏 CMD 窗口（必须在最后设置）
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    
    let mut child = cmd.spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn {}: {}", service_name, e))?;
    
    // 异步读取并记录 stdout
    if let Some(stdout) = child.stdout.take() {
        let service_name = service_name.to_string();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!("[{} stdout] {}", service_name, line);
            }
        });
    }
    
    // 异步读取并记录 stderr
    if let Some(stderr) = child.stderr.take() {
        let service_name = service_name.to_string();
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::error!("[{} stderr] {}", service_name, line);
            }
        });
    }
    
    tracing::info!("[{}] 进程已启动", service_name);
    
    Ok(child)
}
```

#### 使用示例

```rust
use crate::utils::process::start_service_with_logging;

// FileServer 启动
pub async fn start_file_server(config: &FileServerConfig) -> Result<Child> {
    let cmd_path = resolve_npm_bin("nuwax-file-server").await?;
    
    let env_vars = vec![
        ("PORT".to_string(), config.port.to_string()),
        ("NODE_ENV".to_string(), "production".to_string()),
        ("LOG_LEVEL".to_string(), "info".to_string()),
    ];
    
    let args = vec!["--production".to_string()];
    
    let child = start_service_with_logging(
        &cmd_path,
        "FileServer",
        Some(env_vars),
        Some(args),
    ).await?;
    
    // 健康检查
    wait_for_port_ready(config.port, Duration::from_secs(10)).await?;
    
    Ok(child)
}

// McpProxy 启动
pub async fn start_mcp_proxy(config: &McpProxyConfig) -> Result<Child> {
    let cmd_path = resolve_npm_bin("mcp-proxy").await?;
    
    let env_vars = vec![
        ("MCP_PORT".to_string(), config.port.to_string()),
    ];
    
    let child = start_service_with_logging(
        &cmd_path,
        "McpProxy",
        Some(env_vars),
        None,
    ).await?;
    
    // 健康检查
    let health_url = format!("http://127.0.0.1:{}/mcp", config.port);
    wait_for_http_ready(&health_url, Duration::from_secs(15)).await?;
    
    Ok(child)
}
```

---

### 方案 3: Node.js 检测逻辑改进 ✅ 必须

解决日志中出现的 Node.js 重复安装问题。

**文件**: `nuwax-agent-core/src/dependency/node.rs`

```rust
use std::process::Stdio;
use tokio::time::Duration;
use anyhow::{Result, Context};

/// 检查 Node.js 是否真正可用
pub async fn is_nodejs_available() -> bool {
    #[cfg(windows)]
    let node_cmd = "node.exe";
    #[cfg(not(windows))]
    let node_cmd = "node";
    
    match tokio::process::Command::new(node_cmd)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        Ok(status) if status.success() => {
            tracing::info!("Node.js is available in PATH");
            true
        }
        Ok(status) => {
            tracing::warn!("Node.js command exists but failed: {:?}", status);
            false
        }
        Err(e) => {
            tracing::debug!("Node.js not found in PATH: {}", e);
            false
        }
    }
}

/// 获取 Node.js 版本
pub async fn get_nodejs_version() -> Result<String> {
    #[cfg(windows)]
    let node_cmd = "node.exe";
    #[cfg(not(windows))]
    let node_cmd = "node";
    
    let output = tokio::process::Command::new(node_cmd)
        .arg("--version")
        .output()
        .await
        .context("Failed to get Node.js version")?;
    
    if !output.status.success() {
        anyhow::bail!("Node.js version check failed");
    }
    
    let version = String::from_utf8(output.stdout)
        .context("Invalid UTF-8 in Node.js version")?
        .trim()
        .to_string();
    
    Ok(version)
}

/// 检查 npm 包是否已全局安装
pub async fn is_npm_package_installed(package_name: &str) -> Result<bool> {
    if !is_nodejs_available().await {
        return Ok(false);
    }
    
    #[cfg(windows)]
    let npm_cmd = "npm.cmd";
    #[cfg(not(windows))]
    let npm_cmd = "npm";
    
    let output = tokio::process::Command::new(npm_cmd)
        .args(&["list", "-g", package_name, "--depth=0"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;
    
    Ok(output.success())
}

/// 确保 npm 包已安装（带完整的前置检查）
pub async fn ensure_npm_package(package_name: &str) -> Result<()> {
    tracing::info!("Ensuring npm package: {}", package_name);
    
    // 1. 检查 Node.js 是否可用
    if !is_nodejs_available().await {
        tracing::warn!("Node.js not available, attempting to install...");
        
        // 尝试安装 Node.js
        install_nodejs().await
            .context("Failed to install Node.js")?;
        
        // 再次验证
        if !is_nodejs_available().await {
            anyhow::bail!("Node.js installation succeeded but still not available");
        }
        
        // 显示安装的版本
        if let Ok(version) = get_nodejs_version().await {
            tracing::info!("Node.js {} is now available", version);
        }
    }
    
    // 2. 检查包是否已安装
    if is_npm_package_installed(package_name).await? {
        tracing::info!("{} is already installed (global)", package_name);
        return Ok(());
    }
    
    // 3. 安装包（带重试限制）
    let max_retries = 3;
    for attempt in 1..=max_retries {
        tracing::info!(
            "Installing {} (attempt {}/{})",
            package_name, attempt, max_retries
        );
        
        match install_npm_package_global(package_name).await {
            Ok(_) => {
                tracing::info!("{} installed successfully", package_name);
                
                // 验证安装
                tokio::time::sleep(Duration::from_secs(1)).await;
                if is_npm_package_installed(package_name).await? {
                    return Ok(());
                } else {
                    tracing::warn!("{} installed but verification failed", package_name);
                }
            }
            Err(e) if attempt < max_retries => {
                tracing::warn!(
                    "Install attempt {}/{} failed: {}", 
                    attempt, max_retries, e
                );
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                anyhow::bail!(
                    "Failed to install {} after {} attempts: {}", 
                    package_name, max_retries, e
                );
            }
        }
    }
    
    anyhow::bail!("Failed to verify {} installation", package_name)
}

/// 安装 npm 全局包
async fn install_npm_package_global(package_name: &str) -> Result<()> {
    #[cfg(windows)]
    let npm_cmd = "npm.cmd";
    #[cfg(not(windows))]
    let npm_cmd = "npm";
    
    let output = tokio::process::Command::new(npm_cmd)
        .args(&["install", "-g", package_name])
        .output()
        .await
        .context("Failed to run npm install")?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("npm install failed: {}", stderr);
    }
    
    Ok(())
}
```

---

## 实施步骤

### 阶段 1: 核心工具函数 (1 小时)

1. **创建 `utils/process.rs` 模块**
   ```bash
   # 在 nuwax-agent-core 中
   mkdir -p src/utils
   touch src/utils/process.rs
   touch src/utils/mod.rs
   ```

2. **实现 `start_service_with_logging` 函数**
   - 复制上面的代码
   - 添加到 `src/utils/process.rs`

3. **在 `lib.rs` 中导出**
   ```rust
   // src/lib.rs
   pub mod utils;
   ```

### 阶段 2: 修改服务启动代码 (2 小时)

4. **FileServer 服务**
   - 文件: `src/service/file_server.rs`
   - 找到: `Command::new(...)` 或 `TokioCommand::new(...)`
   - 替换为: `start_service_with_logging(...)`

5. **McpProxy 服务**
   - 文件: `src/service/mcp_proxy.rs`
   - 同样的修改

6. **其他 `.cmd` 服务**
   - 搜索所有 `Command::new` 并检查是否是 `.cmd` 文件
   - 统一使用工具函数

### 阶段 3: Node.js 检测改进 (1 小时)

7. **改进 Node.js 检测**
   - 文件: `src/dependency/node.rs`
   - 添加 `is_nodejs_available()` 函数
   - 修改 `ensure_npm_package()` 添加前置检查

### 阶段 4: 测试验证 (1 小时)

8. **Windows 测试**
   - 编译 nuwax-agent
   - 启动应用
   - 验证：
     - ✅ 无 CMD 窗口弹出
     - ✅ 日志中能看到服务的 stdout/stderr
     - ✅ mcp-proxy 健康检查成功
     - ✅ Node.js 不会重复安装

9. **macOS/Linux 测试**
   - 确保修改不影响其他平台

---

## 预期效果

### 修复前
```
用户启动 nuwax-agent
  ↓
[弹窗1] CMD 窗口 - nuwax-file-server.cmd
[弹窗2] CMD 窗口 - mcp-proxy.cmd
[弹窗3] CMD 窗口 - 其他服务...
  ↓
用户体验差 😢
```

### 修复后
```
用户启动 nuwax-agent
  ↓
所有服务静默启动（无 CMD 窗口）
  ↓
日志文件中可以看到所有服务输出
  ↓
用户体验好 ✅
```

### 日志示例

**修复后的日志**：
```
2026-02-12T06:27:06 INFO [FileServer] 进程已启动
2026-02-12T06:27:07 INFO [FileServer stdout] 服务运行在: http://localhost:60000
2026-02-12T06:27:08 INFO [FileServer stdout] 环境: production
2026-02-12T06:27:09 INFO [McpProxy] 进程已启动
2026-02-12T06:27:10 INFO [McpProxy stdout] MCP Proxy listening on 127.0.0.1:18099
2026-02-12T06:27:11 INFO [McpProxy stdout] Health check: OK
```

**如果服务启动失败**：
```
2026-02-12T06:27:10 INFO [McpProxy] 进程已启动
2026-02-12T06:27:10 ERROR [McpProxy stderr] Error: Cannot find module 'express'
2026-02-12T06:27:10 ERROR [McpProxy stderr]     at Function.Module._resolveFilename (node:internal/modules/cjs/loader:1048:15)
2026-02-12T06:27:11 ERROR [McpProxy] 健康检查失败: Connection refused
```
→ 现在可以清楚地看到错误原因！

---

## 参考资料

### Windows Process Creation Flags

- `CREATE_NO_WINDOW (0x08000000)`: 隐藏控制台窗口
- 文档: https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags

### Rust 标准库

- `std::os::windows::process::CommandExt`: https://doc.rust-lang.org/std/os/windows/process/trait.CommandExt.html

### mcp-proxy v0.1.39 修复

- Commit: `aa42573 - fix: hide CMD windows on Windows platform for Tauri integration`
- 文件: `mcp-streamable-proxy/src/server_builder.rs:230-241`

---

## 常见问题

### Q1: 为什么不能在创建 Command 时就设置 CREATE_NO_WINDOW？

A: 某些 Rust 库（如 `tokio::process::Command`）在内部可能会重新配置命令参数，导致早期设置的标志被覆盖。最佳实践是在所有配置（env, args, stdin/stdout/stderr）之后最后设置。

### Q2: 隐藏 CMD 窗口后如何调试启动失败？

A: 使用方案 2 的 `start_service_with_logging` 函数，它会捕获 stdout/stderr 并记录到日志文件中。

### Q3: 是否需要在 macOS/Linux 上做特殊处理？

A: 不需要。macOS/Linux 上启动 shell 脚本不会弹窗口，`create_hidden_command` 函数在非 Windows 平台上只是简单的 `Command::new()`。

### Q4: 如果用户已经安装了 Node.js，还会触发自动安装吗？

A: 修复后不会。改进的 `is_nodejs_available()` 函数会先检查 `node --version` 是否成功，只有真正不可用时才会触发安装。

---

## 总结

| 问题 | 现状 | 修复后 |
|------|------|--------|
| **CMD 窗口弹出** | ❌ 多个窗口弹出 | ✅ 完全隐藏 |
| **mcp-proxy 启动失败** | ❌ 15s 超时，看不到错误 | ✅ 日志中能看到详细错误 |
| **Node.js 重复安装** | ❌ 已安装仍尝试安装 10 次 | ✅ 正确检测，不重复安装 |
| **调试困难** | ❌ 窗口隐藏后无法调试 | ✅ 完整的 stdout/stderr 日志 |

**实施时间**: 约 5 小时（包括测试）  
**影响范围**: nuwax-agent Windows 版本  
**风险**: 低（只影响进程启动方式，不改变业务逻辑）
