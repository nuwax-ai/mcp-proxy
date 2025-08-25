use clap::Parser;
use std::path::PathBuf;
use tracing::{info, error, warn};
use anyhow::{Context, Result};
use voice_cli::{
    cli::{Cli, Commands, ServerAction, ModelAction, DaemonAction, ClusterAction, LoadBalancerAction},
    config::ConfigManager,
    utils::{ClusterLoggingContext, init_structured_logging},
    log_cluster_event,
};

#[tokio::main]
async fn main() {
    // Parse command line arguments
    let cli = Cli::parse();
    
    // Initialize basic logging for CLI operations
    init_basic_logging(cli.verbose);
    
    // Load or create configuration
    let config_path = PathBuf::from(&cli.config);
    let config_manager = match ConfigManager::new(config_path) {
        Ok(manager) => manager,
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };
    
    let config = config_manager.config().await;
    
    // Log configuration summary
    if cli.verbose {
        info!("{}", config_manager.get_summary().await);
    }
    
    // Validate environment
    if let Err(e) = config_manager.validate_environment().await {
        warn!("Environment validation warning: {}", e);
    }
    
    // Route to appropriate handler
    let result = match cli.command {
        Commands::Server { action } => handle_server_command(action, &config).await,
        Commands::Model { action } => handle_model_command(action, &config).await,
        Commands::Cluster { action } => handle_cluster_command(action, &config).await,
        Commands::Lb { action } => handle_lb_command(action, &config).await,
        Commands::Daemon { action } => handle_daemon_command(action, &config).await,
    };
    
    // Handle result
    match result {
        Ok(_) => {
            info!("Command completed successfully");
        }
        Err(e) => {
            error!("Command failed: {}", e);
            std::process::exit(1);
        }
    }
}

/// Handle server-related commands
async fn handle_server_command(action: ServerAction, config: &voice_cli::Config) -> Result<()> {
    use voice_cli::cli::server;
    
    match action {
        ServerAction::Run => {
            info!("Running server in foreground mode");
            server::handle_server_run(config).await
                .context("Failed to run server")
        }
        ServerAction::Start => {
            info!("Starting server in background mode");
            server::handle_server_start(config).await
                .context("Failed to start server")
        }
        ServerAction::Stop => {
            info!("Stopping server");
            server::handle_server_stop(config).await
                .context("Failed to stop server")
        }
        ServerAction::Restart => {
            info!("Restarting server");
            server::handle_server_restart(config).await
                .context("Failed to restart server")
        }
        ServerAction::Status => {
            info!("Checking server status");
            server::handle_server_status(config).await
                .context("Failed to check server status")
        }
    }
}

/// Handle model-related commands
async fn handle_model_command(action: ModelAction, config: &voice_cli::Config) -> Result<()> {
    use voice_cli::cli::model;
    
    match action {
        ModelAction::Download { model_name } => {
            info!("Downloading model: {}", model_name);
            model::handle_model_download(config, &model_name).await
                .context("Failed to download model")
        }
        ModelAction::List => {
            info!("Listing models");
            model::handle_model_list(config).await
                .context("Failed to list models")
        }
        ModelAction::Validate => {
            info!("Validating models");
            model::handle_model_validate(config).await
                .context("Failed to validate models")
        }
        ModelAction::Remove { model_name } => {
            info!("Removing model: {}", model_name);
            model::handle_model_remove(config, &model_name).await
                .context("Failed to remove model")
        }
        ModelAction::Diagnose { model_name } => {
            info!("Diagnosing model: {}", model_name);
            model::handle_model_diagnose(config, &model_name).await
                .context("Failed to diagnose model")
        }
    }
}

/// Handle daemon-related commands (internal use)
async fn handle_daemon_command(action: DaemonAction, config: &voice_cli::Config) -> Result<()> {
    use voice_cli::cli::server;
    
    match action {
        DaemonAction::Serve => {
            // This is the internal command called by the daemon process
            server::handle_daemon_serve(config).await
                .context("Failed to serve daemon")
        }
    }
}

/// Handle cluster-related commands
async fn handle_cluster_command(action: ClusterAction, config: &voice_cli::Config) -> Result<()> {
    use voice_cli::cli::cluster;
    
    match action {
        ClusterAction::Run {
            node_id,
            http_port,
            grpc_port,
            can_process_tasks,
        } => {
            // Initialize structured logging for cluster operations
            let node_id_str = node_id.as_deref().unwrap_or("auto-generated");
            let logging_context = ClusterLoggingContext::new(
                node_id_str.to_string(),
                "cluster_node".to_string()
            );
            
            // Re-initialize logging with structured context
            if let Err(e) = init_structured_logging(&config, logging_context) {
                warn!("Failed to initialize structured logging: {}", e);
            }
            
            log_cluster_event!(
                info,
                node_id_str,
                "cluster_node",
                "run_command",
                "Running cluster node",
                http_port = http_port,
                grpc_port = grpc_port,
                can_process_tasks = can_process_tasks
            );
            
            cluster::handle_cluster_run(
                config,
                node_id,
                http_port,
                grpc_port,
                can_process_tasks,
            ).await
                .context("Failed to run cluster node")
        }
        ClusterAction::Start {
            node_id,
            http_port,
            grpc_port,
            can_process_tasks,
        } => {
            info!(
                "Starting cluster node: node_id={:?}, http_port={}, grpc_port={}, can_process_tasks={}",
                node_id, http_port, grpc_port, can_process_tasks
            );
            cluster::handle_cluster_start(
                config,
                node_id,
                http_port,
                grpc_port,
                can_process_tasks,
            ).await
                .context("Failed to start cluster node")
        }
        ClusterAction::Stop => {
            info!("Stopping cluster node");
            cluster::handle_cluster_stop(config).await
                .context("Failed to stop cluster node")
        }
        ClusterAction::Restart {
            node_id,
            http_port,
            grpc_port,
            can_process_tasks,
        } => {
            info!(
                "Restarting cluster node: node_id={:?}, http_port={}, grpc_port={}, can_process_tasks={}",
                node_id, http_port, grpc_port, can_process_tasks
            );
            cluster::handle_cluster_restart(
                config,
                node_id,
                http_port,
                grpc_port,
                can_process_tasks,
            ).await
                .context("Failed to restart cluster node")
        }
        ClusterAction::Init {
            node_id,
            http_port,
            grpc_port,
            leader_can_process_tasks,
        } => {
            info!(
                "Initializing cluster: node_id={:?}, http_port={}, grpc_port={}, leader_can_process_tasks={}",
                node_id, http_port, grpc_port, leader_can_process_tasks
            );
            cluster::handle_cluster_init(
                config,
                node_id,
                http_port,
                grpc_port,
                leader_can_process_tasks,
            ).await
                .context("Failed to initialize cluster")
        }
        ClusterAction::Join {
            peer_address,
            node_id,
            http_port,
            grpc_port,
            token,
        } => {
            info!(
                "Joining cluster: peer={}, node_id={:?}, http_port={}, grpc_port={}",
                peer_address, node_id, http_port, grpc_port
            );
            cluster::handle_cluster_join(
                config,
                peer_address,
                node_id,
                http_port,
                grpc_port,
                token,
            ).await
                .context("Failed to join cluster")
        }
        ClusterAction::Status { detailed } => {
            info!("Getting cluster status: detailed={}", detailed);
            cluster::handle_cluster_status(config, detailed).await
                .context("Failed to get cluster status")
        }
        ClusterAction::GenerateConfig { output, template } => {
            info!("Generating cluster config: output={:?}, template={}", output, template);
            cluster::handle_generate_config(config, output, template).await
                .context("Failed to generate cluster config")
        }
        ClusterAction::InstallService {
            service_name,
            node_id,
            http_port,
            grpc_port,
            can_process_tasks,
            memory_limit,
            cpu_limit,
            user,
            group,
        } => {
            info!("Installing systemd service: {}", service_name);
            cluster::handle_install_service(
                config,
                service_name,
                node_id,
                http_port,
                grpc_port,
                can_process_tasks,
                memory_limit,
                cpu_limit,
                user,
                group,
            ).await
                .context("Failed to install systemd service")
        }
        ClusterAction::UninstallService { service_name } => {
            info!("Uninstalling systemd service: {}", service_name);
            cluster::handle_uninstall_service(service_name).await
                .context("Failed to uninstall systemd service")
        }
        ClusterAction::ServiceStatus { service_name } => {
            info!("Checking systemd service status: {}", service_name);
            cluster::handle_service_status(service_name).await
                .context("Failed to check systemd service status")
        }
    }
}

/// Handle load balancer-related commands
async fn handle_lb_command(action: LoadBalancerAction, config: &voice_cli::Config) -> Result<()> {
    use voice_cli::cli::lb;
    
    match action {
        LoadBalancerAction::Run {
            port,
            health_check_interval,
        } => {
            info!("Running load balancer: port={}, health_check_interval={}s", port, health_check_interval);
            lb::handle_lb_run(config, port, health_check_interval).await
                .context("Failed to run load balancer")
        }
        LoadBalancerAction::Start { port } => {
            info!("Starting load balancer: port={}", port);
            lb::handle_lb_start(config, port).await
                .context("Failed to start load balancer")
        }
        LoadBalancerAction::Stop => {
            info!("Stopping load balancer");
            lb::handle_lb_stop(config).await
                .context("Failed to stop load balancer")
        }
        LoadBalancerAction::Restart { port } => {
            info!("Restarting load balancer: port={}", port);
            lb::handle_lb_restart(config, port).await
                .context("Failed to restart load balancer")
        }
        LoadBalancerAction::Status => {
            info!("Checking load balancer status");
            lb::handle_lb_status(config).await
                .context("Failed to check load balancer status")
        }
    }
}


/// Initialize basic logging for CLI operations (before full config is loaded)
fn init_basic_logging(verbose: bool) {
    use tracing_subscriber::{filter::LevelFilter, prelude::*};
    
    let level = if verbose {
        LevelFilter::DEBUG
    } else {
        LevelFilter::INFO
    };
    
    let console_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .compact()
        .with_filter(level);
    
    tracing_subscriber::registry()
        .with(console_layer)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_config_loading() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("test_config.yml");
        
        let config_manager = ConfigManager::new(config_path);
        assert!(config_manager.is_ok());
    }

    #[test]
    fn test_cli_parsing() {
        use clap::Parser;
        
        // Test server run command
        let args = vec!["voice-cli", "server", "run"];
        let cli = Cli::try_parse_from(args);
        assert!(cli.is_ok());
        
        // Test model download command
        let args = vec!["voice-cli", "model", "download", "base"];
        let cli = Cli::try_parse_from(args);
        assert!(cli.is_ok());
    }
}