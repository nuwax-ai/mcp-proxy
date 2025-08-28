use crate::config::{ConfigTemplateGenerator, ServiceType};
use crate::models::Config;
use crate::VoiceCliError;
use std::path::PathBuf;
use tracing::info;

/// Initialize server configuration
pub async fn handle_server_init(config_path: Option<PathBuf>, force: bool) -> crate::Result<()> {
    let output_path = config_path.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("server-config.yml")
    });

    // 检查文件是否已存在
    if output_path.exists() && !force {
        println!("❌ Configuration file already exists: {:?}", output_path);
        println!("💡 Use --force to overwrite, or specify a different path with --config");
        return Ok(());
    }

    // 生成配置文件
    ConfigTemplateGenerator::generate_config_file(ServiceType::Server, &output_path)?;

    println!("✅ Server configuration initialized: {:?}", output_path);
    println!("📝 Edit the configuration file and run:");
    println!("   voice-cli server run --config {:?}", output_path);

    Ok(())
}

/// Run server in foreground mode (direct HTTP server)
pub async fn handle_server_run(config: &Config) -> crate::Result<()> {
    info!("Starting voice-cli server in foreground mode...");

    // Initialize logging - keep the guard alive for the duration of the process
    info!("About to initialize logging...");
    crate::utils::init_logging(config)?;
    info!("Logging initialized successfully");

    info!("About to create server...");
    // Start the HTTP server
    let server = crate::server::create_server_with_graceful_shutdown(config.clone()).await?;
    info!("Server created successfully");

    info!(
        "Server running on {}:{}",
        config.server.host, config.server.port
    );
    info!("Press Ctrl+C to stop the server");

    // Run server with graceful shutdown
    server
        .await
        .map_err(|e| VoiceCliError::Config(format!("Server error: {}", e)))?;

    Ok(())
}

// Background mode is no longer supported.
// Use foreground mode with shell scripts for background operation.

