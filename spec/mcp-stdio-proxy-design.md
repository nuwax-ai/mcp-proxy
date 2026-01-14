# MCP-Stdio-Proxy 独立 CLI 工具设计

## 项目概述

创建一个**独立的命令行工具**，将远程的 MCP 服务（SSE/Streamable HTTP 协议）转换为本地 stdio 协议，供 AI Agent 直接使用。

## 核心定位

- **独立工具**：与现有 mcp-proxy HTTP 服务完全分离
- **单一功能**：只做 URL → stdio 的协议转换
- **即插即用**：stdin 输入，stdout 输出，无状态运行
- **零配置**：合理的默认配置，最少参数

## 使用场景

```
远程 MCP 服务 (SSE/Streamable HTTP) → [mcp-stdio-proxy] → stdio → AI Agent
```

## 命令设计

### 基础用法
```bash
# 最简单的形式 - 自动检测协议
mcp-stdio-proxy https://api.github.com/mcp

# 带认证
mcp-stdio-proxy https://api.github.com/mcp --auth "Bearer ghp_token"

# 完整参数
mcp-stdio-proxy <URL> [OPTIONS]
```

### 命令结构
```bash
mcp-stdio-proxy --help

Usage: mcp-stdio-proxy <URL> [OPTIONS]

Arguments:
  <URL>  MCP 服务的 URL 地址 (支持 SSE 和 Streamable HTTP)

Options:
  -a, --auth <HEADER>     认证 header (如: "Bearer token")
  -H, --header <K=V>      自定义 HTTP headers
  -t, --timeout <SECS>    超时时间 [default: 30]
  -r, --retries <COUNT>   重试次数 [default: 3]
  -v, --verbose           详细输出
  -q, --quiet            静默模式
  -h, --help             帮助信息
  -V, --version          版本信息
```

## 工作流程

### 1. 启动和初始化
```
输入: mcp-stdio-proxy https://api.github.com/mcp --auth "Bearer token"

步骤:
1. 解析命令行参数
2. 创建 HTTP 客户端
3. 检测 MCP 协议类型 (SSE/Streamable)
4. 建立连接
5. 进入代理循环
```

### 2. 运行时循环
```
循环:
├─ 读取 stdin (JSON-RPC 请求)
├─ 转发到远程 MCP 服务
├─ 接收响应
├─ 写入 stdout (JSON-RPC 响应)
└─ 处理服务器事件 (SSE 特有)
```

### 3. 错误处理
```
错误类型:
├─ 协议检测失败 → 清晰提示 + 建议
├─ 连接失败 → 重试机制 + 超时处理
├─ 认证失败 → 401/403 处理
├─ JSON 解析错误 → 标准 JSON-RPC 错误响应
└─ 网络中断 → 优雅退出
```

## 技术实现

### 项目结构
```
mcp-stdio-proxy/
├── Cargo.toml
├── src/
│   ├── main.rs           # CLI 入口
│   ├── cli.rs            # 命令解析
│   ├── protocol.rs       # 协议检测
│   ├── client.rs         # MCP 客户端封装
│   ├── stdio.rs          # stdio 处理
│   ├── error.rs          # 错误定义
│   └── utils.rs          # 工具函数
├── tests/
│   ├── integration_test.rs
│   └── protocol_test.rs
└── README.md
```

### 核心组件

#### 1. 协议自动检测
```rust
// src/protocol.rs
pub async fn detect_mcp_protocol(url: &str, client: &Client) -> Result<McpProtocol> {
    // 1. 尝试 Streamable HTTP
    if is_streamable_http(url, client).await? {
        return Ok(McpProtocol::Stream);
    }
    
    // 2. 尝试 SSE
    if is_sse_protocol(url, client).await? {
        return Ok(McpProtocol::Sse);
    }
    
    // 3. 检测失败
    bail!("无法识别的 MCP 协议类型")
}
```

#### 2. MCP 客户端封装
```rust
// src/client.rs
#[async_trait]
pub trait McpClient {
    async fn initialize(&mut self) -> Result<()>;
    async fn send_request(&mut self, request: Value) -> Result<Value>;
    async fn next_event(&mut self) -> Option<Value>;
}

pub struct SseMcpClient {
    inner: SseClient,  // 复用现有的 SSE 客户端
}

pub struct StreamMcpClient {
    inner: StreamableHttpClient,  // 复用现有的 Streamable 客户端
}
```

#### 3. stdio 代理循环
```rust
// src/stdio.rs
pub struct StdioProxy {
    client: Box<dyn McpClient>,
}

impl StdioProxy {
    pub async fn run(&mut self) -> Result<()> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut reader = BufReader::new(stdin);
        let mut writer = BufWriter::new(stdout);
        
        let mut line = String::new();
        
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let request: Value = serde_json::from_str(&line)?;
                    let response = self.client.send_request(request).await?;
                    
                    serde_json::to_writer(&mut writer, &response)?;
                    writer.write_all(b"\n").await?;
                    writer.flush().await?;
                }
                Err(e) => return Err(e.into()),
            }
        }
        
        Ok(())
    }
}
```

### 复用现有代码

由于这是一个**独立工具**，我们可以选择：

1. **方案 A**: 作为独立 crate，依赖 mcp-proxy 库
2. **方案 B**: 复制必要的代码，完全独立
3. **方案 C**: 作为 mcp-proxy 工作空间的新成员

**推荐方案 C**: 工作空间模式

```toml
# 根目录 Cargo.toml
[workspace]
members = ["mcp-proxy", "mcp-stdio-proxy"]

# mcp-stdio-proxy/Cargo.toml
[dependencies]
mcp-proxy = { path = "../mcp-proxy" }
# 其他依赖...
```

## 使用示例

### 1. 基本使用
```bash
# 启动代理
mcp-stdio-proxy https://api.github.com/mcp

# 测试连接
echo '{"method":"tools/list","params":{},"id":1}' | \
  mcp-stdio-proxy https://api.github.com/mcp
```

### 2. 为 AI Agent 配置

```json
{
  "mcpServers": {
    "github": {
      "command": "mcp-stdio-proxy",
      "args": ["https://api.github.com/mcp", "--auth", "Bearer ${GITHUB_TOKEN}"]
    }
  }
}
```

### 3. 开发调试
```bash
# 调试模式
mcp-stdio-proxy https://api.example.com/mcp --verbose

# 然后手动输入测试:
#{"method":"tools/list","params":{},"id":1}
```

## 错误处理和用户反馈

### 友好的错误信息
```rust
match detect_mcp_protocol(&url, &client).await {
    Ok(protocol) => {
        if !quiet {
            eprintln!("🔍 检测到 {:?} 协议", protocol);
        }
    }
    Err(e) => {
        eprintln!("❌ 协议检测失败: {}", e);
        eprintln!("💡 请检查:");
        eprintln!("   - URL 是否正确可访问");
        eprintln!("   - 服务是否支持 MCP 协议");
        eprintln!("   - 网络连接是否正常");
        std::process::exit(1);
    }
}
```

### 进度指示
```
🚀 MCP-Stdio-Proxy: https://api.github.com/mcp
🔍 检测到 Streamable HTTP 协议
🔗 建立连接...
✅ 连接成功，开始代理转换...
💡 现在可以通过 stdin 发送 JSON-RPC 请求
```

## 性能考虑

### 优化策略
1. **零拷贝传输** - 尽可能直接转发数据
2. **连接复用** - 保持长连接
3. **异步处理** - 全程异步非阻塞
4. **内存效率** - 流式处理，避免大内存分配

### 性能目标
- 启动时间: < 500ms
- 请求延迟: < 5ms 额外开销
- 内存占用: < 20MB
- CPU 使用率: < 2%

## 测试策略

### 单元测试
```rust
#[tokio::test]
async fn test_protocol_detection() {
    let url = "https://sse.example.com/mcp";
    let client = create_test_client();
    
    let protocol = detect_mcp_protocol(url, &client).await.unwrap();
    assert_eq!(protocol, McpProtocol::Sse);
}
```

### 集成测试
```bash
#!/bin/bash
# 测试基本功能
echo '{"method":"ping","params":{},"id":1}' | \
  mcp-stdio-proxy https://api.example.com/mcp

# 测试认证
echo '{"method":"tools/list","params":{},"id":1}' | \
  mcp-stdio-proxy https://api.github.com/mcp --auth "Bearer $TOKEN"

# 测试错误处理
echo 'invalid json' | \
  mcp-stdio-proxy https://api.example.com/mcp 2>&1 | grep -q "解析错误"
```

## 发布计划

### 阶段 1: MVP (Week 1-2)
- [ ] 基础协议检测
- [ ] SSE 客户端集成
- [ ] stdio 代理循环
- [ ] 基本错误处理

### 阶段 2: 完善 (Week 3-4)
- [ ] Streamable HTTP 支持
- [ ] 认证机制
- [ ] 性能优化
- [ ] 完整测试

### 阶段 3: 发布 (Week 5)
- [ ] 文档完善
- [ ] 示例集合
- [ ] 发布准备
- [ ] 社区反馈

## 使用场景验证

### 场景 1: Claude Desktop 集成
```json
{
  "mcpServers": {
    "github": {
      "command": "mcp-stdio-proxy",
      "args": ["https://api.github.com/mcp", "--auth", "Bearer ${GITHUB_TOKEN}"]
    }
  }
}
```

### 场景 2: 开发调试
```bash
# 快速测试 MCP 服务
mcp-stdio-proxy https://my-dev-service.com/mcp --verbose

# 手动输入 JSON-RPC 请求进行调试
```

### 场景 3: CI/CD 集成
```yaml
# GitHub Actions
- name: Test MCP Service
  run: |
    echo '{"method":"tools/list","params":{},"id":1}' | \
      mcp-stdio-proxy ${{ secrets.MCP_SERVICE_URL }} \
      --auth "Bearer ${{ secrets.MCP_TOKEN }}"
```

这个设计提供了一个**专注、简洁、独立**的 CLI 工具，完美满足将远程 MCP 服务转换为本地 stdio 协议的需求。