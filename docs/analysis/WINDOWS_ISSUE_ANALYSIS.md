# Windows 测试问题完整分析与修复方案

## 日志分析结果

基于 `nuwax-agent.log.2026-02-12` 的分析，我发现了以下关键问题：

### 问题 1: ✅ 隐藏 CMD 窗口 - 已修复
**状态**: 已在 `mcp-streamable-proxy` 中修复

### 问题 2: ❌ mcp-proxy 启动失败

**错误日志**:
```
2026-02-12T06:19:01.052725Z ERROR nuwax_agent_core::service: 
[McpProxy] 健康检查失败: MCP Proxy 健康检查超时: 等待 15s 后 http://127.0.0.1:18099/mcp 仍未就绪
```

**详细分析**:

1. **mcp-proxy 进程启动成功**:
   ```
   2026-02-12T06:18:41.088275Z INFO nuwax_agent_core::service: [McpProxy] 可执行文件路径: C:\Users\MECHREVO\.local\bin\mcp-proxy.cmd
   2026-02-12T06:18:41.088293Z INFO nuwax_agent_core::service: [McpProxy] 监听地址: 127.0.0.1:18099
   ```

2. **但是 HTTP 健康检查失败**:
   - 15秒后 `http://127.0.0.1:18099/mcp` 仍然无法访问
   - 这说明 mcp-proxy 进程虽然启动了，但 HTTP 服务器没有正常运行

**可能的原因**:

1. **mcp-proxy 自身的启动错误** (最可能):
   - 子进程可能有 panic/crash
   - 缺少必要的依赖
   - 配置文件解析错误
   - 端口被占用

2. **日志输出被隐藏**:
   - 由于使用了 CMD 窗口隐藏，mcp-proxy 的 stderr/stdout 可能没有被正确捕获
   - Tauri 应用需要配置子进程的输出重定向

3. **Node.js 环境问题**:
   - mcp-proxy.cmd 是一个 Windows 批处理文件，依赖 Node.js
   - 虽然 PATH 中有 Node.js，但可能存在其他环境变量缺失

**诊断步骤**:

```powershell
# 1. 手动测试 mcp-proxy 是否能运行
C:\Users\MECHREVO\.local\bin\mcp-proxy.cmd --version

# 2. 尝试手动启动服务器
C:\Users\MECHREVO\.local\bin\mcp-proxy.cmd server

# 3. 检查端口占用
netstat -ano | findstr "18099"

# 4. 查看 Node.js 全局包安装情况
npm list -g mcp-stdio-proxy

# 5. 检查 mcp-proxy.cmd 内容
type C:\Users\MECHREVO\.local\bin\mcp-proxy.cmd
```

### 问题 3: ⚠️ Node.js 安装检测逻辑问题

**观察到的行为**:

```
# 第一次启动 - Node.js 未安装
2026-02-12T06:14:50.437049Z  INFO agent_tauri_client_lib: [resolve_node_bin] npm -> fallback to PATH

# 多次重复尝试安装 mcp-stdio-proxy（说明检测失败）
2026-02-12T06:15:22.509303Z  INFO agent_tauri_client_lib: [Dependency] 开始全局安装 npm 包: mcp-stdio-proxy
2026-02-12T06:15:29.360632Z  INFO agent_tauri_client_lib: [Dependency] 开始全局安装 npm 包: mcp-stdio-proxy
2026-02-12T06:15:30.769366Z  INFO agent_tauri_client_lib: [Dependency] 开始全局安装 npm 包: mcp-stdio-proxy
...（共 10 次重复）

# 第二次启动 - Node.js 已安装
2026-02-12T06:16:55.747776Z  INFO agent_tauri_client_lib: [NodeInstall] 开始自动安装 Node.js...
2026-02-12T06:16:59.168654Z  INFO nuwax_agent_core::dependency::node: Found local Node.js: v22.14.0
2026-02-12T06:17:00.275214Z  INFO agent_tauri_client_lib: [resolve_node_bin] npm -> "C:\\Users\\MECHREVO\\.local\\bin\\npm.cmd"
2026-02-12T06:17:13.857546Z  INFO agent_tauri_client_lib: [Dependency] mcp-stdio-proxy 全局安装成功
```

**问题分析**:

1. **第一次启动时** (Node.js 未安装):
   - 程序没有先安装 Node.js
   - 直接尝试使用 PATH 中的 npm（可能来自系统已安装的 Node.js）
   - 但是这个 npm 可能不可用或有问题，导致安装失败
   - 触发了多次重试（10 次）

2. **第二次启动时** (Node.js 已通过应用安装):
   - 检测到 `~/.local/bin/node` 存在
   - 使用正确的 npm 路径
   - 安装成功

**根本原因**:
- **缺少 Node.js 可用性检测**: 应该在使用 npm 之前，先检查 `node` 命令是否真正可用
- **检测逻辑不完整**: `fallback to PATH` 意味着直接使用系统 PATH 中的 npm，但没有验证它是否能正常工作

## 修复方案

### 1. mcp-proxy 启动失败的修复

这个问题不在 `mcp-proxy` 代码库中，而在调用方（nuwax-agent）。但我们可以提供诊断指导：

**建议给 nuwax-agent 团队**:

```rust
// 在启动 mcp-proxy 时，需要捕获子进程的 stdout/stderr
use std::process::{Command, Stdio};

let mut child = Command::new(mcp_proxy_path)
    .args(&["server", "--port", "18099"])
    .stdout(Stdio::piped())  // 捕获标准输出
    .stderr(Stdio::piped())  // 捕获标准错误
    .spawn()?;

// 读取并记录输出
let stdout = child.stdout.take().unwrap();
let stderr = child.stderr.take().unwrap();

// 使用异步任务读取输出
tokio::spawn(async move {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut reader = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        tracing::info!("[McpProxy stdout] {}", line);
    }
});

tokio::spawn(async move {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut reader = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        tracing::error!("[McpProxy stderr] {}", line);
    }
});
```

**临时解决方案（给用户）**:

```powershell
# 手动启动 mcp-proxy 来查看错误信息
cd C:\Users\MECHREVO\.local\bin
.\mcp-proxy.cmd server --port 18099

# 如果报错，记录错误信息
```

### 2. Node.js 检测逻辑增强

虽然这个逻辑在 nuwax-agent 中，但我们可以提供参考实现：

```rust
/// 检查 Node.js 是否真正可用
async fn is_node_available() -> bool {
    #[cfg(windows)]
    let node_cmd = "node.exe";
    #[cfg(not(windows))]
    let node_cmd = "node";
    
    match tokio::process::Command::new(node_cmd)
        .arg("--version")
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            tracing::info!("Found Node.js: {}", version.trim());
            true
        }
        Ok(_) => {
            tracing::warn!("Node.js command exists but failed");
            false
        }
        Err(e) => {
            tracing::warn!("Node.js not found: {}", e);
            false
        }
    }
}

/// 修正后的 npm 安装流程
async fn ensure_npm_package(package_name: &str) -> Result<()> {
    // 1. 先检查 Node.js 是否可用
    if !is_node_available().await {
        tracing::info!("Node.js not available, installing...");
        install_nodejs().await?;
        
        // 再次验证
        if !is_node_available().await {
            anyhow::bail!("Failed to install Node.js");
        }
    }
    
    // 2. 检查包是否已安装
    if is_package_installed(package_name).await? {
        tracing::info!("{} is already installed", package_name);
        return Ok(());
    }
    
    // 3. 安装包
    install_npm_package(package_name).await?;
    
    Ok(())
}
```

### 3. mcp-streamable-proxy CREATE_NO_WINDOW 修复

**已修复**: 将 `CREATE_NO_WINDOW` 标志移到所有命令配置的最后。

### 4. mcp-sse-proxy CREATE_NO_WINDOW 验证

**当前状态**: 使用 `CreationFlags(0x08000000)` + `JobObject`，应该是正确的。

需要验证：
```rust
#[cfg(windows)]
{
    use process_wrap::tokio::CreationFlags;
    wrapped_cmd.wrap(CreationFlags(0x08000000));
    wrapped_cmd.wrap(JobObject);
}
```

## 总结

### 问题优先级

1. **高优先级 - mcp-proxy 启动失败**:
   - 需要 nuwax-agent 团队捕获子进程输出
   - 需要用户手动运行 mcp-proxy 来诊断具体错误

2. **中优先级 - Node.js 检测逻辑**:
   - 需要在 nuwax-agent 中添加 Node.js 可用性检测
   - 避免重复安装尝试

3. **低优先级 - CMD 窗口隐藏**:
   - mcp-streamable-proxy 已修复
   - 需要编译新版本并测试

### 下一步行动

1. **立即诊断** - 用户手动运行:
   ```powershell
   C:\Users\MECHREVO\.local\bin\mcp-proxy.cmd server --port 18099
   ```

2. **提供完整错误日志**: 运行上述命令并提供输出

3. **编译测试新版本**: 测试 CREATE_NO_WINDOW 修复效果

4. **联系 nuwax-agent 团队**: 提供子进程输出捕获和 Node.js 检测的修复建议
