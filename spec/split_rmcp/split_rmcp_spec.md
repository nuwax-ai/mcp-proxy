# MCP-Proxy 架构拆分与 stateful_mode 支持技术方案

> **方案概述**：将 mcp-proxy 按协议拆分为两个独立的 lib 库，使用不同版本的 rmcp，并通过协议探测机制自动选择，同时在 Streamable HTTP 模块中实现 stateful_mode 支持。

## 目录

1. [架构拆分设计](#1-架构拆分设计)
2. [问题背景与动机](#2-问题背景与动机)
3. [拆分后的架构](#3-拆分后的架构)
4. [stateful_mode 支持方案](#4-stateful_mode-支持方案)
5. [详细实现方案](#5-详细实现方案)
6. [迁移路径](#6-迁移路径)
7. [总结](#7-总结)

---

## 1. 架构拆分设计

### 1.1 拆分目标

将 mcp-proxy 拆分为**三个独立的 crate**，实现协议隔离和版本解耦：

```
workspace/
├── mcp-proxy/                    # 主程序（bin crate）
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs               # CLI 入口
│   │   ├── detector.rs           # 协议探测器
│   │   └── config.rs             # 统一配置
│   └── 职责：
│       ├── 协议探测和选择
│       ├── 统一 CLI 接口
│       └── 模块编排
│
├── mcp-streamable-proxy/         # Streamable HTTP 模块（lib crate）
│   ├── Cargo.toml
│   │   └── rmcp = "0.12"         # 最新版 rmcp
│   ├── src/
│   │   ├── lib.rs
│   │   ├── proxy_handler.rs      # 带版本控制的 ProxyHandler
│   │   ├── session_manager.rs    # ProxyAwareSessionManager
│   │   └── server.rs             # Streamable HTTP 服务器
│   └── 职责：
│       ├── Streamable HTTP 协议支持
│       ├── stateful_mode: true 支持
│       ├── 后端版本控制机制
│       └── Session 生命周期管理
│
└── mcp-sse-proxy/                # SSE 模块（lib crate）
    ├── Cargo.toml
    │   └── rmcp = "0.10"         # 稳定版 rmcp
    ├── src/
    │   ├── lib.rs
    │   ├── sse_handler.rs        # SSE ProxyHandler
    │   └── server.rs             # SSE 服务器
    └── 职责：
        ├── SSE 协议支持
        ├── 稳定可靠的代理功能
        └── 向后兼容
```

### 1.2 拆分优势

| 维度 | 当前架构 | 拆分后架构 | 收益 |
|------|---------|-----------|------|
| **rmcp 版本** | 单一版本 | 各模块独立版本 | ✅ 可使用不同版本特性 |
| **协议耦合** | 强耦合 | 完全隔离 | ✅ 独立开发和测试 |
| **升级风险** | 影响所有协议 | 仅影响单个模块 | ✅ 降低升级风险 |
| **功能开发** | 受旧版限制 | 新模块自由开发 | ✅ 快速迭代新特性 |
| **平滑迁移** | 不支持 | 渐进式切换 | ✅ 逐步迁移流量 |
| **依赖管理** | 共享依赖冲突 | 独立依赖树 | ✅ 减少版本冲突 |

### 1.3 协议选择策略

**优先级**：Streamable HTTP (高级) > SSE (稳定)

```
┌─────────────────────────────────────────────────────────────┐
│                      MCP Client                             │
└──────────────────────┬──────────────────────────────────────┘
                       │ HTTP Request
┌──────────────────────▼──────────────────────────────────────┐
│                   mcp-proxy (主程序)                         │
│                                                              │
│  ┌────────────────────────────────────────────────────┐     │
│  │              协议探测器 (Detector)                  │     │
│  │  ┌──────────────────────────────────────────┐      │     │
│  │  │ 1. 解析后端 MCP 服务配置                  │      │     │
│  │  │ 2. 尝试 Streamable HTTP 连接 (优先)      │      │     │
│  │  │ 3. 连接成功？                            │      │     │
│  │  │    ├─ Yes → 使用 mcp-streamable-proxy   │      │     │
│  │  │    └─ No  → 降级到 mcp-sse-proxy        │      │     │
│  │  └──────────────────────────────────────────┘      │     │
│  └────────────────────┬───────────────────────────────┘     │
│                       │                                      │
│         ┌─────────────┴──────────────┐                      │
│         │ 协议选择                    │                      │
│  ┌──────▼──────┐            ┌────────▼────────┐            │
│  │ Streamable  │            │      SSE        │            │
│  │  Dispatch   │            │   Dispatch      │            │
│  └──────┬──────┘            └────────┬────────┘            │
└─────────┼────────────────────────────┼─────────────────────┘
          │                            │
          │                            │
┌─────────▼────────┐          ┌────────▼─────────┐
│ mcp-streamable-  │          │  mcp-sse-proxy   │
│     proxy        │          │                  │
│                  │          │                  │
│ rmcp = "0.12"    │          │ rmcp = "0.10"    │
│ (最新版)         │          │ (稳定版)         │
│                  │          │                  │
│ Features:        │          │ Features:        │
│ • stateful_mode  │          │ • 稳定可靠       │
│ • 版本控制       │          │ • 向后兼容       │
│ • Session管理    │          │ • 低延迟         │
└─────────┬────────┘          └────────┬─────────┘
          │                            │
          └────────────┬───────────────┘
                       │ stdio transport
          ┌────────────▼────────────┐
          │  Backend MCP Server     │
          │    (stdio process)      │
          └─────────────────────────┘
```

**探测逻辑**：

```rust
// mcp-proxy/src/detector.rs

pub enum ProxyProtocol {
    Streamable,  // Streamable HTTP (优先)
    Sse,         // SSE (降级)
}

pub async fn detect_protocol(mcp_config: &McpConfig) -> Result<ProxyProtocol> {
    // 1. 尝试启动后端 MCP 服务
    let backend = start_mcp_backend(mcp_config).await?;

    // 2. 尝试 Streamable HTTP 连接
    match try_streamable_connection(&backend).await {
        Ok(_) => {
            info!("检测到 Streamable HTTP 协议支持，使用新版 rmcp");
            Ok(ProxyProtocol::Streamable)
        }
        Err(e) => {
            warn!("Streamable HTTP 连接失败: {}, 降级到 SSE", e);
            Ok(ProxyProtocol::Sse)
        }
    }
}

async fn try_streamable_connection(backend: &Backend) -> Result<()> {
    // 发送 Streamable HTTP 握手请求
    // 超时时间：3秒
    tokio::time::timeout(
        Duration::from_secs(3),
        backend.streamable_handshake()
    ).await??;
    Ok(())
}
```

---

## 2. 问题背景与动机

### 2.1 单一 rmcp 版本的限制

**当前状态**：

```toml
# mcp-proxy/Cargo.toml (旧架构)
[dependencies]
rmcp = "0.10"  # 所有协议共用同一版本
```

**问题**：

1. **版本升级困境**
   - SSE 功能稳定，不希望频繁升级
   - Streamable HTTP 是新功能，需要使用最新 rmcp 特性
   - 单一版本无法满足两者需求

2. **特性冲突**
   ```
   rmcp 0.10: 稳定的 SSE + Streamable HTTP 实现
   ├─ SSE 模块需要：稳定性 ✅
   └─ 新功能受限 ❌

   rmcp 0.12: 移除 SSE + 增强 Streamable HTTP + 改进的 SessionManager
   ├─ Streamable 需要：新特性 ✅
   └─ SSE 支持被移除 ❌ (无法同时支持两种协议)
   ```

3. **迁移风险**
   - 升级 rmcp 影响所有代理功能
   - 无法渐进式验证新版本
   - 回滚成本高

### 2.2 stateful_mode 的层级耦合问题

**问题描述**：在 Streamable HTTP 模式下使用 `stateful_mode: true` 时，后端重连导致 "Session error: Channel closed" 错误。

**根本原因**：**Session 生命周期与后端连接生命周期不匹配**

```
层级耦合分析：

┌─────────────────────────────────────────────────────────────┐
│                     SessionManager                          │
│  生命周期：长期存在，直到显式关闭                             │
│  状态感知：❌ 不感知后端连接状态                              │
│  假设：Handler 稳定，不会失效                                │
└──────────────────────┬──────────────────────────────────────┘
                       │ 绑定
┌──────────────────────▼──────────────────────────────────────┐
│                    ProxyHandler                             │
│  生命周期：Handler 本身长期存在                              │
│  状态感知：✅ 通过 ArcSwap 管理后端                           │
│  行为：后端可热替换 (swap_backend)                           │
└──────────────────────┬──────────────────────────────────────┘
                       │ ArcSwap
┌──────────────────────▼──────────────────────────────────────┐
│                    PeerInner (后端连接)                      │
│  生命周期：跟随后端连接，可能断开重连                         │
│  状态感知：✅ 连接断开时被替换为 None                         │
│  行为：不稳定，可能失效                                      │
└─────────────────────────────────────────────────────────────┘

矛盾：SessionManager 假设 Handler 稳定，
     但 ProxyHandler 的后端连接是不稳定的
```

**典型错误流程**：

```
时间线：

T0: 初始状态
    ├─ Backend: 运行中 (版本 1)
    ├─ Session: 无
    └─ ProxyHandler: peer = Some(backend_v1)

T1: 客户端请求
    ├─ SessionManager.create_session() → session-123
    ├─ 绑定到 ProxyHandler (backend_v1)
    ├─ 创建 Channel (tx, rx)
    └─ 正常工作 ✅

T2: 后端断开（网络问题/服务重启）
    ├─ ProxyHandler.swap_backend(None)
    ├─ peer: ArcSwap = None
    ├─ session-123 仍然存在 ⚠️
    ├─ Channel 状态：？（可能关闭）
    └─ 客户端请求失败 ❌

T3: 后端重连（watchdog 重连成功）
    ├─ ProxyHandler.swap_backend(Some(backend_v2))
    ├─ peer: ArcSwap = Some(v2)  (版本递增)
    ├─ session-123 仍然存在，但绑定到旧 handler
    └─ ⚠️ Session 不知道后端已更换

T4: 客户端请求 (带 session-123)
    ├─ SessionManager.resume(session-123)
    ├─ 尝试使用旧 Channel
    ├─ ERROR: Channel closed ❌
    └─ 客户端收到错误，无法自动恢复
```

**关键矛盾**：

| 层级 | 期望 | 实际 | 结果 |
|------|------|------|------|
| SessionManager | Session 长期稳定 | 后端可能重连 | ❌ 不一致 |
| ProxyHandler | 支持热替换 | Session 不感知 | ❌ 状态失步 |
| 客户端 | Session 可复用 | 后端重连后失效 | ❌ 错误 |

---

## 3. 拆分后的架构

### 3.1 整体架构图

```
┌──────────────────────────────────────────────────────────────┐
│                         客户端                                │
└──────────────────────┬───────────────────────────────────────┘
                       │ HTTP Request
┌──────────────────────▼───────────────────────────────────────┐
│                   mcp-proxy (bin)                             │
│                                                               │
│  ┌─────────────────────────────────────────────────────┐     │
│  │              ProxyRouter                            │     │
│  │  - 配置解析                                          │     │
│  │  - 协议探测                                          │     │
│  │  - 路由分发                                          │     │
│  └────────────────┬────────────────────────────────────┘     │
│                   │                                           │
│                   │ match protocol                            │
│         ┌─────────┴─────────┐                                │
│         │                   │                                │
│  ┌──────▼──────┐     ┌──────▼──────┐                         │
│  │ Streamable  │     │     SSE     │                         │
│  │  Dispatch   │     │  Dispatch   │                         │
│  └──────┬──────┘     └──────┬──────┘                         │
└─────────┼────────────────────┼──────────────────────────────┘
          │                    │
          │                    │
┌─────────▼────────────┐  ┌────▼─────────────────┐
│ mcp-streamable-proxy │  │  mcp-sse-proxy       │
│ (lib crate)          │  │  (lib crate)         │
│                      │  │                      │
│ Cargo.toml:          │  │ Cargo.toml:          │
│   rmcp = "0.12"      │  │   rmcp = "0.10"      │
│                      │  │                      │
│ 模块结构：           │  │ 模块结构：           │
│ ├─ ProxyHandler      │  │ ├─ SseHandler        │
│ │  (带版本控制)     │  │ │  (稳定版)          │
│ ├─ ProxyAware-       │  │ ├─ SseServer         │
│ │  SessionManager    │  │ └─ 基础代理功能     │
│ └─ StreamableServer  │  │                      │
└──────────┬───────────┘  └──────┬───────────────┘
           │                     │
           └──────────┬──────────┘
                      │ stdio transport
           ┌──────────▼──────────┐
           │  Backend MCP Server │
           └─────────────────────┘
```

### 3.2 mcp-proxy (bin crate)

**职责**：协议探测、路由分发、CLI 接口

**关键文件**：

```
mcp-proxy/
├── Cargo.toml
│   [dependencies]
│   mcp-streamable-proxy = { path = "../mcp-streamable-proxy" }
│   mcp-sse-proxy = { path = "../mcp-sse-proxy" }
│
├── src/
│   ├── main.rs              # CLI 入口
│   ├── detector.rs          # 协议探测
│   ├── router.rs            # 路由分发
│   └── config.rs            # 统一配置
```

**核心逻辑**：

```rust
// mcp-proxy/src/main.rs

use mcp_streamable_proxy::StreamableProxy;
use mcp_sse_proxy::SseProxy;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. 解析配置
    let config = Config::parse()?;

    // 2. 协议探测
    let protocol = detector::detect_protocol(&config.mcp).await?;

    // 3. 路由分发
    match protocol {
        Protocol::Streamable => {
            info!("使用 Streamable HTTP 协议 (rmcp 新版)");
            StreamableProxy::new(config)
                .run()
                .await?;
        }
        Protocol::Sse => {
            info!("使用 SSE 协议 (rmcp 稳定版)");
            SseProxy::new(config)
                .run()
                .await?;
        }
    }

    Ok(())
}
```

### 3.3 mcp-streamable-proxy (lib crate)

**职责**：Streamable HTTP 协议支持 + stateful_mode + 版本控制

**依赖版本**：

```toml
# mcp-streamable-proxy/Cargo.toml
[package]
name = "mcp-streamable-proxy"
version = "0.1.0"
edition = "2024"

[dependencies]
rmcp = "0.12"  # 最新版，支持改进的 SessionManager 和自定义 Session
tokio = { workspace = true }
axum = { workspace = true }
tracing = { workspace = true }
dashmap = { workspace = true }
arc-swap = { workspace = true }
```

**模块结构**：

```
mcp-streamable-proxy/src/
├── lib.rs                     # 公共接口
├── proxy_handler.rs           # 带版本控制的 ProxyHandler
├── session_manager.rs         # ProxyAwareSessionManager
├── server.rs                  # Streamable HTTP 服务器
└── config.rs                  # 配置定义
```

**关键特性**：

1. **后端版本控制**
   ```rust
   pub struct ProxyHandler {
       peer: Arc<ArcSwapOption<PeerInner>>,
       backend_version: Arc<AtomicU64>,  // 新增
       // ...
   }
   ```

2. **ProxyAwareSessionManager**
   ```rust
   pub struct ProxyAwareSessionManager {
       inner: LocalSessionManager,
       handler: Arc<ProxyHandler>,
       session_versions: DashMap<String, SessionMetadata>,  // 版本跟踪
   }
   ```

3. **stateful_mode: true 支持**
   ```rust
   StreamableHttpService::new(
       handler_factory,
       ProxyAwareSessionManager::new(handler).into(),
       StreamableHttpServerConfig {
           stateful_mode: true,  // ✅ 支持
           ..Default::default()
       },
   )
   ```

### 3.4 mcp-sse-proxy (lib crate)

**职责**：SSE 协议支持，稳定可靠

**依赖版本**：

```toml
# mcp-sse-proxy/Cargo.toml
[package]
name = "mcp-sse-proxy"
version = "0.1.0"
edition = "2024"

[dependencies]
rmcp = "0.10"  # 稳定版本，包含完整 SSE 支持
tokio = { workspace = true }
axum = { workspace = true }
tracing = { workspace = true }
arc-swap = { workspace = true }
```

**模块结构**：

```
mcp-sse-proxy/src/
├── lib.rs                     # 公共接口
├── sse_handler.rs             # SSE ProxyHandler (稳定版)
├── server.rs                  # SSE 服务器
└── config.rs                  # 配置定义
```

**特点**：

- ✅ 使用稳定的 rmcp 0.10
- ✅ 保留完整的 SSE 支持（0.12 已移除）
- ✅ 不引入新特性，确保稳定性
- ✅ 向后兼容

---

## 4. stateful_mode 支持方案

> **仅在 mcp-streamable-proxy 中实现**

### 4.1 核心机制：版本控制

**设计思路**：通过版本号协调 Session 生命周期与后端连接生命周期

```
┌─────────────────────────────────────────────────────────────┐
│              ProxyAwareSessionManager                       │
│  ┌───────────────────────────────────────────────────┐     │
│  │ session_versions: DashMap<SessionID, Version>     │     │
│  │                                                    │     │
│  │ create_session() {                                │     │
│  │   session_id = generate_id();                     │     │
│  │   version = handler.get_backend_version();        │     │
│  │   session_versions.insert(session_id, version);   │     │
│  │   return session_id;                              │     │
│  │ }                                                  │     │
│  │                                                    │     │
│  │ resume(session_id) {                              │     │
│  │   stored_version = session_versions.get(session_id); │  │
│  │   current_version = handler.get_backend_version();│     │
│  │   if stored_version != current_version {          │     │
│  │     return Err(SessionError::NotFound);  // 失效  │     │
│  │   }                                                │     │
│  │   return inner.resume(session_id);                │     │
│  │ }                                                  │     │
│  └───────────────────────────────────────────────────┘     │
│                          │                                  │
│                          │ 查询版本                         │
│                  ┌───────▼────────┐                         │
│                  │  ProxyHandler  │                         │
│                  │  backend_version: Arc<AtomicU64>         │
│                  │                │                         │
│                  │  swap_backend(new) {                     │
│                  │    self.peer.store(new);                 │
│                  │    self.backend_version.fetch_add(1); ← 关键
│                  │  }                │                      │
│                  └───────┬────────┘                         │
└──────────────────────────┼──────────────────────────────────┘
                           │
                   后端版本递增时机：
                   • swap_backend(None) - 断开
                   • swap_backend(Some) - 重连
```

### 4.2 完整工作流程

```
初始状态：
  backend_version = 1
  sessions = {}

─────────────────────────────────────────────────────────────

T1: 客户端请求
    ├─ SessionManager.create_session()
    ├─ backend_version = 1
    ├─ 创建 session-123
    ├─ sessions = { "session-123": { version: 1 } }
    └─ 正常工作 ✅

─────────────────────────────────────────────────────────────

T2: 后端断开
    ├─ ProxyHandler.swap_backend(None)
    ├─ backend_version: 1 → 2 (递增)
    ├─ sessions = { "session-123": { version: 1 } }  (未清理)
    └─ 新请求失败（后端不可用）❌

─────────────────────────────────────────────────────────────

T3: 后端重连
    ├─ ProxyHandler.swap_backend(Some(backend_new))
    ├─ backend_version: 2 → 3 (再次递增)
    └─ sessions = { "session-123": { version: 1 } }  (仍未清理)

─────────────────────────────────────────────────────────────

T4: 客户端请求 (session-123)
    ├─ SessionManager.resume("session-123")
    ├─ 检查：stored_version(1) != current_version(3)
    ├─ 版本不匹配 → 返回 SessionError::NotFound ✅
    ├─ 清理 session-123
    └─ 客户端收到错误，自动重新初始化

─────────────────────────────────────────────────────────────

T5: 客户端重新初始化
    ├─ SessionManager.create_session()
    ├─ backend_version = 3
    ├─ 创建 session-456
    ├─ sessions = { "session-456": { version: 3 } }
    └─ 正常工作 ✅
```

### 4.3 实现代码

**ProxyHandler 修改**：

```rust
// mcp-streamable-proxy/src/proxy_handler.rs

use std::sync::atomic::{AtomicU64, Ordering};
use arc_swap::ArcSwapOption;

pub struct ProxyHandler {
    peer: Arc<ArcSwapOption<PeerInner>>,
    cached_info: ServerInfo,
    mcp_id: String,
    tool_filter: ToolFilter,
    backend_version: Arc<AtomicU64>,  // ← 新增
}

impl ProxyHandler {
    pub fn new(running: RunningService<RoleClient, ClientInfo>, mcp_id: String) -> Self {
        // ... 初始化代码 ...

        Self {
            peer: Arc::new(ArcSwapOption::from(Some(Arc::new(inner)))),
            cached_info,
            mcp_id,
            tool_filter: ToolFilter::default(),
            backend_version: Arc::new(AtomicU64::new(1)),  // ← 初始版本
        }
    }

    pub fn swap_backend(&self, new_client: Option<RunningService<RoleClient, ClientInfo>>) {
        match new_client {
            Some(client) => {
                let peer = client.deref().clone();
                let inner = PeerInner {
                    peer,
                    _running: Arc::new(client),
                };
                self.peer.store(Some(Arc::new(inner)));
                info!("[ProxyHandler] 后端连接已更新 - MCP ID: {}", self.mcp_id);
            }
            None => {
                self.peer.store(None);
                info!("[ProxyHandler] 后端连接已断开 - MCP ID: {}", self.mcp_id);
            }
        }

        // ← 关键：每次 swap 都递增版本号
        let new_version = self.backend_version.fetch_add(1, Ordering::SeqCst) + 1;
        info!(
            "[ProxyHandler] 后端版本更新: {} - MCP ID: {}",
            new_version, self.mcp_id
        );
    }

    /// 获取当前后端版本号
    pub fn get_backend_version(&self) -> u64 {
        self.backend_version.load(Ordering::SeqCst)
    }

    pub fn is_backend_available(&self) -> bool {
        let inner_guard = self.peer.load();
        match inner_guard.as_ref() {
            Some(inner) => !inner.peer.is_transport_closed(),
            None => false,
        }
    }
}
```

**ProxyAwareSessionManager 实现**：

```rust
// mcp-streamable-proxy/src/session_manager.rs

use std::sync::Arc;
use dashmap::DashMap;
use rmcp::transport::streamable_http_server::session::{
    SessionManager, SessionError, LocalSessionManager,
};

use super::proxy_handler::ProxyHandler;

/// Session 元数据
#[derive(Debug, Clone)]
struct SessionMetadata {
    backend_version: u64,  // session 创建时绑定的后端版本
}

/// 感知代理状态的 SessionManager
///
/// 职责：
/// 1. 委托 LocalSessionManager 处理核心 session 逻辑
/// 2. 维护 session → backend_version 映射
/// 3. 在 resume 时检查版本一致性
/// 4. 版本不匹配时使 session 失效
pub struct ProxyAwareSessionManager {
    inner: LocalSessionManager,
    handler: Arc<ProxyHandler>,
    session_versions: DashMap<String, SessionMetadata>,  // 使用 DashMap
}

impl ProxyAwareSessionManager {
    pub fn new(handler: Arc<ProxyHandler>) -> Self {
        Self {
            inner: LocalSessionManager::default(),
            handler,
            session_versions: DashMap::new(),
        }
    }

    /// 检查后端版本是否匹配
    fn check_backend_version(&self, session_id: &str) -> bool {
        if let Some(meta) = self.session_versions.get(session_id) {
            let current_version = self.handler.get_backend_version();
            if meta.backend_version != current_version {
                tracing::debug!(
                    "Session {} version mismatch: {} != {}",
                    session_id,
                    meta.backend_version,
                    current_version
                );
                return false;
            }
        }
        true
    }
}

#[async_trait::async_trait]
impl SessionManager for ProxyAwareSessionManager {
    async fn create_session(&self) -> Option<String> {
        let session_id = self.inner.create_session().await?;

        // 记录创建时的后端版本
        let version = self.handler.get_backend_version();
        self.session_versions.insert(
            session_id.clone(),
            SessionMetadata {
                backend_version: version,
            },
        );

        tracing::debug!(
            "Created session {} with backend version {}",
            session_id,
            version
        );

        Some(session_id)
    }

    async fn initialize_session(
        &self,
        session_id: &str,
        service: BoxService<JsonRpcMessage, JsonRpcResponse, anyhow::Error>,
    ) -> Result<BoxService<JsonRpcMessage, JsonRpcResponse, anyhow::Error>, SessionError> {
        // 检查后端是否可用
        if !self.handler.is_backend_available() {
            tracing::warn!(
                "Rejecting session initialization {}: backend not available",
                session_id
            );
            return Err(SessionError::NotFound);
        }

        // 检查版本一致性
        if !self.check_backend_version(session_id) {
            tracing::warn!(
                "Rejecting session initialization {}: version mismatch",
                session_id
            );
            return Err(SessionError::NotFound);
        }

        self.inner.initialize_session(session_id, service).await
    }

    async fn has_session(&self, session_id: &str) -> bool {
        if !self.check_backend_version(session_id) {
            return false;
        }
        self.inner.has_session(session_id).await
    }

    async fn close_session(&self, session_id: &str) {
        self.session_versions.remove(session_id);
        self.inner.close_session(session_id).await;
        tracing::debug!("Closed session {}", session_id);
    }

    async fn create_stream(
        &self,
        session_id: &str,
    ) -> Option<BoxStream<'static, Result<JsonRpcResponse, SessionError>>> {
        if !self.handler.is_backend_available() {
            tracing::warn!("Rejecting stream creation {}: backend not available", session_id);
            return None;
        }

        if !self.check_backend_version(session_id) {
            tracing::warn!("Rejecting stream creation {}: version mismatch", session_id);
            return None;
        }

        self.inner.create_stream(session_id).await
    }

    async fn create_standalone_stream(
        &self,
    ) -> (
        BoxStream<'static, Result<JsonRpcResponse, SessionError>>,
        tokio::sync::mpsc::UnboundedSender<JsonRpcMessage>,
    ) {
        self.inner.create_standalone_stream().await
    }

    async fn accept_message(
        &self,
        session_id: &str,
        message: JsonRpcMessage,
    ) -> Result<(), SessionError> {
        if !self.handler.is_backend_available() {
            tracing::warn!(
                "Rejecting message for session {}: backend not available",
                session_id
            );
            return Err(SessionError::NotFound);
        }

        if !self.check_backend_version(session_id) {
            tracing::warn!(
                "Rejecting message for session {}: version mismatch",
                session_id
            );
            return Err(SessionError::NotFound);
        }

        self.inner.accept_message(session_id, message).await
    }

    async fn resume(
        &self,
        session_id: &str,
    ) -> Result<BoxService<JsonRpcMessage, JsonRpcResponse, anyhow::Error>, SessionError> {
        // 关键方法：resume 时检查后端版本
        if let Some(meta) = self.session_versions.get(session_id) {
            let current_version = self.handler.get_backend_version();
            if meta.backend_version != current_version {
                // 版本不匹配 → 后端已重连 → session 失效
                tracing::info!(
                    "Session {} invalidated: backend version changed ({} -> {})",
                    session_id,
                    meta.backend_version,
                    current_version
                );
                // 清理失效 session
                self.session_versions.remove(session_id);
                self.inner.close_session(session_id).await;

                return Err(SessionError::NotFound);
            }
        }

        if !self.handler.is_backend_available() {
            tracing::warn!("Cannot resume session {}: backend not available", session_id);
            return Err(SessionError::NotFound);
        }

        self.inner.resume(session_id).await
    }
}
```

---

## 5. 详细实现方案

### 5.1 rmcp 版本与 Features 配置

#### 版本差异对比

| 维度 | rmcp 0.10.0 (SSE) | rmcp 0.12.0 (Streamable) |
|------|-------------------|-------------------------|
| **发布时间** | 较早 | 最新 |
| **SSE 支持** | ✅ 完整支持 | ❌ 已移除 |
| **Streamable HTTP** | ✅ 支持 | ✅ 增强支持 |
| **SessionManager** | 基础版 | 改进版（支持自定义） |
| **稳定性** | ✅ 生产验证 | ⚠️ 较新，需验证 |
| **适用场景** | SSE 协议代理 | Streamable HTTP 代理 |

#### Features 详细对比

**rmcp 0.10.0 特有 Features（SSE 相关）**：

```toml
# SSE 服务端
transport-sse-server              # SSE 服务器传输层（使用 axum）
server-side-http                  # HTTP 服务端能力（包含 SSE）

# SSE 客户端
transport-sse-client              # SSE 客户端传输层
transport-sse-client-reqwest      # SSE 客户端（使用 reqwest）
client-side-sse                   # 客户端 SSE 支持
```

**rmcp 0.12.0 新增/改进 Features**：

```toml
# Streamable HTTP 增强
transport-streamable-http-server-session  # 会话管理（新增）
                                          # 支持自定义 SessionManager

# 其他改进
# - 改进的 SessionManager trait
# - 更好的错误处理
# - 性能优化
```

**共同 Features**：

```toml
# 核心功能
server                            # 服务端功能
client                            # 客户端功能
base64                            # Base64 编码
macros                            # 宏支持

# 传输层（共同）
transport-async-rw                # 异步读写传输
transport-child-process           # 子进程传输
transport-io                      # 标准 I/O 传输
transport-worker                  # Worker 传输
transport-streamable-http-client  # Streamable HTTP 客户端
transport-streamable-http-server  # Streamable HTTP 服务端

# HTTP 相关
reqwest                           # HTTP 客户端
tower                             # Tower 中间件
axum                              # Axum 框架集成
```

#### mcp-sse-proxy 配置（rmcp 0.10.0）

```toml
# mcp-sse-proxy/Cargo.toml

[package]
name = "mcp-sse-proxy"
version = "0.1.0"
edition = "2024"

[dependencies]
# 使用 rmcp 0.10，保留 SSE 支持
rmcp = { version = "0.10", features = [
    # 核心功能
    "server",                                    # 服务端功能
    "client",                                    # 客户端功能（连接后端）

    # SSE 相关（0.10 特有）
    "transport-sse-server",                      # SSE 服务器传输（核心）
    "transport-sse-client",                      # SSE 客户端传输
    "transport-sse-client-reqwest",              # SSE 客户端（reqwest）
    "server-side-http",                          # HTTP 服务端（SSE 需要）
    "client-side-sse",                           # 客户端 SSE

    # Streamable HTTP（用于协议探测）
    "transport-streamable-http-client",          # 探测 Streamable 支持
    "transport-streamable-http-client-reqwest",  # Streamable 客户端

    # 后端连接
    "transport-child-process",                   # stdio 子进程传输
    "transport-io",                              # I/O 传输

    # HTTP 客户端
    "reqwest",                                   # HTTP 请求

    # Web 框架
    "axum",                                      # Axum 集成
    "tower",                                     # Tower 中间件
] }

# 其他依赖
tokio = { workspace = true }
axum = { workspace = true }
tracing = { workspace = true }
arc-swap = { workspace = true }
anyhow = { workspace = true }
```

**Features 说明**：

| Feature | 用途 | 必需性 |
|---------|------|-------|
| `transport-sse-server` | SSE 服务器核心 | ✅ 必需 |
| `transport-sse-client` | 连接支持 SSE 的后端 | ✅ 必需 |
| `server-side-http` | HTTP/SSE 服务端基础 | ✅ 必需 |
| `transport-streamable-http-client` | 协议探测 | ⚠️ 可选（探测用） |
| `transport-child-process` | stdio 后端连接 | ✅ 必需 |

#### mcp-streamable-proxy 配置（rmcp 0.12.0）

```toml
# mcp-streamable-proxy/Cargo.toml

[package]
name = "mcp-streamable-proxy"
version = "0.1.0"
edition = "2024"

[dependencies]
# 使用 rmcp 0.12（最新版），专注 Streamable HTTP
rmcp = { version = "0.12", features = [
    # 核心功能
    "server",                                    # 服务端功能
    "client",                                    # 客户端功能

    # Streamable HTTP（0.12 核心）
    "transport-streamable-http-server",          # Streamable 服务器（核心）
    "transport-streamable-http-server-session",  # 会话管理（支持自定义）
    "transport-streamable-http-client",          # Streamable 客户端
    "transport-streamable-http-client-reqwest",  # Streamable 客户端（reqwest）

    # HTTP 服务端
    "server-side-http",                          # HTTP 服务端基础

    # 后端连接
    "transport-child-process",                   # stdio 子进程传输
    "transport-io",                              # I/O 传输
    "transport-async-rw",                        # 异步读写

    # HTTP 客户端
    "reqwest",                                   # HTTP 请求

    # Web 框架
    "axum",                                      # Axum 集成
    "tower",                                     # Tower 中间件

    # 工具
    "base64",                                    # Base64 编码
    "macros",                                    # 宏支持
] }

# 特殊依赖：DashMap 用于高性能并发 Session 映射
dashmap = { workspace = true }

# 其他依赖
tokio = { workspace = true }
axum = { workspace = true }
tracing = { workspace = true }
arc-swap = { workspace = true }
anyhow = { workspace = true }
async-trait = "0.1"
```

**Features 说明**：

| Feature | 用途 | 必需性 |
|---------|------|-------|
| `transport-streamable-http-server` | Streamable 服务器核心 | ✅ 必需 |
| `transport-streamable-http-server-session` | 自定义 SessionManager | ✅ 必需（关键） |
| `server-side-http` | HTTP 服务端基础 | ✅ 必需 |
| `transport-child-process` | stdio 后端连接 | ✅ 必需 |
| ~~`transport-sse-*`~~ | ❌ 0.12 中已移除 | N/A |

#### 关键差异总结

| 维度 | mcp-sse-proxy (0.10) | mcp-streamable-proxy (0.12) |
|------|---------------------|----------------------------|
| **协议** | SSE | Streamable HTTP |
| **rmcp 版本** | 0.10.0 | 0.12.0 |
| **核心 Feature** | `transport-sse-server` | `transport-streamable-http-server` |
| **Session 管理** | 默认 LocalSessionManager | 自定义 ProxyAwareSessionManager |
| **特殊能力** | SSE 长连接推送 | stateful_mode + 版本控制 |
| **稳定性** | ✅ 生产验证 | ⚠️ 需验证 |

#### 迁移注意事项

1. **不要在同一个 crate 中同时使用两个版本**
   - Rust 不支持同一依赖的多个版本共存于同一 crate
   - 通过拆分为独立 lib crate 解决

2. **协议探测时的兼容性**
   - mcp-sse-proxy 包含 `transport-streamable-http-client` 用于探测
   - mcp-streamable-proxy 不需要 SSE features

3. **Session 管理差异**
   - 0.10: 使用默认 `LocalSessionManager`，无需自定义
   - 0.12: 需要实现 `ProxyAwareSessionManager` 以支持版本控制

### 5.2 工作空间配置

**根目录 Cargo.toml**：

```toml
# /Cargo.toml
[workspace]
members = [
    "mcp-proxy",
    "mcp-streamable-proxy",
    "mcp-sse-proxy",
    "document-parser",
    "voice-cli",
    "oss-client",
]

[workspace.dependencies]
# 共享依赖（版本一致）
tokio = { version = "1", features = ["full"] }
axum = { version = "0.7", features = ["macros"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
dashmap = "6"
arc-swap = "1"

# rmcp 版本由各子 crate 独立管理，不放在 workspace.dependencies
```

### 5.2 mcp-streamable-proxy 完整结构

```
mcp-streamable-proxy/
├── Cargo.toml
│   [package]
│   name = "mcp-streamable-proxy"
│   version = "0.1.0"
│
│   [dependencies]
│   rmcp = "0.12"  # 最新版 rmcp
│   tokio = { workspace = true }
│   axum = { workspace = true }
│   tracing = { workspace = true }
│   dashmap = { workspace = true }
│   arc-swap = { workspace = true }
│   anyhow = { workspace = true }
│   async-trait = "0.1"
│
├── src/
│   ├── lib.rs
│   │   // 公共 API
│   │   pub mod proxy_handler;
│   │   pub mod session_manager;
│   │   pub mod server;
│   │   pub mod config;
│   │
│   │   pub use proxy_handler::{ProxyHandler, ToolFilter};
│   │   pub use session_manager::ProxyAwareSessionManager;
│   │   pub use server::StreamableProxy;
│   │
│   ├── proxy_handler.rs
│   │   // 带版本控制的 ProxyHandler
│   │   pub struct ProxyHandler { ... }
│   │   impl ProxyHandler {
│   │       pub fn swap_backend(...) { ... }
│   │       pub fn get_backend_version(&self) -> u64 { ... }
│   │   }
│   │
│   ├── session_manager.rs
│   │   // ProxyAwareSessionManager 实现
│   │   pub struct ProxyAwareSessionManager { ... }
│   │   impl SessionManager for ProxyAwareSessionManager { ... }
│   │
│   ├── server.rs
│   │   // Streamable HTTP 服务器
│   │   pub struct StreamableProxy { ... }
│   │   impl StreamableProxy {
│   │       pub async fn run(&self) -> Result<()> { ... }
│   │   }
│   │
│   └── config.rs
│       // 配置定义
│       pub struct StreamableProxyConfig { ... }
│
└── tests/
    └── integration_test.rs
```

### 5.3 mcp-sse-proxy 完整结构

```
mcp-sse-proxy/
├── Cargo.toml
│   [package]
│   name = "mcp-sse-proxy"
│   version = "0.1.0"
│
│   [dependencies]
│   rmcp = "0.10"  # 稳定版 rmcp
│   tokio = { workspace = true }
│   axum = { workspace = true }
│   tracing = { workspace = true }
│   arc-swap = { workspace = true }
│   anyhow = { workspace = true }
│
├── src/
│   ├── lib.rs
│   │   // 公共 API
│   │   pub mod sse_handler;
│   │   pub mod server;
│   │   pub mod config;
│   │
│   │   pub use sse_handler::SseHandler;
│   │   pub use server::SseProxy;
│   │
│   ├── sse_handler.rs
│   │   // SSE ProxyHandler (从现有代码迁移)
│   │   pub struct SseHandler { ... }
│   │
│   ├── server.rs
│   │   // SSE 服务器
│   │   pub struct SseProxy { ... }
│   │   impl SseProxy {
│   │       pub async fn run(&self) -> Result<()> { ... }
│   │   }
│   │
│   └── config.rs
│       // 配置定义
│       pub struct SseProxyConfig { ... }
│
└── tests/
    └── integration_test.rs
```

### 5.4 mcp-proxy 主程序

```
mcp-proxy/
├── Cargo.toml
│   [package]
│   name = "mcp-proxy"
│   version = "0.3.0"
│
│   [dependencies]
│   mcp-streamable-proxy = { path = "../mcp-streamable-proxy" }
│   mcp-sse-proxy = { path = "../mcp-sse-proxy" }
│   tokio = { workspace = true }
│   clap = { version = "4", features = ["derive"] }
│   tracing = { workspace = true }
│   anyhow = { workspace = true }
│
├── src/
│   ├── main.rs
│   │   // CLI 入口
│   │   #[tokio::main]
│   │   async fn main() -> Result<()> {
│   │       let args = Args::parse();
│   │
│   │       // 协议探测
│   │       let protocol = detector::detect(&args).await?;
│   │
│   │       // 路由分发
│   │       router::dispatch(protocol, args).await
│   │   }
│   │
│   ├── detector.rs
│   │   // 协议探测器
│   │   pub async fn detect(config: &Config) -> Result<Protocol> {
│   │       // 尝试 Streamable HTTP
│   │       // 失败降级到 SSE
│   │   }
│   │
│   ├── router.rs
│   │   // 路由分发
│   │   pub async fn dispatch(protocol: Protocol, args: Args) -> Result<()> {
│   │       match protocol {
│   │           Protocol::Streamable => {
│   │               use mcp_streamable_proxy::StreamableProxy;
│   │               StreamableProxy::new(args.into()).run().await
│   │           }
│   │           Protocol::Sse => {
│   │               use mcp_sse_proxy::SseProxy;
│   │               SseProxy::new(args.into()).run().await
│   │           }
│   │       }
│   │   }
│   │
│   └── config.rs
│       // 统一配置
│       pub struct Config { ... }
│
├── client/        # 现有 CLI 相关代码
└── server/        # 现有 API 服务器代码
```

### 5.5 协议探测实现

```rust
// mcp-proxy/src/detector.rs

use anyhow::{Result, Context};
use std::time::Duration;
use tokio::process::Command;
use tracing::{info, warn};

pub enum Protocol {
    Streamable,  // Streamable HTTP (优先)
    Sse,         // SSE (降级)
}

pub async fn detect(config: &McpConfig) -> Result<Protocol> {
    info!("开始探测 MCP 服务协议类型...");

    // 1. 启动后端 MCP 服务
    let mut backend = Command::new(&config.command)
        .args(config.args.as_deref().unwrap_or(&[]))
        .envs(config.env.clone().unwrap_or_default())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("启动后端 MCP 服务失败")?;

    // 2. 尝试 Streamable HTTP 握手
    match try_streamable_handshake(&mut backend).await {
        Ok(_) => {
            info!("✅ 检测到 Streamable HTTP 协议支持");
            info!("📦 使用 mcp-streamable-proxy (rmcp 新版)");

            // 关闭探测进程
            let _ = backend.kill().await;

            Ok(Protocol::Streamable)
        }
        Err(e) => {
            warn!("⚠️  Streamable HTTP 握手失败: {}", e);
            warn!("⬇️  降级到 SSE 协议");
            info!("📦 使用 mcp-sse-proxy (rmcp 稳定版)");

            // 关闭探测进程
            let _ = backend.kill().await;

            Ok(Protocol::Sse)
        }
    }
}

async fn try_streamable_handshake(backend: &mut Child) -> Result<()> {
    use rmcp::transport::streamable_http_client;

    // 超时时间：3秒
    tokio::time::timeout(
        Duration::from_secs(3),
        async {
            // 发送 Streamable HTTP 握手请求
            // 这里简化处理，实际需要通过 stdio 发送特定消息
            // 并检查响应

            // 如果后端支持 Streamable HTTP，会返回相应的响应头
            // 如果不支持，会超时或返回错误

            // TODO: 实现具体的握手逻辑
            Ok(())
        }
    ).await?
}
```

---

## 6. 迁移路径

### 6.1 Phase 1: 创建新模块（1-2天）

**步骤**：

1. **创建目录结构**
   ```bash
   mkdir -p mcp-streamable-proxy/src
   mkdir -p mcp-sse-proxy/src
   ```

2. **初始化 Cargo.toml**
   ```bash
   cd mcp-streamable-proxy
   cargo init --lib

   cd ../mcp-sse-proxy
   cargo init --lib
   ```

3. **配置依赖版本**
   - `mcp-streamable-proxy`: rmcp = "0.12" (最新版)
   - `mcp-sse-proxy`: rmcp = "0.10" (稳定版)

4. **代码迁移**
   - 将现有 `proxy_handler.rs` 复制到两个模块
   - `mcp-streamable-proxy`: 添加版本控制修改
   - `mcp-sse-proxy`: 保持原样（稳定版）

### 6.2 Phase 2: 实现 ProxyAwareSessionManager（1天）

**步骤**：

1. 创建 `mcp-streamable-proxy/src/session_manager.rs`
2. 实现 `ProxyAwareSessionManager`
3. 修改 `ProxyHandler` 添加 `backend_version`
4. 单元测试

### 6.3 Phase 3: 实现协议探测（1天）

**步骤**：

1. 创建 `mcp-proxy/src/detector.rs`
2. 实现 `detect()` 函数
3. 实现 `try_streamable_handshake()`
4. 集成测试

### 6.4 Phase 4: 主程序集成（1天）

**步骤**：

1. 修改 `mcp-proxy/src/main.rs`
2. 添加依赖到两个 lib 库
3. 实现路由分发逻辑
4. 端到端测试

### 6.5 Phase 5: 测试与验证（2-3天）

**测试场景**：

| 场景 | 测试内容 | 预期结果 |
|------|---------|---------|
| **协议探测** | Streamable HTTP 后端 | 使用 streamable-proxy |
| **协议探测** | SSE 后端 | 使用 sse-proxy |
| **版本控制** | 后端重连 | Session 自动失效 |
| **stateful_mode** | 服务端推送 | 正常工作 |
| **兼容性** | 旧客户端 | SSE 模式正常 |

### 6.6 Phase 6: 平滑迁移（长期）

**策略**：

```
阶段 1：共存阶段（当前）
  ├─ 默认：协议探测，自动选择
  ├─ 配置：可强制指定协议
  └─ 监控：记录两种协议的使用率

阶段 2：逐步迁移（3-6个月）
  ├─ 鼓励使用 Streamable HTTP
  ├─ 收集 stateful_mode 使用反馈
  └─ 优化新协议性能

阶段 3：完全迁移（6-12个月）
  ├─ Streamable HTTP 成为默认
  ├─ SSE 标记为 deprecated
  └─ 准备移除 SSE 模块

阶段 4：清理阶段（12个月后）
  ├─ 移除 mcp-sse-proxy
  └─ mcp-streamable-proxy 成为唯一实现
```

---

## 7. 总结

### 7.1 架构优势

| 维度 | 改进 |
|------|------|
| **模块化** | ✅ 按协议清晰拆分，职责明确 |
| **版本管理** | ✅ 两个 rmcp 版本共存，互不影响 |
| **风险控制** | ✅ 新功能在新模块开发，不影响稳定性 |
| **平滑迁移** | ✅ 协议探测 + 自动降级，无缝切换 |
| **stateful 支持** | ✅ 版本控制机制，优雅处理重连 |
| **可维护性** | ✅ 独立测试、独立部署、独立迭代 |

### 7.2 技术亮点

1. **版本控制机制**
   - `AtomicU64` 实现无锁版本递增
   - `DashMap` 高效并发 session 映射
   - 自动失效失效 session

2. **协议探测**
   - 优先 Streamable HTTP，自动降级 SSE
   - 无需用户配置，自动适配
   - 透明切换，不影响用户体验

3. **适配器模式**
   - `ProxyAwareSessionManager` 不修改 rmcp 核心
   - 委托模式复用 `LocalSessionManager`
   - 清晰的层级边界

### 7.3 实施路线

```
Week 1: 创建模块 + 代码迁移
Week 2: 实现 ProxyAwareSessionManager
Week 3: 协议探测 + 主程序集成
Week 4: 测试验证 + 文档更新
Week 5+: 生产验证 + 平滑迁移
```

### 7.4 预期收益

- ✅ **支持 stateful_mode: true**，启用服务端推送能力
- ✅ **两个 rmcp 版本共存**，新旧功能互不影响
- ✅ **协议自动探测**，用户无感知切换
- ✅ **平滑迁移路径**，逐步替换旧实现
- ✅ **符合 CLAUDE.md 规范**，使用 dashmap、无锁设计

---

## 附录

### A. 关键文件清单

| 文件路径 | 操作 | 说明 |
|---------|------|------|
| `mcp-streamable-proxy/Cargo.toml` | 新建 | Streamable HTTP 模块配置 |
| `mcp-streamable-proxy/src/proxy_handler.rs` | 新建 | 带版本控制的 ProxyHandler |
| `mcp-streamable-proxy/src/session_manager.rs` | 新建 | ProxyAwareSessionManager |
| `mcp-sse-proxy/Cargo.toml` | 新建 | SSE 模块配置 |
| `mcp-sse-proxy/src/sse_handler.rs` | 新建 | 稳定的 SSE Handler |
| `mcp-proxy/src/detector.rs` | 新建 | 协议探测器 |
| `mcp-proxy/src/router.rs` | 新建 | 路由分发器 |
| `mcp-proxy/Cargo.toml` | 修改 | 添加两个 lib 库依赖 |

### B. rmcp 最新源码分析（基于 temp/rust-sdk）

#### 版本信息

```toml
[workspace.package]
version = "0.12.0"  # 当前版本
```

#### 关键 Commit 分析

**SSE 移除（Breaking Change）**：

```
commit eb5a7f7 (2024-12-02)
feat!: remove SSE transport support (#562)

SSE transport has been removed from the MCP specification in favor of
streamable HTTP. This removes all SSE-specific transport code.

BREAKING CHANGES:
- Removed: transport-sse-client feature
- Removed: transport-sse-client-reqwest feature
- Removed: transport-sse-server feature
- Removed: SseClientTransport type
- Removed: SseServer type

Migration: Use StreamableHttpClientTransport and StreamableHttpService
```

**影响**：
- ❌ rmcp >= 0.11 完全移除 SSE 支持
- ✅ Streamable HTTP 成为唯一协议
- ⚠️ 这强化了我们拆分方案的必要性

#### SessionManager Trait 实现

**源码位置**：`temp/rust-sdk/crates/rmcp/src/transport/streamable_http_server/session.rs`

```rust
pub trait SessionManager: Send + Sync + 'static {
    type Error: std::error::Error + Send + 'static;
    type Transport: crate::transport::Transport<RoleServer>;

    // 关键方法
    fn create_session(&self)
        -> impl Future<Output = Result<(SessionId, Self::Transport), Self::Error>> + Send;

    fn initialize_session(&self, id: &SessionId, message: ClientJsonRpcMessage)
        -> impl Future<Output = Result<ServerJsonRpcMessage, Self::Error>> + Send;

    fn resume(&self, id: &SessionId, last_event_id: String)
        -> impl Future<Output = Result<impl Stream<Item = ServerSseMessage>, Self::Error>> + Send;

    // ... 其他方法
}
```

**LocalSessionManager 实现细节**：

```rust
// 源码：temp/rust-sdk/crates/rmcp/src/transport/streamable_http_server/session/local.rs

pub struct LocalSessionManager {
    // ⚠️ 使用 RwLock<HashMap> 而非 DashMap
    pub sessions: tokio::sync::RwLock<HashMap<SessionId, LocalSessionHandle>>,
    pub session_config: SessionConfig,
}

impl SessionManager for LocalSessionManager {
    async fn create_session(&self) -> Result<(SessionId, Self::Transport), Self::Error> {
        let id = session_id();
        let (handle, worker) = create_local_session(id.clone(), self.session_config.clone());

        // 写锁：插入新 session
        self.sessions.write().await.insert(id.clone(), handle);

        Ok((id, WorkerTransport::spawn(worker)))
    }

    async fn resume(&self, id: &SessionId, last_event_id: String)
        -> Result<impl Stream<Item = ServerSseMessage>, Self::Error> {
        // 读锁：获取 session
        let sessions = self.sessions.read().await;
        let handle = sessions.get(id)
            .ok_or(LocalSessionManagerError::SessionNotFound(id.clone()))?;
        let receiver = handle.resume(last_event_id.parse()?).await?;
        Ok(ReceiverStream::new(receiver.inner))
    }
}
```

**性能问题**：
- ❌ `RwLock<HashMap>` 在高并发下性能较差
- ❌ 每次读取都需要获取锁
- ✅ 我们使用 `DashMap` 可以显著提升性能

#### 关键优化建议

**1. 使用 DashMap 替代 RwLock<HashMap>**

```rust
// 当前 LocalSessionManager (rmcp 0.12 源码)
sessions: tokio::sync::RwLock<HashMap<SessionId, LocalSessionHandle>>

// 我们的优化 (ProxyAwareSessionManager)
session_versions: DashMap<String, SessionMetadata>
```

**优势**：
- ✅ 无锁并发读写
- ✅ 更好的性能（符合 CLAUDE.md 建议）
- ✅ 更简洁的 API

**2. 版本控制机制**

rmcp 源码中的 SessionManager 不支持后端重连场景，我们的 `ProxyAwareSessionManager` 通过版本控制机制解决：

```rust
pub struct ProxyAwareSessionManager {
    inner: LocalSessionManager,          // 委托核心逻辑
    handler: Arc<ProxyHandler>,          // 检查后端状态
    session_versions: DashMap<String, SessionMetadata>,  // 版本跟踪
}

// 关键：resume 时检查版本
async fn resume(&self, id: &SessionId, last_event_id: String) -> Result<...> {
    if let Some(meta) = self.session_versions.get(id) {
        let current_version = self.handler.get_backend_version();
        if meta.backend_version != current_version {
            // 后端已重连，session 失效
            return Err(SessionError::NotFound);
        }
    }
    self.inner.resume(id, last_event_id).await
}
```

#### 新特性支持

rmcp 0.12.0 引入的新特性：

| 特性 | Commit | 说明 | 我们是否需要 |
|------|--------|------|------------|
| **Task 支持** | 621c9f6 | SEP-1686: Task support | ⚠️ 可选 |
| **Elicitation** | e9029cc | SEP-1330: Elicitation Enum Schema | ⚠️ 可选 |
| **自定义请求** | e0faf1e | Custom requests support | ⚠️ 可选 |
| **自定义通知** | 2e3cc4a | Custom server notifications | ⚠️ 可选 |
| **OutputSchema 验证** | df84555 | 输出 schema 验证 | ✅ 推荐 |

#### 升级建议

**mcp-streamable-proxy 使用最新源码的策略**：

```toml
# 选项 1：使用发布版本 (推荐)
rmcp = { version = "0.12", features = [...] }

# 选项 2：使用 git 依赖 (最新特性)
rmcp = {
    git = "https://github.com/modelcontextprotocol/rust-sdk",
    branch = "main",
    features = [...]
}

# 选项 3：使用本地源码 (开发调试)
rmcp = {
    path = "../../temp/rust-sdk/crates/rmcp",
    features = [...]
}
```

**推荐使用选项 1**：
- ✅ 稳定可靠
- ✅ 版本管理清晰
- ✅ 便于追踪问题

**临时使用选项 3 的场景**：
- 🔍 调试 SessionManager 相关问题
- 🧪 测试新特性
- 🛠️ 需要修改 rmcp 源码

### C. 参考资料

- [rmcp 0.10.0 文档](https://docs.rs/rmcp/0.10.0)：稳定版 API，包含完整 SSE 支持
- [rmcp 0.10.0 Features](https://docs.rs/crate/rmcp/0.10.0/features)：0.10 版本 features 列表
- [rmcp 0.12.0 文档](https://docs.rs/rmcp/0.12.0)：最新版 API，增强的 SessionManager
- [rmcp 0.12.0 Features](https://docs.rs/crate/rmcp/0.12.0/features)：0.12 版本 features 列表
- [rmcp 官方源码](https://github.com/modelcontextprotocol/rust-sdk)：最新开发动态
- [MCP 规范](https://spec.modelcontextprotocol.io/)：协议规范
- CLAUDE.md：项目开发规范
