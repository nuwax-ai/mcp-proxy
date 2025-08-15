use crate::{
    model::{McpServerCommandConfig, SseServerSettings},
    proxy::ProxyHandler,
};

/**
 * Create a local SSE server that proxies requests to a stdio MCP server.
 */
use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo},
    transport::{
        child_process::TokioChildProcess,
        sse_server::{SseServer, SseServerConfig},
    },
};
use std::error::Error as StdError;
use tokio::process::Command;
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// Run the SSE server with a stdio client
///
/// This function connects to a stdio server and exposes it as an SSE server.
#[allow(dead_code)]
pub async fn run_sse_server(
    stdio_params: McpServerCommandConfig,
    sse_settings: SseServerSettings,
) -> Result<(), Box<dyn StdError>> {
    info!(
        "Running SSE server on {:?} with command: {}",
        sse_settings.bind_addr, stdio_params.command,
    );

    // Configure SSE server
    let config = SseServerConfig {
        bind: sse_settings.bind_addr,
        sse_path: "/sse".to_string(),
        post_path: "/message".to_string(),
        ct: CancellationToken::new(),
        sse_keep_alive: sse_settings.keep_alive,
    };

    let mut command = Command::new(&stdio_params.command);

    // 正确处理Option<Vec<String>>
    if let Some(args) = &stdio_params.args {
        command.args(args);
    }

    // 正确处理Option<HashMap<String, String>>
    if let Some(env_vars) = &stdio_params.env {
        for (key, value) in env_vars {
            command.env(key, value);
        }
    }

    // Create child process
    let tokio_process = TokioChildProcess::new(command)?;

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

    // Create client service
    let client = client_info.serve(tokio_process).await?;

    // Get server info
    let server_info = client.peer_info();
    info!("Connected to server: {server_info:#?}");

    // Create proxy handler
    let proxy_handler = ProxyHandler::new(client);

    // Start the SSE server
    let sse_server = SseServer::serve_with_config(config.clone()).await?;

    // Register the proxy handler with the SSE server
    let ct = sse_server.with_service(move || proxy_handler.clone());

    // Wait for Ctrl+C to shut down
    signal::ctrl_c().await?;
    ct.cancel();

    Ok(())
}
