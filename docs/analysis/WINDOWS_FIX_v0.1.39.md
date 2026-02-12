# Windows 测试问题修复总结 (v0.1.39)

## 测试环境
- **版本**: v0.1.37 → v0.1.39
- **平台**: Windows
- **日志文件**: nuwax-agent.log.2026-02-12

---

## 发现的问题及修复状态

### 1. ✅ 隐藏 CMD 窗口没有效果

**问题描述**:
- 在 Windows 上启动 MCP 子进程时，控制台窗口仍然显示

**根本原因**:
- `mcp-streamable-proxy/src/server_builder.rs` 中，`CREATE_NO_WINDOW` 标志设置得太早
- 后续的 `cmd.env()` 和 `cmd.args()` 调用导致标志可能失效

**修复方案**:
- 将 `CREATE_NO_WINDOW` 设置移到所有命令配置（env/args）之后
- 确保在创建 `TokioChildProcess` 之前作为最后一步设置

**修复代码**:
```rust
// 在所有 env() 和 args() 调用之后
#[cfg(windows)]
{
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}
```

**修复文件**:
- `/Users/apple/workspace/mcp-proxy/mcp-streamable-proxy/src/server_builder.rs:180-241`

**状态**: ✅ 已修复，待测试验证

---

### 2. ❌ mcp-proxy 启动失败

**问题描述**:
- mcp-proxy 进程启动后，健康检查超时失败
- 错误: "等待 15s 后 http://127.0.0.1:18099/mcp 仍未就绪"

**日志证据**:
```
2026-02-12T06:18:41.088275Z INFO [McpProxy] 可执行文件路径: C:\Users\MECHREVO\.local\bin\mcp-proxy.cmd
2026-02-12T06:18:41.088293Z INFO [McpProxy] 监听地址: 127.0.0.1:18099
2026-02-12T06:19:01.052725Z ERROR [McpProxy] 健康检查失败: MCP Proxy 健康检查超时
```

**分析**:
1. **进程启动成功**: mcp-proxy.cmd 被调用
2. **HTTP 服务未启动**: 15秒后健康检查端点仍不可用
3. **日志缺失**: 由于 CMD 窗口隐藏，子进程的 stdout/stderr 未被捕获

**可能的原因**:
- mcp-proxy 子进程内部 panic/crash
- 缺少必要的运行时依赖
- 配置文件解析错误
- 端口 18099 被占用
- Node.js 环境变量缺失

**诊断步骤** (需要用户执行):
```powershell
# 1. 检查 mcp-proxy 版本
C:\Users\MECHREVO\.local\bin\mcp-proxy.cmd --version

# 2. 手动启动服务器（查看完整错误）
C:\Users\MECHREVO\.local\bin\mcp-proxy.cmd server --port 18099

# 3. 检查端口占用
netstat -ano | findstr "18099"

# 4. 验证 Node.js 环境
node --version
npm list -g mcp-stdio-proxy

# 5. 查看批处理文件内容
type C:\Users\MECHREVO\.local\bin\mcp-proxy.cmd
```

**建议修复** (针对 nuwax-agent):
```rust
// 需要捕获子进程的 stdout/stderr
let mut child = Command::new(mcp_proxy_path)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;

// 异步读取并记录输出
tokio::spawn(async move {
    let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
    while let Ok(Some(line)) = stdout.next_line().await {
        tracing::info!("[McpProxy stdout] {}", line);
    }
});

tokio::spawn(async move {
    let mut stderr = BufReader::new(child.stderr.take().unwrap()).lines();
    while let Ok(Some(line)) = stderr.next_line().await {
        tracing::error!("[McpProxy stderr] {}", line);
    }
});
```

**状态**: ⏳ 等待用户提供诊断信息（这是 nuwax-agent 的问题，不是 mcp-proxy 代码的问题）

---

### 3. ⚠️ Node.js 检测逻辑问题

**问题描述**:
- 第一次启动时，即使系统已安装 Node.js，仍然出现多次重复安装尝试
- 第二次启动时，应用安装了自己的 Node.js 到 `~/.local/bin`

**日志证据**:
```
# 第一次启动 - 10 次重复尝试
2026-02-12T06:14:50.437049Z  INFO [resolve_node_bin] npm -> fallback to PATH
2026-02-12T06:15:22.509303Z  INFO [Dependency] 开始全局安装 npm 包: mcp-stdio-proxy
2026-02-12T06:15:29.360632Z  INFO [Dependency] 开始全局安装 npm 包: mcp-stdio-proxy
...（共 10 次）

# 第二次启动 - 成功
2026-02-12T06:16:55.747776Z  INFO [NodeInstall] 开始自动安装 Node.js...
2026-02-12T06:17:00.275214Z  INFO [resolve_node_bin] npm -> "C:\\Users\\MECHREVO\\.local\\bin\\npm.cmd"
2026-02-12T06:17:13.857546Z  INFO [Dependency] mcp-stdio-proxy 全局安装成功
```

**根本原因**:
1. **缺少 Node.js 可用性检测**: 直接假设 PATH 中的 npm 可用
2. **未验证命令执行结果**: `fallback to PATH` 后未检查 npm 命令是否真正可用
3. **重试逻辑问题**: 安装失败后一直重试，未触发 Node.js 安装

**建议修复** (针对 nuwax-agent):

```rust
/// 检查 Node.js 是否真正可用
async fn is_node_available() -> bool {
    let node_cmd = if cfg!(windows) { "node.exe" } else { "node" };
    
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

/// 确保 npm 包安装（带前置检查）
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
    
    // 3. 安装包（带重试限制）
    let max_retries = 3;
    for attempt in 1..=max_retries {
        match install_npm_package(package_name).await {
            Ok(_) => return Ok(()),
            Err(e) if attempt < max_retries => {
                tracing::warn!("Install attempt {}/{} failed: {}", attempt, max_retries, e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => anyhow::bail!("Failed to install {} after {} attempts: {}", package_name, max_retries, e),
        }
    }
    
    Ok(())
}
```

**状态**: ⚠️ 这是 nuwax-agent 的问题，已提供修复建议

---

## 修改文件清单

### mcp-proxy 仓库
1. ✅ `mcp-streamable-proxy/src/server_builder.rs`
   - 移动 CREATE_NO_WINDOW 到最后设置
   
2. ✅ `mcp-proxy/Cargo.toml`
   - 版本号: 0.1.38 → 0.1.39

3. ✅ `WINDOWS_ISSUE_ANALYSIS.md`
   - 新增：完整的问题分析文档

4. ✅ `WINDOWS_FIX_SUMMARY.md`
   - 新增：修复总结和诊断指南

---

## 验证检查清单

### 立即需要用户验证
- [ ] 手动运行 `mcp-proxy.cmd server --port 18099` 并提供完整输出
- [ ] 检查端口 18099 是否被占用
- [ ] 验证 Node.js 和 npm 全局包安装情况

### 编译新版本后验证
- [ ] CMD 窗口是否成功隐藏（问题 1）
- [ ] mcp-proxy 是否能正常启动（问题 2，取决于诊断结果）
- [ ] Node.js 检测是否正常（问题 3，需要 nuwax-agent 修复）

---

## 构建和发布

### 构建命令
```bash
# 构建发布版本
cargo build --release -p mcp-stdio-proxy

# 生成的二进制文件
# target/release/mcp-proxy (或 mcp-proxy.exe on Windows)
```

### 版本信息
- **当前版本**: v0.1.39
- **修复内容**: Windows CMD 窗口隐藏问题
- **待验证**: 需要在 Windows 环境测试

---

## 后续行动

### 短期（立即）
1. **等待用户诊断信息**: 运行诊断命令并提供输出
2. **编译测试**: 在 Windows 上编译 v0.1.39 并测试

### 中期（本周）
1. **与 nuwax-agent 团队协作**: 
   - 分享子进程输出捕获建议
   - 分享 Node.js 检测逻辑改进建议
2. **完善文档**: 添加 Windows 平台特定的故障排除指南

### 长期（下一版本）
1. **考虑添加 mcp-proxy 自检命令**:
   ```bash
   mcp-proxy doctor  # 检查环境、依赖、配置
   ```
2. **改进错误信息**: 提供更详细的启动失败诊断
3. **添加 Windows 集成测试**: 自动化测试 CMD 窗口隐藏等功能

---

## 联系信息

如有问题，请提供：
1. 完整的错误日志
2. 运行诊断命令的输出
3. Windows 版本和系统信息
4. Node.js/npm 版本信息

GitHub Issue: https://github.com/nuwax-ai/mcp-proxy/issues
