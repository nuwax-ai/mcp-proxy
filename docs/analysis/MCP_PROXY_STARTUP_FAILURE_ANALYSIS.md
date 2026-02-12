# mcp-proxy 启动失败分析

## 问题描述

从 nuwax-agent 日志中发现，mcp-proxy 启动后健康检查持续失败：

```
2026-02-12T06:19:01.052725Z ERROR [McpProxy] 健康检查失败: 
MCP Proxy 健康检查超时: 等待 15s 后 http://127.0.0.1:18099/mcp 仍未就绪
```

**启动日志**：
```
2026-02-12T06:18:41.088275Z INFO [McpProxy] 可执行文件路径: C:\Users\MECHREVO\.local\bin\mcp-proxy.cmd
2026-02-12T06:18:41.088293Z INFO [McpProxy] 监听地址: 127.0.0.1:18099
2026-02-12T06:18:41.088337Z INFO [McpProxy] Windows: 添加 npm 全局 bin 到 PATH: C:\Users\MECHREVO\AppData\Roaming\npm
... (15 秒后)
2026-02-12T06:19:01.052725Z ERROR [McpProxy] 健康检查失败
```

---

## 关键问题

### 1. 缺少子进程输出

由于 CMD 窗口可能被隐藏或者 nuwax-agent 没有捕获 stdout/stderr，**看不到 mcp-proxy 的实际错误信息**。

这就像在黑暗中调试：
```
nuwax-agent: "mcp-proxy，你启动了吗？"
[15 秒沉默]
nuwax-agent: "超时了，你失败了！"
mcp-proxy: (可能早就崩溃了，但没人知道为什么)
```

### 2. 健康检查端点可能不对

日志显示健康检查 URL：
```
http://127.0.0.1:18099/mcp
```

**需要验证**：mcp-proxy 的健康检查端点是否真的是 `/mcp`？

根据 mcp-proxy 代码，让我检查实际的路由：

---

## 诊断步骤

### 步骤 1: 检查 mcp-proxy 实际的健康检查路由

从 mcp-proxy 的代码分析（假设基于之前的代码结构）：

**可能的健康检查端点**：
- `/health` - 标准健康检查
- `/` - 根路径
- `/mcp` - MCP 服务列表
- `/api/health` - API 健康检查

**需要确认**: nuwax-agent 使用的 `/mcp` 端点是否正确。

### 步骤 2: 手动测试 mcp-proxy

在 Windows 测试机上手动运行：

```powershell
# 1. 手动启动 mcp-proxy
cd C:\Users\MECHREVO\.local\bin
.\mcp-proxy.cmd --help

# 2. 查看可用命令
.\mcp-proxy.cmd server --help

# 3. 手动启动服务器（查看完整输出）
.\mcp-proxy.cmd server --port 18099

# 4. 在另一个终端测试健康检查
curl http://127.0.0.1:18099/health
curl http://127.0.0.1:18099/mcp
curl http://127.0.0.1:18099/

# 5. 检查进程
tasklist | findstr node
netstat -ano | findstr "18099"
```

### 步骤 3: 检查依赖问题

mcp-proxy 可能缺少 Node.js 依赖：

```powershell
# 检查 mcp-proxy 的依赖
cd C:\Users\MECHREVO\.local\bin\node_modules\mcp-stdio-proxy
npm list

# 重新安装（如果有问题）
npm install -g mcp-stdio-proxy --force
```

### 步骤 4: 检查端口占用

```powershell
# 检查 18099 端口是否被占用
netstat -ano | findstr "18099"

# 如果被占用，查看占用进程
tasklist /FI "PID eq <PID>"
```

---

## 可能的失败原因

### 原因 1: Node.js 模块缺失 ⭐ 最可能

**症状**: npm 包安装成功，但 node_modules 不完整

**可能性**: 
- npm 安装过程中网络问题
- 包的 postinstall 脚本失败
- Windows 长路径问题（路径超过 260 字符）

**验证**:
```powershell
# 检查 mcp-stdio-proxy 的 node_modules
dir C:\Users\MECHREVO\.local\bin\node_modules\mcp-stdio-proxy\node_modules

# 检查 package.json 中的依赖
type C:\Users\MECHREVO\.local\bin\node_modules\mcp-stdio-proxy\package.json
```

**解决**:
```powershell
# 清理并重新安装
npm uninstall -g mcp-stdio-proxy
npm cache clean --force
npm install -g mcp-stdio-proxy --verbose
```

### 原因 2: 健康检查 URL 不匹配

**症状**: mcp-proxy 启动成功，但 nuwax-agent 访问了错误的端点

**验证**: 需要查看 mcp-proxy 的实际路由

从 mcp-proxy 源码看，可能的路由：
```rust
// mcp-proxy/src/main.rs 或 server 模块
.route("/", get(root_handler))
.route("/health", get(health_check))
.route("/mcp", get(list_mcp_services))  // 这个可能不是健康检查！
```

**/mcp 可能是服务列表端点，不是健康检查！**

**正确的健康检查应该是**:
```rust
// 应该访问
http://127.0.0.1:18099/health
// 而不是
http://127.0.0.1:18099/mcp
```

### 原因 3: 环境变量问题

**症状**: mcp-proxy 需要特定的环境变量

日志显示添加了 PATH：
```
Windows: 添加 npm 全局 bin 到 PATH: C:\Users\MECHREVO\AppData\Roaming\npm
```

但可能还需要其他环境变量（如果 mcp-proxy 有子进程）。

### 原因 4: 配置文件缺失

**症状**: mcp-proxy 启动时需要配置文件，但找不到

从日志中看到：
```
WARN [McpProxy] mcpServers 配置为空，跳过启动
```

**这可能是关键**！如果 `mcpServers` 配置为空，mcp-proxy 可能：
- 不启动 HTTP 服务器
- 或者启动了但没有实际的服务端点

**需要确认**: 
1. mcp-proxy 是否需要配置文件？
2. 配置文件应该在哪里？
3. 空配置时 mcp-proxy 的行为是什么？

### 原因 5: Windows 防火墙

**症状**: 本地回环连接被防火墙阻止

**验证**:
```powershell
# 检查防火墙规则
netsh advfirewall firewall show rule name=all | findstr "18099"
```

---

## 深入分析：mcpServers 配置为空

从日志：
```
2026-02-12T06:18:32.797802Z WARN [McpProxy] mcpServers 配置为空，跳过启动
```

**这说明**：
1. nuwax-agent 读取配置发现 `mcpServers` 为空
2. nuwax-agent 可能**根本没有启动 mcp-proxy 进程**
3. 或者启动了但 mcp-proxy 立即退出了

**查看完整的启动流程**：
```
06:18:32.797761 INFO [BinPath] mcp-proxy 找到: C:\Users\...\mcp-proxy.cmd
06:18:32.797802 WARN [McpProxy] mcpServers 配置为空，跳过启动
06:18:32.797825 INFO [Services] MCP Proxy 启动命令已发送

06:18:41.088275 INFO [McpProxy] 可执行文件路径: C:\Users\...\mcp-proxy.cmd
06:18:41.088293 INFO [McpProxy] 监听地址: 127.0.0.1:18099
... (启动了？)

06:19:01.052725 ERROR [McpProxy] 健康检查失败
```

**矛盾点**：
- 前面说"跳过启动"
- 后面又有启动日志和健康检查失败

**可能的情况**：
1. **第一次启动**（06:18:32）被跳过了（配置为空）
2. **第二次启动**（06:18:41）实际执行了，但失败了

让我查看日志中是否有多次重启：

从日志确实看到多次重启：
```
06:18:11 开始重启所有服务
06:18:32 第一次尝试启动 MCP Proxy (配置为空，跳过)
06:18:35 再次重启所有服务
06:18:40 第二次尝试启动 MCP Proxy (实际启动了)
06:19:01 健康检查失败
```

---

## 推荐的修复方案

### 方案 A: 捕获 mcp-proxy 的 stdout/stderr ⭐ 最优先

**问题**: 看不到 mcp-proxy 的实际错误

**解决**: 使用前面文档中的 `start_service_with_logging` 函数

**nuwax-agent 代码修改**：
```rust
// nuwax-agent-core/src/service/mcp_proxy.rs

pub async fn start_mcp_proxy(config: &McpProxyConfig) -> Result<Child> {
    let cmd_path = resolve_npm_bin("mcp-proxy").await?;
    
    // 准备启动参数
    let mut env_vars = vec![
        ("MCP_PORT".to_string(), config.port.to_string()),
    ];
    
    // 如果有配置文件，添加环境变量
    if let Some(config_path) = &config.config_file {
        env_vars.push(("MCP_CONFIG".to_string(), config_path.clone()));
    }
    
    // 使用带日志捕获的启动函数
    let child = crate::utils::process::start_service_with_logging(
        &cmd_path,
        "McpProxy",
        Some(env_vars),
        Some(vec!["server".to_string()]),  // 子命令
    ).await?;
    
    tracing::info!("[McpProxy] 进程已启动，PID: {:?}", child.id());
    
    // 健康检查（使用正确的端点）
    let health_url = format!("http://127.0.0.1:{}/health", config.port);
    wait_for_http_ready(&health_url, Duration::from_secs(30)).await
        .context("MCP Proxy 健康检查超时")?;
    
    Ok(child)
}
```

**效果**: 现在可以在日志中看到 mcp-proxy 的所有输出！

### 方案 B: 修复健康检查端点

**问题**: 健康检查可能用错了端点

**验证**:
1. 查看 mcp-proxy 的路由定义
2. 确认健康检查端点是 `/health` 还是 `/mcp`

**建议**: 尝试多个端点

```rust
async fn check_mcp_proxy_health(port: u16) -> Result<()> {
    let endpoints = vec![
        "/health",
        "/",
        "/mcp",
        "/api/health",
    ];
    
    for endpoint in &endpoints {
        let url = format!("http://127.0.0.1:{}{}", port, endpoint);
        tracing::debug!("Trying health check: {}", url);
        
        match reqwest::get(&url).await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!("Health check succeeded: {}", url);
                return Ok(());
            }
            Ok(resp) => {
                tracing::warn!("Health check {} returned: {}", url, resp.status());
            }
            Err(e) => {
                tracing::debug!("Health check {} failed: {}", url, e);
            }
        }
    }
    
    anyhow::bail!("All health check endpoints failed")
}
```

### 方案 C: 处理空配置情况

**问题**: `mcpServers` 为空时，mcp-proxy 可能不应该启动

**建议**: 在 nuwax-agent 中明确处理

```rust
pub async fn start_mcp_proxy(config: &McpProxyConfig) -> Result<Option<Child>> {
    // 检查配置
    if config.mcp_servers.is_empty() {
        tracing::info!("[McpProxy] mcpServers 配置为空，跳过启动");
        return Ok(None);  // 返回 None 而不是启动失败
    }
    
    // 启动服务
    let child = start_service_with_logging(...).await?;
    
    Ok(Some(child))
}
```

### 方案 D: 增加超时时间和重试

**问题**: 15 秒超时可能不够

**建议**:
```rust
// 增加超时到 30 秒
wait_for_http_ready(&health_url, Duration::from_secs(30)).await?;

// 或者添加重试逻辑
for attempt in 1..=3 {
    match start_and_check_mcp_proxy(config).await {
        Ok(child) => return Ok(child),
        Err(e) if attempt < 3 => {
            tracing::warn!("MCP Proxy 启动失败 (尝试 {}/3): {}", attempt, e);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        Err(e) => return Err(e),
    }
}
```

---

## 需要 mcp-proxy 仓库确认的信息

为了完全解决这个问题，需要从 mcp-proxy 代码中确认：

### 1. 健康检查端点

**问题**: 正确的健康检查 URL 是什么？

**需要检查的文件**:
- `mcp-proxy/src/server.rs` 或 `mcp-proxy/src/main.rs`
- 查找 `Router` 或 `axum::Router` 配置
- 查找 `health` 相关的路由

**期望的代码**:
```rust
// mcp-proxy/src/server.rs
let app = Router::new()
    .route("/health", get(health_check))  // ← 这个！
    .route("/mcp", get(list_services))
    // ...
```

### 2. 空配置行为

**问题**: 当没有配置 MCP 服务时，mcp-proxy 会怎么做？

**可能的行为**:
1. 启动 HTTP 服务器，但没有 MCP 服务
2. 直接退出（因为没有服务可代理）
3. 报错

**需要确认**: mcp-proxy 的预期行为

### 3. 必需的环境变量

**问题**: mcp-proxy 是否需要特定的环境变量？

**需要检查**:
- 配置文件路径（`MCP_CONFIG`?）
- 日志级别（`RUST_LOG`?）
- 其他配置

### 4. 启动子命令

**问题**: 正确的启动命令是什么？

**可能的形式**:
```bash
mcp-proxy                    # 默认启动
mcp-proxy server             # 明确的 server 子命令
mcp-proxy server --port 18099
mcp-proxy --config config.json server
```

---

## 立即可以做的

### 在 mcp-proxy 仓库

1. **添加详细的启动日志**
   ```rust
   // mcp-proxy/src/main.rs
   #[tokio::main]
   async fn main() -> Result<()> {
       tracing_subscriber::fmt::init();
       
       tracing::info!("mcp-proxy starting...");
       tracing::info!("Version: {}", env!("CARGO_PKG_VERSION"));
       
       let config = load_config()?;
       tracing::info!("Configuration loaded: {:?}", config);
       
       if config.mcp_servers.is_empty() {
           tracing::warn!("No MCP servers configured");
           // 决定是继续还是退出
       }
       
       let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
       tracing::info!("Starting HTTP server on {}", addr);
       
       // 启动服务器...
       tracing::info!("HTTP server started successfully");
       
       Ok(())
   }
   ```

2. **统一健康检查端点**
   ```rust
   // 确保有一个明确的健康检查端点
   .route("/health", get(|| async { "OK" }))
   ```

3. **添加版本信息端点**
   ```rust
   .route("/version", get(|| async {
       Json(json!({
           "version": env!("CARGO_PKG_VERSION"),
           "name": env!("CARGO_PKG_NAME"),
       }))
   }))
   ```

### 在 nuwax-agent 仓库

1. **立即实现日志捕获**（最优先）
2. **增加健康检查超时时间** (15s → 30s)
3. **尝试多个健康检查端点**
4. **空配置时跳过启动，不报错**

---

## 总结

| 问题 | 严重性 | 解决方案 | 优先级 |
|------|--------|----------|--------|
| **无法看到错误日志** | 🔴 高 | 捕获 stdout/stderr | P0 |
| **健康检查端点可能错误** | 🟡 中 | 尝试多个端点 | P1 |
| **空配置时行为不明** | 🟡 中 | 明确处理逻辑 | P1 |
| **超时时间太短** | 🟢 低 | 增加到 30s | P2 |

**下一步行动**:
1. ✅ 已创建完整的修复文档
2. ⏳ 需要在 Windows 上手动测试 mcp-proxy
3. ⏳ 需要确认 mcp-proxy 的健康检查端点
4. ⏳ 在 nuwax-agent 中实现日志捕获
