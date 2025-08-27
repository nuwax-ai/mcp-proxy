use crate::config::{ConfigTemplateGenerator, ServiceType};
use crate::config_rs_integration::{ConfigRsLoader, CliOverrides};
use crate::load_balancer::VoiceCliLoadBalancer;
use crate::models::Config;
use crate::models::MetadataStore;
use crate::{Result, VoiceCliError};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info};

/// Initialize load balancer configuration
pub async fn handle_lb_init(
    config_path: Option<PathBuf>,
    port: Option<u16>,
    force: bool,
) -> crate::Result<()> {
    let output_path = config_path.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("lb-config.yml")
    });

    // 检查文件是否已存在
    if output_path.exists() && !force {
        println!("❌ Configuration file already exists: {:?}", output_path);
        println!("💡 Use --force to overwrite, or specify a different path with --config");
        return Ok(());
    }

    // 生成基础配置
    ConfigTemplateGenerator::generate_config_file(ServiceType::LoadBalancer, &output_path)?;

    // 如果指定了端口参数，更新配置文件
    if let Some(port) = port {
        // 使用新的 config-rs 加载器加载配置
        let cli_overrides = CliOverrides {
            lb_port: Some(port),
            ..Default::default()
        };
        let config = ConfigRsLoader::load(Some(&output_path), &cli_overrides, Some(ServiceType::LoadBalancer))?;
        config
            .save(&output_path)
            .map_err(|e| crate::VoiceCliError::Config(e.to_string()))?;
    }

    println!(
        "✅ Load balancer configuration initialized: {:?}",
        output_path
    );
    if let Some(port) = port {
        println!("🔧 Port set to: {}", port);
    }
    println!("📝 Edit the configuration file and run:");
    println!("   voice-cli lb start --config {:?}", output_path);

    Ok(())
}

/// Handle load balancer run command (foreground mode)
pub async fn handle_lb_run(config: &Config, port: u16, health_check_interval: u64) -> Result<()> {
    info!("Starting load balancer in foreground mode on port {}", port);

    info!("Load balancer configuration:");
    info!("  Port: {}", port);
    info!("  Health Check Interval: {}s", health_check_interval);
    info!("  Metadata Store: {}", config.cluster.metadata_db_path);
    info!("  PID File: {}", config.load_balancer.pid_file);

    // Initialize metadata store for cluster information
    let metadata_store = Arc::new(
        MetadataStore::new(&config.cluster.metadata_db_path).map_err(|e| {
            VoiceCliError::Config(format!("Failed to initialize metadata store: {}", e))
        })?,
    );

    // Create and configure the load balancer service using existing config
    let mut lb_config = config.load_balancer.clone();
    lb_config.port = port;
    lb_config.health_check_interval = health_check_interval;
    lb_config.enabled = true;

    let mut lb_service = VoiceCliLoadBalancer::new(lb_config, metadata_store)
        .await
        .map_err(|e| VoiceCliError::Config(format!("Failed to create load balancer: {}", e)))?;

    // Set up graceful shutdown
    let shutdown_signal = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install CTRL+C signal handler");
        info!("Received shutdown signal, stopping load balancer...");
    };

    // Start the load balancer service
    info!("✅ Load balancer starting on port {}", port);
    info!("   Health check interval: {}s", health_check_interval);
    info!("   Cluster metadata: {}", config.cluster.metadata_db_path);

    tokio::select! {
        result = lb_service.start() => {
            match result {
                Ok(_) => info!("Load balancer stopped normally"),
                Err(e) => {
                    error!("Load balancer error: {}", e);
                    return Err(VoiceCliError::Config(format!("Load balancer failed: {}", e)));
                }
            }
        }
        _ = shutdown_signal => {
            info!("Graceful shutdown initiated");
        }
    }

    info!("Load balancer stopped");
    Ok(())
}


/// Check if load balancer is running on the specified port
async fn is_lb_running(port: u16) -> Result<bool> {
    Ok(is_port_responding(port).await)
}

/// Check if a port is responding
async fn is_port_responding(port: u16) -> bool {
    tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .is_ok()
}

/// Get configuration path from command line arguments
fn get_config_path_from_args() -> String {
    std::env::args()
        .collect::<Vec<String>>()
        .windows(2)
        .find(|window| window[0] == "--config" || window[0] == "-c")
        .map(|window| window[1].clone())
        .unwrap_or_else(|| "config.yml".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_is_port_responding() {
        // Test with a port that's definitely not in use
        assert!(!is_port_responding(65534).await);
    }
}
