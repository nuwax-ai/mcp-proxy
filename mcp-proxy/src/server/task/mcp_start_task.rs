//! MCP Service Start Task
//!
//! This module handles starting MCP services using the Builder APIs from
//! mcp-sse-proxy and mcp-streamable-proxy libraries.
//!
//! The refactored implementation removes direct rmcp dependency by delegating
//! protocol-specific logic to the proxy libraries.

use crate::{
    AppError, DynamicRouterService, get_proxy_manager,
    model::{
        CheckMcpStatusResponseStatus, McpConfig, McpProtocol, McpProtocolPath, McpRouterPath,
        McpServerCommandConfig, McpServerConfig, McpServiceStatus, McpType,
    },
    proxy::{McpHandler, SseBackendConfig, SseServerBuilder, StreamBackendConfig, StreamServerBuilder},
};

use anyhow::Result;
use log::{debug, info};

/// Start an MCP service based on configuration
///
/// This function creates and configures an MCP proxy service based on the
/// provided configuration. It supports both SSE and Streamable HTTP client
/// protocols, with automatic backend protocol detection for URL-based services.
pub async fn mcp_start_task(
    mcp_config: McpConfig,
) -> Result<(axum::Router, tokio_util::sync::CancellationToken)> {
    let mcp_id = mcp_config.mcp_id.clone();
    let client_protocol = mcp_config.client_protocol.clone();

    // Create router path based on client protocol (determines exposed API interface)
    let mcp_router_path: McpRouterPath =
        McpRouterPath::new(mcp_id, client_protocol).map_err(|e| AppError::McpServerError(e))?;

    let mcp_json_config = mcp_config
        .mcp_json_config
        .clone()
        .expect("mcp_json_config is required");

    let mcp_server_config = McpServerConfig::try_from(mcp_json_config)?;

    // Use the integrated method to create the server
    integrate_server_with_axum(
        mcp_server_config.clone(),
        mcp_router_path.clone(),
        mcp_config.mcp_type,
    )
    .await
}

/// Integrate MCP server with axum router
///
/// This function:
/// 1. Determines backend protocol (stdio, SSE, or Streamable HTTP)
/// 2. Creates the appropriate server using Builder APIs
/// 3. Registers the handler with ProxyManager
/// 4. Sets up dynamic routing
pub async fn integrate_server_with_axum(
    mcp_config: McpServerConfig,
    mcp_router_path: McpRouterPath,
    mcp_type: McpType,
) -> Result<(axum::Router, tokio_util::sync::CancellationToken)> {
    let base_path = mcp_router_path.base_path.clone();
    let mcp_id = mcp_router_path.mcp_id.clone();

    // Determine backend protocol from configuration
    let backend_protocol = match &mcp_config {
        // Command-line config: use stdio protocol
        McpServerConfig::Command(_) => McpProtocol::Stdio,
        // URL config: parse type field or auto-detect
        McpServerConfig::Url(url_config) => {
            // Check type field first
            if let Some(type_str) = &url_config.r#type {
                match type_str.parse::<McpProtocol>() {
                    Ok(protocol) => {
                        debug!("Using configured protocol type: {} -> {:?}", type_str, protocol);
                        protocol
                    }
                    Err(_) => {
                        // If parsing fails, auto-detect
                        debug!("Protocol type '{}' unrecognized, auto-detecting", type_str);
                        let detected_protocol =
                            crate::server::detect_mcp_protocol(url_config.get_url())
                                .await
                                .map_err(|e| {
                                    anyhow::anyhow!(
                                        "Protocol type '{}' unrecognized and auto-detection failed: {}",
                                        type_str,
                                        e
                                    )
                                })?;
                        debug!(
                            "Auto-detected protocol: {:?} (original config: '{}')",
                            detected_protocol, type_str
                        );
                        detected_protocol
                    }
                }
            } else {
                // No type field, auto-detect
                debug!("No type field specified, auto-detecting protocol");
                let detected_protocol = crate::server::detect_mcp_protocol(url_config.get_url())
                    .await
                    .map_err(|e| anyhow::anyhow!("Auto-detection failed: {}", e))?;
                detected_protocol
            }
        }
    };

    debug!(
        "MCP ID: {}, client protocol: {:?}, backend protocol: {:?}",
        mcp_id, mcp_router_path.mcp_protocol, backend_protocol
    );

    // Create server based on client protocol using Builder APIs
    let (router, ct, handler) = match mcp_router_path.mcp_protocol.clone() {
        // ================ Client uses SSE protocol ================
        McpProtocol::Sse => {
            let sse_path = match &mcp_router_path.mcp_protocol_path {
                McpProtocolPath::SsePath(sse_path) => sse_path,
                _ => unreachable!(),
            };

            // Build backend config for SSE
            let backend_config = build_sse_backend_config(&mcp_config, backend_protocol)?;

            debug!(
                "Creating SSE server, sse_path={}, post_path={}",
                sse_path.sse_path, sse_path.message_path
            );

            let (router, ct, handler) = SseServerBuilder::new(backend_config)
                .mcp_id(mcp_id.clone())
                .sse_path(sse_path.sse_path.clone())
                .post_path(sse_path.message_path.clone())
                .build()
                .await?;

            info!(
                "SSE server started - MCP ID: {}, type: {:?}",
                mcp_router_path.mcp_id, mcp_type
            );

            (router, ct, McpHandler::Sse(handler))
        }

        // ================ Client uses Streamable HTTP protocol ================
        McpProtocol::Stream => {
            // Build backend config for Stream
            let backend_config = build_stream_backend_config(&mcp_config, backend_protocol)?;

            let (router, ct, handler) = StreamServerBuilder::new(backend_config)
                .mcp_id(mcp_id.clone())
                .stateful(false)
                .build()
                .await?;

            info!(
                "Streamable HTTP server started - MCP ID: {}, type: {:?}",
                mcp_router_path.mcp_id, mcp_type
            );

            (router, ct, McpHandler::Stream(handler))
        }

        // Client stdio protocol is not supported in server mode
        McpProtocol::Stdio => {
            return Err(anyhow::anyhow!(
                "Client protocol cannot be Stdio. McpRouterPath::new does not support creating Stdio protocol router paths"
            ));
        }
    };

    // Clone cancellation token for monitoring
    let ct_clone = ct.clone();
    let mcp_id_clone = mcp_id.clone();

    // Store MCP service status
    let mcp_service_status = McpServiceStatus::new(
        mcp_id_clone.clone(),
        mcp_type.clone(),
        mcp_router_path.clone(),
        ct_clone.clone(),
        CheckMcpStatusResponseStatus::Ready,
    );

    // Add MCP service status and proxy handler to global manager
    let proxy_manager = get_proxy_manager();
    proxy_manager.add_mcp_service_status_and_proxy(mcp_service_status, Some(handler));

    // Add base path fallback handler for SSE protocol
    let router = if matches!(mcp_router_path.mcp_protocol, McpProtocol::Sse) {
        let modified_router = router.fallback(base_path_fallback_handler);
        info!("SSE base path handler added, base_path: {}", base_path);
        modified_router
    } else {
        router
    };

    // Register route to global route table
    info!("Registering route: base_path={}, mcp_id={}", base_path, mcp_id);
    info!(
        "SSE path config: sse_path={}, post_path={}",
        match &mcp_router_path.mcp_protocol_path {
            McpProtocolPath::SsePath(sse_path) => &sse_path.sse_path,
            _ => "N/A",
        },
        match &mcp_router_path.mcp_protocol_path {
            McpProtocolPath::SsePath(sse_path) => &sse_path.message_path,
            _ => "N/A",
        }
    );
    DynamicRouterService::register_route(&base_path, router.clone());
    info!("Route registration complete: base_path={}", base_path);

    Ok((router, ct))
}

/// Build SSE backend configuration from MCP server config
fn build_sse_backend_config(
    mcp_config: &McpServerConfig,
    backend_protocol: McpProtocol,
) -> Result<SseBackendConfig> {
    match mcp_config {
        McpServerConfig::Command(cmd_config) => {
            log_command_details(cmd_config);
            Ok(SseBackendConfig::Stdio {
                command: cmd_config.command.clone(),
                args: cmd_config.args.clone(),
                env: cmd_config.env.clone(),
            })
        }
        McpServerConfig::Url(url_config) => {
            match backend_protocol {
                McpProtocol::Stdio => {
                    Err(anyhow::anyhow!("URL-based MCP service cannot use Stdio protocol"))
                }
                McpProtocol::Sse => {
                    info!("Connecting to SSE backend: {}", url_config.get_url());
                    Ok(SseBackendConfig::SseUrl {
                        url: url_config.get_url().to_string(),
                        headers: url_config.headers.clone(),
                    })
                }
                McpProtocol::Stream => {
                    info!("Connecting to Streamable HTTP backend (SSE frontend): {}", url_config.get_url());
                    Ok(SseBackendConfig::StreamUrl {
                        url: url_config.get_url().to_string(),
                        headers: url_config.headers.clone(),
                    })
                }
            }
        }
    }
}

/// Build Stream backend configuration from MCP server config
fn build_stream_backend_config(
    mcp_config: &McpServerConfig,
    backend_protocol: McpProtocol,
) -> Result<StreamBackendConfig> {
    match mcp_config {
        McpServerConfig::Command(cmd_config) => {
            log_command_details(cmd_config);
            Ok(StreamBackendConfig::Stdio {
                command: cmd_config.command.clone(),
                args: cmd_config.args.clone(),
                env: cmd_config.env.clone(),
            })
        }
        McpServerConfig::Url(url_config) => {
            match backend_protocol {
                McpProtocol::Stdio => {
                    Err(anyhow::anyhow!("URL-based MCP service cannot use Stdio protocol"))
                }
                McpProtocol::Sse => {
                    // Note: StreamServerBuilder currently only supports Streamable HTTP URL backend
                    // SSE backend with Stream frontend would require protocol conversion
                    // For now, we return an error for this combination
                    Err(anyhow::anyhow!(
                        "SSE backend with Streamable HTTP frontend is not yet supported. \
                         Please use SSE frontend or configure a Streamable HTTP backend."
                    ))
                }
                McpProtocol::Stream => {
                    info!("Connecting to Streamable HTTP backend: {}", url_config.get_url());
                    Ok(StreamBackendConfig::Url {
                        url: url_config.get_url().to_string(),
                        headers: url_config.headers.clone(),
                    })
                }
            }
        }
    }
}

/// Log command execution details for debugging
fn log_command_details(mcp_config: &McpServerCommandConfig) {
    let args_str = mcp_config
        .args
        .as_ref()
        .map_or(String::new(), |args| args.join(" "));
    let cmd_str = format!("Executing command: {} {}", mcp_config.command, args_str);
    debug!("{cmd_str}");

    if let Some(env_vars) = &mcp_config.env {
        let env_vars: Vec<String> = env_vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
        if !env_vars.is_empty() {
            debug!("Environment variables: {}", env_vars.join(", "));
        }
    }

    debug!("Full command: {:?}", mcp_config.command);

    let env_str = mcp_config.env.as_ref().map_or(String::new(), |env| {
        env.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<String>>()
            .join(" ")
    });

    let full_command = format!("{} {} {}", mcp_config.command, args_str, env_str);
    info!("Full command string: {:?}", full_command);
}

/// Base path fallback handler - supports direct access to base path with automatic redirection
#[axum::debug_handler]
async fn base_path_fallback_handler(
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
) -> impl axum::response::IntoResponse {
    let path = uri.path();
    info!("Base path handler: {} {}", method, path);

    // Determine if SSE or Stream protocol
    if path.contains("/sse/proxy/") {
        // SSE protocol handling
        match method {
            axum::http::Method::GET => {
                // Extract MCP ID from path
                let mcp_id = path.split("/sse/proxy/").nth(1);

                if let Some(mcp_id) = mcp_id {
                    // Check if MCP service exists
                    let proxy_manager = get_proxy_manager();
                    if proxy_manager.get_mcp_service_status(mcp_id).is_none() {
                        // MCP service not found
                        (
                            axum::http::StatusCode::NOT_FOUND,
                            [("Content-Type", "text/plain".to_string())],
                            format!("MCP service '{}' not found", mcp_id).to_string(),
                        )
                    } else {
                        // MCP service exists, check Accept header
                        let accept_header = headers.get("accept");
                        if let Some(accept) = accept_header {
                            let accept_str = accept.to_str().unwrap_or("");
                            if accept_str.contains("text/event-stream") {
                                // Correct Accept header, redirect to /sse
                                let redirect_uri = format!("{}/sse", path);
                                info!("SSE redirect to: {}", redirect_uri);
                                (
                                    axum::http::StatusCode::FOUND,
                                    [("Location", redirect_uri.to_string())],
                                    "Redirecting to SSE endpoint".to_string(),
                                )
                            } else {
                                // Incorrect Accept header
                                (
                                    axum::http::StatusCode::BAD_REQUEST,
                                    [("Content-Type", "text/plain".to_string())],
                                    "SSE error: Invalid Accept header, expected 'text/event-stream'".to_string(),
                                )
                            }
                        } else {
                            // No Accept header
                            (
                                axum::http::StatusCode::BAD_REQUEST,
                                [("Content-Type", "text/plain".to_string())],
                                "SSE error: Missing Accept header, expected 'text/event-stream'"
                                    .to_string(),
                            )
                        }
                    }
                } else {
                    // Cannot extract MCP ID from path
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        [("Content-Type", "text/plain".to_string())],
                        "SSE error: Invalid SSE path".to_string(),
                    )
                }
            }
            axum::http::Method::POST => {
                // POST request redirect to /message
                let redirect_uri = format!("{}/message", path);
                info!("SSE redirect to: {}", redirect_uri);
                (
                    axum::http::StatusCode::FOUND,
                    [("Location", redirect_uri.to_string())],
                    "Redirecting to message endpoint".to_string(),
                )
            }
            _ => {
                // Other methods return 405 Method Not Allowed
                (
                    axum::http::StatusCode::METHOD_NOT_ALLOWED,
                    [("Allow", "GET, POST".to_string())],
                    "Only GET and POST methods are allowed".to_string(),
                )
            }
        }
    } else if path.contains("/stream/proxy/") {
        // Stream protocol handling - return success directly without redirect
        match method {
            axum::http::Method::GET => {
                // GET request returns server info
                (
                    axum::http::StatusCode::OK,
                    [("Content-Type", "application/json".to_string())],
                    r#"{"jsonrpc":"2.0","result":{"info":"Streamable MCP Server","version":"1.0"}}"#.to_string(),
                )
            }
            axum::http::Method::POST => {
                // POST request returns success, let StreamableHttpService handle
                (
                    axum::http::StatusCode::OK,
                    [("Content-Type", "application/json".to_string())],
                    r#"{"jsonrpc":"2.0","result":{"message":"Stream request received","protocol":"streamable-http"}}"#.to_string(),
                )
            }
            _ => {
                // Other methods return 405 Method Not Allowed
                (
                    axum::http::StatusCode::METHOD_NOT_ALLOWED,
                    [("Allow", "GET, POST".to_string())],
                    "Only GET and POST methods are allowed".to_string(),
                )
            }
        }
    } else {
        // Unknown protocol
        (
            axum::http::StatusCode::BAD_REQUEST,
            [("Content-Type", "text/plain".to_string())],
            "Unknown protocol or path".to_string(),
        )
    }
}
