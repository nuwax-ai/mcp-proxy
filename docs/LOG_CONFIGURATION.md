# mcp-proxy 日志配置指南

## 概述

mcp-proxy 支持灵活的日志配置，可以通过配置文件或环境变量进行配置，非常适合与 Tauri 等客户端集成。

## 日志功能特性

- ✅ **按日期滚动**：每天自动创建新的日志文件
- ✅ **自动清理**：保留最近 N 天的日志文件
- ✅ **多级别过滤**：支持 trace/debug/info/warn/error
- ✅ **双输出**：同时输出到控制台和文件
- ✅ **结构化日志**：使用 tracing 框架
- ✅ **环境变量覆盖**：支持运行时动态配置

## 配置方式

### 方式 1: 环境变量（推荐用于 Tauri 集成）

环境变量具有最高优先级，会覆盖配置文件中的设置。

```bash
# 设置日志目录（Tauri 可以传递应用数据目录）
export MCP_PROXY_LOG_DIR="/path/to/logs"

# 设置日志级别
export MCP_PROXY_LOG_LEVEL="info"

# 设置服务器端口
export MCP_PROXY_PORT="18099"
```

**Windows (PowerShell)**:
```powershell
$env:MCP_PROXY_LOG_DIR = "C:\Users\YourName\AppData\Roaming\YourApp\logs"
$env:MCP_PROXY_LOG_LEVEL = "info"
$env:MCP_PROXY_PORT = "18099"
```

**Windows (CMD)**:
```cmd
set MCP_PROXY_LOG_DIR=C:\Users\YourName\AppData\Roaming\YourApp\logs
set MCP_PROXY_LOG_LEVEL=info
set MCP_PROXY_PORT=18099
```

### 方式 2: 配置文件

创建 `config.yml` 文件：

```yaml
server:
  port: 18099

log:
  level: "info"
  path: "./logs"
  retain_days: 7
```

配置文件查找顺序：
1. `/app/config.yml` (Docker 容器内)
2. `./config.yml` (当前工作目录)
3. 环境变量 `BOT_SERVER_CONFIG` 指定的路径

## Tauri 集成示例

### Rust 端 (nuwax-agent)

```rust
use std::process::Command;
use std::path::PathBuf;
use tauri::api::path::app_data_dir;

pub async fn start_mcp_proxy(
    tauri_config: &tauri::Config,
    port: u16,
) -> Result<Child> {
    // 获取 Tauri 应用数据目录
    let app_data = app_data_dir(tauri_config)
        .ok_or_else(|| anyhow::anyhow!("无法获取应用数据目录"))?;
    
    // 创建日志目录
    let log_dir = app_data.join("logs").join("mcp-proxy");
    std::fs::create_dir_all(&log_dir)?;
    
    let log_dir_str = log_dir.to_string_lossy().to_string();
    
    tracing::info!("MCP Proxy 日志目录: {}", log_dir_str);
    
    // 查找 mcp-proxy 可执行文件
    let mcp_proxy_path = resolve_npm_bin("mcp-proxy").await?;
    
    // 启动进程，传递环境变量
    let mut cmd = tokio::process::Command::new(&mcp_proxy_path);
    
    cmd.env("MCP_PROXY_LOG_DIR", &log_dir_str);
    cmd.env("MCP_PROXY_LOG_LEVEL", "info");
    cmd.env("MCP_PROXY_PORT", port.to_string());
    
    // Windows: 隐藏 CMD 窗口
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    
    // 捕获 stdout/stderr 用于调试
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    
    let mut child = cmd.spawn()?;
    
    // 读取并记录输出
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!("[mcp-proxy stdout] {}", line);
            }
        });
    }
    
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::error!("[mcp-proxy stderr] {}", line);
            }
        });
    }
    
    Ok(child)
}
```

### TypeScript/JavaScript 端

```typescript
import { Command } from '@tauri-apps/api/shell';
import { appDataDir, join } from '@tauri-apps/api/path';

async function startMcpProxy(port: number = 18099) {
  // 获取应用数据目录
  const appData = await appDataDir();
  const logDir = await join(appData, 'logs', 'mcp-proxy');
  
  console.log(`MCP Proxy 日志目录: ${logDir}`);
  
  // 创建命令
  const command = new Command('mcp-proxy', [], {
    env: {
      MCP_PROXY_LOG_DIR: logDir,
      MCP_PROXY_LOG_LEVEL: 'info',
      MCP_PROXY_PORT: port.toString(),
    }
  });
  
  // 监听输出
  command.on('output', (data) => {
    console.log('[mcp-proxy]', data);
  });
  
  command.on('error', (error) => {
    console.error('[mcp-proxy error]', error);
  });
  
  // 执行命令
  const child = await command.spawn();
  
  console.log(`MCP Proxy 已启动，PID: ${child.pid}`);
  
  return child;
}
```

## 日志文件格式

### 文件命名规则

```
日志目录/
├── log.2026-02-12      # 2026年2月12日的日志
├── log.2026-02-13      # 2026年2月13日的日志
└── log.2026-02-14      # 2026年2月14日的日志（当前）
```

**自动清理**：默认保留最近 5 天的日志，可通过 `retain_days` 配置。

### 日志内容示例

```
2026-02-12T14:30:15.123456Z  INFO mcp_proxy: ========================================
2026-02-12T14:30:15.123789Z  INFO mcp_proxy: MCP-Proxy 服务启动
2026-02-12T14:30:15.124012Z  INFO mcp_proxy: 命令: proxy (HTTP 服务器模式)
2026-02-12T14:30:15.124234Z  INFO mcp_proxy: 版本: 0.1.39
2026-02-12T14:30:15.124456Z  INFO mcp_proxy: 配置信息:
2026-02-12T14:30:15.124678Z  INFO mcp_proxy:   - 监听端口: 18099
2026-02-12T14:30:15.124890Z  INFO mcp_proxy:   - 日志目录: /path/to/logs
2026-02-12T14:30:15.125012Z  INFO mcp_proxy:   - 日志级别: info
2026-02-12T14:30:15.125234Z  INFO mcp_proxy:   - 日志保留: 7 天
2026-02-12T14:30:15.125456Z  INFO mcp_proxy: 环境变量覆盖:
2026-02-12T14:30:15.125678Z  INFO mcp_proxy:   - MCP_PROXY_LOG_DIR: /custom/log/path
2026-02-12T14:30:15.125890Z  INFO mcp_proxy: ========================================
2026-02-12T14:30:15.234567Z  INFO mcp_proxy: 尝试绑定到地址: 0.0.0.0:18099
2026-02-12T14:30:15.345678Z  INFO mcp_proxy: 成功绑定到地址: 0.0.0.0:18099
2026-02-12T14:30:15.456789Z  INFO mcp_proxy: 初始化应用状态...
2026-02-12T14:30:15.567890Z  INFO mcp_proxy: 应用状态初始化完成
2026-02-12T14:30:15.678901Z  INFO mcp_proxy: 初始化路由...
2026-02-12T14:30:15.789012Z  INFO mcp_proxy: 路由初始化完成
2026-02-12T14:30:15.890123Z  INFO mcp_proxy: ✅ 服务启动成功，监听地址: 0.0.0.0:18099
2026-02-12T14:30:15.901234Z  INFO mcp_proxy: ✅ 健康检查端点: http://0.0.0.0:18099/health
2026-02-12T14:30:15.912345Z  INFO mcp_proxy: ✅ MCP 服务列表: http://0.0.0.0:18099/mcp
2026-02-12T14:30:16.023456Z  INFO mcp_proxy: ✅ MCP服务状态检查定时任务已启动
2026-02-12T14:30:16.134567Z  INFO mcp_proxy: 系统信息:
2026-02-12T14:30:16.145678Z  INFO mcp_proxy:   - 操作系统: windows
2026-02-12T14:30:16.156789Z  INFO mcp_proxy:   - 架构: x86_64
2026-02-12T14:30:16.167890Z  INFO mcp_proxy:   - 工作目录: "C:\\Users\\YourName\\AppData\\Local"
2026-02-12T14:30:16.178901Z  INFO mcp_proxy: 🚀 HTTP 服务器启动，等待连接...
```

## 日志级别说明

| 级别 | 用途 | 建议场景 |
|------|------|---------|
| `trace` | 最详细的调试信息 | 开发调试 |
| `debug` | 调试信息 | 开发和测试 |
| `info` | 一般信息 | **生产环境推荐** |
| `warn` | 警告信息 | 最小日志 |
| `error` | 错误信息 | 仅记录错误 |

## 启动时的 stderr 输出

即使日志写入文件，mcp-proxy 也会在启动时向 **stderr** 输出关键信息：

```
========================================
MCP-Proxy 启动中...
版本: 0.1.39
配置加载完成:
  - 端口: 18099
  - 日志目录: /path/to/logs
  - 日志级别: info
  - 日志保留天数: 7
========================================
```

这确保即使日志系统初始化失败，也能看到基本的启动信息。

## 故障排查

### 问题 1: 日志文件未创建

**可能原因**：
- 日志目录路径不存在或无权限
- 环境变量设置错误

**解决方案**：
```bash
# 检查日志目录是否存在
ls -la /path/to/logs

# 检查环境变量
echo $MCP_PROXY_LOG_DIR

# 手动创建目录
mkdir -p /path/to/logs
chmod 755 /path/to/logs
```

### 问题 2: 看不到日志输出

**可能原因**：
- 日志级别设置过高（如 `error`）
- Windows 下 CMD 窗口被隐藏

**解决方案**：
```bash
# 降低日志级别
export MCP_PROXY_LOG_LEVEL="debug"

# 或使用 RUST_LOG 环境变量
export RUST_LOG="mcp_proxy=debug"
```

### 问题 3: 日志文件过多

**可能原因**：
- `retain_days` 设置过大

**解决方案**：
```bash
# 设置保留天数（通过配置文件）
# config.yml
log:
  retain_days: 3  # 只保留 3 天

# 或手动清理旧日志
find /path/to/logs -name "log.*" -mtime +7 -delete
```

## 完整示例：nuwax-agent 集成

```rust
// nuwax-agent-core/src/service/mcp_proxy.rs

use anyhow::{Context, Result};
use tokio::process::Child;
use std::path::PathBuf;

pub struct McpProxyConfig {
    pub port: u16,
    pub log_dir: PathBuf,
    pub log_level: String,
}

pub async fn start_mcp_proxy(config: McpProxyConfig) -> Result<Child> {
    // 确保日志目录存在
    std::fs::create_dir_all(&config.log_dir)
        .context("创建 MCP Proxy 日志目录失败")?;
    
    tracing::info!("启动 MCP Proxy:");
    tracing::info!("  - 端口: {}", config.port);
    tracing::info!("  - 日志目录: {}", config.log_dir.display());
    tracing::info!("  - 日志级别: {}", config.log_level);
    
    // 查找可执行文件
    let mcp_proxy_path = which::which("mcp-proxy")
        .or_else(|_| {
            // Fallback: 尝试 npm 全局路径
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))?;
            
            #[cfg(windows)]
            let npm_bin = PathBuf::from(&home).join(".local/bin/mcp-proxy.cmd");
            
            #[cfg(not(windows))]
            let npm_bin = PathBuf::from(&home).join(".local/bin/mcp-proxy");
            
            if npm_bin.exists() {
                Ok(npm_bin)
            } else {
                Err(which::Error::CannotFindBinaryPath)
            }
        })
        .context("找不到 mcp-proxy 可执行文件")?;
    
    tracing::info!("mcp-proxy 路径: {}", mcp_proxy_path.display());
    
    // 构建命令
    let mut cmd = tokio::process::Command::new(&mcp_proxy_path);
    
    // 设置环境变量
    cmd.env("MCP_PROXY_LOG_DIR", config.log_dir.to_string_lossy().as_ref());
    cmd.env("MCP_PROXY_LOG_LEVEL", &config.log_level);
    cmd.env("MCP_PROXY_PORT", config.port.to_string());
    
    // 捕获输出
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    
    // Windows: 隐藏 CMD 窗口
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    
    // 启动进程
    let mut child = cmd.spawn()
        .context("启动 mcp-proxy 进程失败")?;
    
    // 异步读取 stdout
    if let Some(stdout) = child.stdout.take() {
        use tokio::io::{AsyncBufReadExt, BufReader};
        
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!("[mcp-proxy] {}", line);
            }
        });
    }
    
    // 异步读取 stderr
    if let Some(stderr) = child.stderr.take() {
        use tokio::io::{AsyncBufReadExt, BufReader};
        
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::error!("[mcp-proxy stderr] {}", line);
            }
        });
    }
    
    tracing::info!("MCP Proxy 进程已启动，PID: {:?}", child.id());
    
    Ok(child)
}
```

## 环境变量参考

| 环境变量 | 类型 | 默认值 | 说明 |
|---------|------|--------|------|
| `MCP_PROXY_PORT` | u16 | `3000` | HTTP 服务监听端口 |
| `MCP_PROXY_LOG_DIR` | String | `./logs` | 日志文件目录 |
| `MCP_PROXY_LOG_LEVEL` | String | `info` | 日志级别 (trace/debug/info/warn/error) |
| `RUST_LOG` | String | - | Rust 标准日志环境变量（更细粒度控制） |
| `BOT_SERVER_CONFIG` | String | - | 自定义配置文件路径 |

## 高级用法：RUST_LOG

`RUST_LOG` 环境变量提供更细粒度的日志控制：

```bash
# 只显示 mcp_proxy 模块的 debug 级别日志
export RUST_LOG="mcp_proxy=debug"

# 显示多个模块的日志
export RUST_LOG="mcp_proxy=debug,tokio=info,hyper=warn"

# 显示所有模块的 trace 级别日志（非常详细）
export RUST_LOG="trace"

# 组合使用
export RUST_LOG="debug,mcp_proxy=trace,hyper::proto=error"
```

**注意**：`RUST_LOG` 优先级高于 `MCP_PROXY_LOG_LEVEL`。

## 总结

- ✅ 使用 `MCP_PROXY_LOG_DIR` 环境变量指定日志目录（Tauri 集成推荐）
- ✅ 日志文件按日期自动滚动，默认保留 5 天
- ✅ 关键启动信息会输出到 stderr，即使日志系统失败也能看到
- ✅ 支持通过环境变量动态配置，无需修改配置文件
- ✅ Windows 下可以隐藏 CMD 窗口，但仍然捕获日志输出

更多信息请参考：
- [Tracing 文档](https://docs.rs/tracing)
- [Tracing Subscriber 文档](https://docs.rs/tracing-subscriber)
