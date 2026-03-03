//! Streamable HTTP Server Builder
//!
//! This module provides a high-level Builder API for creating Streamable HTTP MCP servers.
//! It encapsulates all rmcp-specific types and provides a simple interface for mcp-proxy.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::Result;
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo},
    transport::{
        TokioChildProcess,
        streamable_http_client::{
            StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
        },
        streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService},
    },
};

// Unix 进程组支持
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;

// Windows 静默运行支持
#[cfg(windows)]
use process_wrap::tokio::{CreationFlags, JobObject};
#[cfg(windows)]
use windows::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_NEW_PROCESS_GROUP};

use crate::{ProxyAwareSessionManager, ProxyHandler, ToolFilter};
pub use mcp_common::ToolFilter as CommonToolFilter;

/// Backend configuration for the MCP server
///
/// Defines how the proxy connects to the upstream MCP service.
#[derive(Debug, Clone)]
pub enum BackendConfig {
    /// Connect to a local command via stdio
    Stdio {
        /// Command to execute (e.g., "npx", "python", etc.)
        command: String,
        /// Arguments for the command
        args: Option<Vec<String>>,
        /// Environment variables
        env: Option<HashMap<String, String>>,
    },
    /// Connect to a remote URL
    Url {
        /// URL of the MCP service
        url: String,
        /// Custom HTTP headers (including Authorization)
        headers: Option<HashMap<String, String>>,
    },
}

/// Configuration for the Streamable HTTP server
#[derive(Debug, Clone, Default)]
pub struct StreamServerConfig {
    /// Enable stateful mode with session management
    pub stateful_mode: bool,
    /// MCP service identifier for logging
    pub mcp_id: Option<String>,
    /// Tool filter configuration
    pub tool_filter: Option<ToolFilter>,
}

/// Builder for creating Streamable HTTP MCP servers
///
/// Provides a fluent API for configuring and building MCP proxy servers.
///
/// # Example
///
/// ```rust,ignore
/// use mcp_streamable_proxy::server_builder::{StreamServerBuilder, BackendConfig};
///
/// // Create a server with stdio backend
/// let (router, ct) = StreamServerBuilder::new(BackendConfig::Stdio {
///     command: "npx".into(),
///     args: Some(vec!["-y".into(), "@modelcontextprotocol/server-filesystem".into()]),
///     env: None,
/// })
/// .mcp_id("my-server")
/// .stateful(false)
/// .build()
/// .await?;
/// ```
pub struct StreamServerBuilder {
    backend_config: BackendConfig,
    server_config: StreamServerConfig,
}

impl StreamServerBuilder {
    /// Create a new builder with the given backend configuration
    pub fn new(backend: BackendConfig) -> Self {
        Self {
            backend_config: backend,
            server_config: StreamServerConfig::default(),
        }
    }

    /// Set whether to enable stateful mode
    ///
    /// Stateful mode enables session management and server-side push.
    pub fn stateful(mut self, enabled: bool) -> Self {
        self.server_config.stateful_mode = enabled;
        self
    }

    /// Set the MCP service identifier
    ///
    /// Used for logging and service identification.
    pub fn mcp_id(mut self, id: impl Into<String>) -> Self {
        self.server_config.mcp_id = Some(id.into());
        self
    }

    /// Set the tool filter configuration
    pub fn tool_filter(mut self, filter: ToolFilter) -> Self {
        self.server_config.tool_filter = Some(filter);
        self
    }

    /// Build the server and return an axum Router, CancellationToken, and ProxyHandler
    ///
    /// The router can be merged with other axum routers or served directly.
    /// The CancellationToken can be used to gracefully shut down the service.
    /// The ProxyHandler can be used for status checks and management.
    pub async fn build(self) -> Result<(axum::Router, CancellationToken, ProxyHandler)> {
        let mcp_id = self
            .server_config
            .mcp_id
            .clone()
            .unwrap_or_else(|| "stream-proxy".into());

        // Create client info for connecting to backend
        let client_info = ClientInfo {
            protocol_version: Default::default(),
            capabilities: ClientCapabilities::builder()
                .enable_experimental()
                .enable_roots()
                .enable_roots_list_changed()
                .enable_sampling()
                .build(),
            ..Default::default()
        };

        // Connect to backend based on configuration
        let client = match &self.backend_config {
            BackendConfig::Stdio { command, args, env } => {
                self.connect_stdio(command, args, env, &client_info).await?
            }
            BackendConfig::Url { url, headers } => {
                self.connect_url(url, headers, &client_info).await?
            }
        };

        // Create proxy handler
        let proxy_handler = if let Some(ref tool_filter) = self.server_config.tool_filter {
            ProxyHandler::with_tool_filter(client, mcp_id.clone(), tool_filter.clone())
        } else {
            ProxyHandler::with_mcp_id(client, mcp_id.clone())
        };

        // Clone handler before creating server
        let handler_for_return = proxy_handler.clone();

        // Create server with configured stateful mode
        let (router, ct) = self.create_server(proxy_handler).await?;

        info!(
            "[StreamServerBuilder] Server created - mcp_id: {}, stateful: {}",
            mcp_id, self.server_config.stateful_mode
        );

        Ok((router, ct, handler_for_return))
    }

    /// Connect to a stdio backend (child process)
    async fn connect_stdio(
        &self,
        command: &str,
        args: &Option<Vec<String>>,
        env: &Option<HashMap<String, String>>,
        client_info: &ClientInfo,
    ) -> Result<rmcp::service::RunningService<rmcp::RoleClient, ClientInfo>> {
        // Windows 上预处理 npx 命令，避免 .cmd 文件导致窗口闪烁
        #[cfg(windows)]
        let (command, args) = self.preprocess_npx_command_windows(command, args.clone());
        #[cfg(not(windows))]
        let args = args.clone();

        // 使用 process-wrap 创建子进程命令（跨平台进程清理）
        // process-wrap 会自动处理进程组（Unix）或 Job Object（Windows）
        // 并且在 Drop 时自动清理子进程树
        let mut wrapped_cmd = CommandWrap::with_new(&command, |cmd| {
            let (final_path, filtered_env) = mcp_common::prepare_stdio_env(env);
            if let Some(path) = final_path {
                cmd.env("PATH", path);
            } else {
                warn!("[StreamServerBuilder] PATH not available from parent process or config");
            }

            if let Some(cmd_args) = &args {
                cmd.args(cmd_args);
            }

            if let Some(vars) = filtered_env {
                for (k, v) in vars {
                    cmd.env(k, v);
                }
            }
        });

        // Unix: 创建进程组，支持 killpg 清理整个进程树
        #[cfg(unix)]
        wrapped_cmd.wrap(ProcessGroup::leader());

        // Windows: 使用 CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP 隐藏控制台窗口
        #[cfg(windows)]
        {
            use windows::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_NEW_PROCESS_GROUP};
            info!(
                "[StreamServerBuilder] Setting CreationFlags: CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP"
            );
            wrapped_cmd.wrap(CreationFlags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP));
            wrapped_cmd.wrap(JobObject);
        }

        // 所有平台: Drop 时自动清理进程
        wrapped_cmd.wrap(KillOnDrop);

        info!(
            "[StreamServerBuilder] Starting child process - command: {}, args: {:?}",
            command,
            args.as_ref().unwrap_or(&vec![])
        );

        let mcp_id = self
            .server_config
            .mcp_id
            .as_deref()
            .unwrap_or("unknown");

        // 诊断日志：子进程关键环境变量
        mcp_common::diagnostic::log_stdio_spawn_context("StreamServerBuilder", mcp_id, env);

        // MCP 服务通过 stdin/stdout 进行 JSON-RPC 通信，必须使用 piped（默认行为）
        // 只设置 stderr 为 null，避免控制台错误输出导致窗口弹出
        let (tokio_process, _stderr) = TokioChildProcess::builder(wrapped_cmd)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                anyhow::anyhow!(
                    "{}",
                    mcp_common::diagnostic::format_spawn_error(mcp_id, &command, &args, e)
                )
            })?;
        let client = client_info.clone().serve(tokio_process).await?;

        info!("[StreamServerBuilder] Child process connected successfully");
        Ok(client)
    }

    /// Connect to a URL backend (remote MCP service)
    async fn connect_url(
        &self,
        url: &str,
        headers: &Option<HashMap<String, String>>,
        client_info: &ClientInfo,
    ) -> Result<rmcp::service::RunningService<rmcp::RoleClient, ClientInfo>> {
        info!("[StreamServerBuilder] Connecting to URL backend: {}", url);

        // Build HTTP client with custom headers (excluding Authorization)
        let mut req_headers = reqwest::header::HeaderMap::new();
        let mut auth_header: Option<String> = None;

        if let Some(config_headers) = headers {
            for (key, value) in config_headers {
                // Authorization header is handled separately by rmcp
                if key.eq_ignore_ascii_case("Authorization") {
                    auth_header = Some(value.strip_prefix("Bearer ").unwrap_or(value).to_string());
                    continue;
                }

                req_headers.insert(
                    reqwest::header::HeaderName::try_from(key)
                        .map_err(|e| anyhow::anyhow!("Invalid header name '{}': {}", key, e))?,
                    value.parse().map_err(|e| {
                        anyhow::anyhow!("Invalid header value for '{}': {}", key, e)
                    })?,
                );
            }
        }

        let http_client = reqwest::Client::builder()
            .default_headers(req_headers)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

        // Create transport configuration
        let config = StreamableHttpClientTransportConfig {
            uri: url.to_string().into(),
            auth_header,
            ..Default::default()
        };

        let transport = StreamableHttpClientTransport::with_client(http_client, config);
        let client = client_info.clone().serve(transport).await?;

        info!("[StreamServerBuilder] URL backend connected successfully");
        Ok(client)
    }

    /// Windows 上预处理 npx 命令
    ///
    /// 将 `npx -y package@version` 转换为直接的 `node` 命令，
    /// 避免使用 .cmd 批处理文件导致窗口闪烁。
    #[cfg(windows)]
    fn preprocess_npx_command_windows(
        &self,
        command: &str,
        args: Option<Vec<String>>,
    ) -> (String, Option<Vec<String>>) {
        // 检测 npx 命令
        let is_npx = command == "npx"
            || command == "npx.cmd"
            || command.ends_with("/npx")
            || command.ends_with("\\npx")
            || command.ends_with("/npx.cmd")
            || command.ends_with("\\npx.cmd");

        if !is_npx {
            return (command.to_string(), args);
        }

        let args = match args {
            Some(a) => a,
            None => return (command.to_string(), None),
        };

        // 提取包名（跳过 -y 标志）
        let package_spec = args.iter().find(|s| !s.starts_with('-') && s.contains('@'));

        let Some(pkg) = package_spec else {
            return (command.to_string(), Some(args));
        };

        // 解析包名（去掉版本号）
        let package_name = pkg.split('@').next().unwrap_or(pkg);

        // 尝试找到已安装的包
        if let Some((node_exe, js_entry)) = self.find_npx_package_entry_windows(package_name) {
            info!(
                "[StreamServerBuilder] Windows npx 转换: npx {} -> node {}",
                pkg,
                js_entry.display()
            );

            // 构建新参数
            let mut new_args = vec![js_entry.to_string_lossy().to_string()];
            for arg in &args {
                if arg != "-y" && arg != pkg {
                    new_args.push(arg.clone());
                }
            }

            return (node_exe.to_string_lossy().to_string(), Some(new_args));
        }

        // 未找到已安装的包，保持原样
        info!(
            "[StreamServerBuilder] Windows npx 未找到已安装的包: {}，保持原命令",
            pkg
        );
        (command.to_string(), Some(args))
    }

    /// 查找 npx 包的 node 可执行文件和 JS 入口
    #[cfg(windows)]
    fn find_npx_package_entry_windows(
        &self,
        package_name: &str,
    ) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        // 查找 node.exe
        let node_exe = self.find_node_exe_windows()?;

        // 在多个可能的位置查找已安装的包
        let search_paths = self.get_npx_cache_paths_windows();

        for node_modules_dir in search_paths {
            let package_dir = node_modules_dir.join(package_name);
            if !package_dir.exists() {
                continue;
            }

            // 读取 package.json 查找入口
            let package_json_path = package_dir.join("package.json");
            if let Ok(content) = std::fs::read_to_string(&package_json_path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    // 查找 bin 字段
                    let bin_entry = json.get("bin").and_then(|b| {
                        if let Some(s) = b.as_str() {
                            Some(s.to_string())
                        } else if let Some(obj) = b.as_object() {
                            obj.get(package_name)
                                .or_else(|| obj.values().next())
                                .and_then(|v| v.as_str())
                                .map(str::to_string)
                        } else {
                            None
                        }
                    });

                    if let Some(bin_entry) = bin_entry {
                        let js_entry = package_dir.join(bin_entry);
                        if js_entry.exists() {
                            info!(
                                "[StreamServerBuilder] Windows 找到包入口: {} -> {}",
                                package_name,
                                js_entry.display()
                            );
                            return Some((node_exe.clone(), js_entry));
                        }
                    }
                }
            }
        }

        None
    }

    /// 查找 node.exe 路径
    #[cfg(windows)]
    fn find_node_exe_windows(&self) -> Option<std::path::PathBuf> {
        use std::path::PathBuf;

        // 1. 检查环境变量
        if let Ok(node_from_env) = std::env::var("NUWAX_NODE_EXE") {
            let path = PathBuf::from(node_from_env);
            if path.exists() {
                return Some(path);
            }
        }

        // 2. 检查应用资源目录
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let resource_paths = [
                    exe_dir.join("resources").join("node").join("bin").join("node.exe"),
                    exe_dir.parent()
                        .unwrap_or(exe_dir)
                        .join("resources")
                        .join("node")
                        .join("bin")
                        .join("node.exe"),
                ];

                for path in resource_paths {
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }

        // 3. 在 PATH 中查找
        which::which("node.exe").ok()
    }

    /// 获取 npx 缓存搜索路径
    #[cfg(windows)]
    fn get_npx_cache_paths_windows(&self) -> Vec<std::path::PathBuf> {
        use std::path::PathBuf;

        let mut paths = Vec::new();

        // npm 全局 node_modules
        if let Ok(appdata) = std::env::var("APPDATA") {
            let appdata_path = PathBuf::from(&appdata);

            // npm 全局目录
            paths.push(appdata_path.join("npm").join("node_modules"));

            // 应用私有目录
            paths.push(
                appdata_path
                    .join("com.nuwax.agent-tauri-client")
                    .join("node_modules"),
            );

            // npx 缓存目录（npm 8.16+）
            paths.push(appdata_path.join("npm-cache").join("_npx"));
        }

        // 应用资源目录
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let resource_paths = [
                    exe_dir.join("resources").join("node").join("node_modules"),
                    exe_dir.parent()
                        .unwrap_or(exe_dir)
                        .join("resources")
                        .join("node")
                        .join("node_modules"),
                ];

                for path in resource_paths {
                    if path.exists() {
                        paths.push(path);
                    }
                }
            }
        }

        paths
    }

    /// Create the Streamable HTTP server
    async fn create_server(
        &self,
        proxy_handler: ProxyHandler,
    ) -> Result<(axum::Router, CancellationToken)> {
        let handler = Arc::new(proxy_handler);
        let ct = CancellationToken::new();

        if self.server_config.stateful_mode {
            // Stateful mode with custom session manager
            let session_manager = ProxyAwareSessionManager::new(handler.clone());
            let handler_for_service = handler.clone();

            let service = StreamableHttpService::new(
                move || Ok((*handler_for_service).clone()),
                session_manager.into(),
                StreamableHttpServerConfig {
                    stateful_mode: true,
                    ..Default::default()
                },
            );

            let router = axum::Router::new().fallback_service(service);
            Ok((router, ct))
        } else {
            // Stateless mode with local session manager
            use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;

            let handler_for_service = handler.clone();

            let service = StreamableHttpService::new(
                move || Ok((*handler_for_service).clone()),
                LocalSessionManager::default().into(),
                StreamableHttpServerConfig {
                    stateful_mode: false,
                    ..Default::default()
                },
            );

            let router = axum::Router::new().fallback_service(service);
            Ok((router, ct))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_creation() {
        let builder = StreamServerBuilder::new(BackendConfig::Stdio {
            command: "echo".into(),
            args: Some(vec!["hello".into()]),
            env: None,
        })
        .mcp_id("test")
        .stateful(true);

        assert!(builder.server_config.mcp_id.is_some());
        assert_eq!(builder.server_config.mcp_id.as_deref(), Some("test"));
        assert!(builder.server_config.stateful_mode);
    }

    #[test]
    fn test_url_backend_config() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".into(), "Bearer token123".into());
        headers.insert("X-Custom".into(), "value".into());

        let builder = StreamServerBuilder::new(BackendConfig::Url {
            url: "http://localhost:8080/mcp".into(),
            headers: Some(headers),
        });

        match &builder.backend_config {
            BackendConfig::Url { url, headers } => {
                assert_eq!(url, "http://localhost:8080/mcp");
                assert!(headers.is_some());
            }
            _ => panic!("Expected URL backend"),
        }
    }
}
