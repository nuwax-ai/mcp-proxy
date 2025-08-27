use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use tracing::{error, info, warn};
use tracing_subscriber;
use voice_cli::{
    cli::{
        Cli, ClusterAction, Commands, LoadBalancerAction, ModelAction, ServerAction,
    },
    config::ServiceType,
    config_rs_integration::ConfigRsLoader,
    log_cluster_event,
    utils::{init_structured_logging, ClusterLoggingContext},
};

#[tokio::main]
async fn main() {
    // Parse command line arguments
    let cli = Cli::parse();

    // Initialize basic console logging for CLI operations
    // This is console-only and will be replaced by proper file logging when services start
    init_console_only_logging(cli.verbose);

    // Generate CLI overrides from command line arguments
    let cli_overrides = match ConfigRsLoader::generate_cli_overrides_from_args(&cli) {
        Ok(overrides) => overrides,
        Err(e) => {
            error!("Failed to generate CLI overrides: {}", e);
            std::process::exit(1);
        }
    };

    // Load configuration based on command type using config-rs with proper hierarchy
    let config = match &cli.command {
        // For init commands, we don't need to load existing config
        Commands::Server {
            action: ServerAction::Init { .. },
        }
        | Commands::Cluster {
            action: ClusterAction::Init { .. },
        }
        | Commands::Lb {
            action: LoadBalancerAction::Init { .. },
        } => {
            // Use default config for init commands
            voice_cli::Config::default()
        }

        // For server commands, use server-specific config
        Commands::Server { action } => {
            let config_path = get_config_path_for_server_action(action, &cli.config);
            match ConfigRsLoader::load(config_path.as_ref(), &cli_overrides, Some(ServiceType::Server)) {
                Ok(config) => config,
                Err(e) => {
                    error!("Failed to load server configuration: {}", e);
                    std::process::exit(1);
                }
            }
        }

        // For cluster commands, use cluster-specific config
        Commands::Cluster { action } => {
            let config_path = get_config_path_for_cluster_action(action, &cli.config);
            match ConfigRsLoader::load(config_path.as_ref(), &cli_overrides, Some(ServiceType::Cluster)) {
                Ok(config) => config,
                Err(e) => {
                    error!("Failed to load cluster configuration: {}", e);
                    std::process::exit(1);
                }
            }
        }

        // For load balancer commands, use load balancer-specific config
        Commands::Lb { action } => {
            let config_path = get_config_path_for_lb_action(action, &cli.config);
            match ConfigRsLoader::load(config_path.as_ref(), &cli_overrides, Some(ServiceType::LoadBalancer)) {
                Ok(config) => config,
                Err(e) => {
                    error!("Failed to load load balancer configuration: {}", e);
                    std::process::exit(1);
                }
            }
        }

        // For other commands, use default config loading
        _ => {
            let config_path = PathBuf::from(&cli.config);
            match ConfigRsLoader::load(Some(&config_path), &cli_overrides, None) {
                Ok(config) => config,
                Err(e) => {
                    error!("Failed to load configuration: {}", e);
                    std::process::exit(1);
                }
            }
        }
    };

    // Log configuration summary if verbose
    if cli.verbose {
        info!("Configuration loaded successfully");
    }

    // Route to appropriate handler
    let result = match cli.command {
        Commands::Server { action } => handle_server_command(action, &config).await,
        Commands::Model { action } => handle_model_command(action, &config).await,
        Commands::Cluster { action } => handle_cluster_command(action, &config).await,
        Commands::Lb { action } => handle_lb_command(action, &config).await,
    };

    // Handle result
    match result {
        Ok(_) => {
            info!("Command completed successfully");
        }
        Err(e) => {
            // Print error to stderr to ensure it's always visible
            eprintln!("❌ Error: {}", e);
            
            // Also print the error chain if available
            let mut current_error = e.source();
            while let Some(err) = current_error {
                eprintln!("   Caused by: {}", err);
                current_error = err.source();
            }
            
            // Also log the error
            error!("Command failed: {}", e);
            std::process::exit(1);
        }
    }
}

/// Handle server-related commands
async fn handle_server_command(action: ServerAction, config: &voice_cli::Config) -> Result<()> {
    use voice_cli::cli::server;

    match action {
        ServerAction::Init {
            config: config_path,
            force,
        } => {
            info!("Initializing server configuration");
            server::handle_server_init(config_path, force)
                .await
                .context("Failed to initialize server configuration")
        }
        ServerAction::Run { config: _ } => {
            info!("Running server in foreground mode");
            server::handle_server_run(config)
                .await
                .context("Failed to run server")
        }
    }
}

/// Handle model-related commands
async fn handle_model_command(action: ModelAction, config: &voice_cli::Config) -> Result<()> {
    use voice_cli::cli::model;

    match action {
        ModelAction::Download { model_name } => {
            info!("Downloading model: {}", model_name);
            model::handle_model_download(config, &model_name)
                .await
                .context("Failed to download model")
        }
        ModelAction::List => {
            info!("Listing models");
            model::handle_model_list(config)
                .await
                .context("Failed to list models")
        }
        ModelAction::Validate => {
            info!("Validating models");
            model::handle_model_validate(config)
                .await
                .context("Failed to validate models")
        }
        ModelAction::Remove { model_name } => {
            info!("Removing model: {}", model_name);
            model::handle_model_remove(config, &model_name)
                .await
                .context("Failed to remove model")
        }
        ModelAction::Diagnose { model_name } => {
            info!("Diagnosing model: {}", model_name);
            model::handle_model_diagnose(config, &model_name)
                .await
                .context("Failed to diagnose model")
        }
    }
}


/// Handle cluster-related commands
async fn handle_cluster_command(action: ClusterAction, config: &voice_cli::Config) -> Result<()> {
    use voice_cli::cli::cluster;

    match action {
        ClusterAction::Init {
            config: config_path,
            http_port,
            grpc_port,
            force,
        } => {
            info!("Initializing cluster configuration");
            cluster::handle_cluster_init(config_path, http_port, grpc_port, force)
                .await
                .context("Failed to initialize cluster configuration")
        }
        ClusterAction::Run {
            config: _,
            node_id,
            http_port,
            grpc_port,
            can_process_tasks,
            advertise_ip,
        } => {
            // Initialize structured logging for cluster operations
            let node_id_str = node_id.as_deref().unwrap_or("auto-generated");
            let logging_context =
                ClusterLoggingContext::new(node_id_str.to_string(), "cluster_node".to_string());

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
                http_port.unwrap_or(config.cluster.http_port),
                grpc_port.unwrap_or(config.cluster.grpc_port),
                can_process_tasks,
                advertise_ip,
            )
            .await
            .context("Failed to run cluster node")
        }
        ClusterAction::Join {
            peer_address,
            advertise_ip,
            node_id,
            http_port,
            grpc_port,
            token,
        } => {
            info!(
                "Joining cluster: peer={}, advertise_ip={}, node_id={:?}, http_port={:?}, grpc_port={:?}",
                peer_address, advertise_ip, node_id, http_port, grpc_port
            );
            cluster::handle_cluster_join(
                config,
                peer_address,
                node_id,
                http_port.unwrap_or(config.cluster.http_port),
                grpc_port.unwrap_or(config.cluster.grpc_port),
                token,
                Some(advertise_ip),
            )
            .await
            .context("Failed to join cluster")
        }
        ClusterAction::Status { detailed } => {
            info!("Getting cluster status: detailed={}", detailed);
            cluster::handle_cluster_status(config, detailed)
                .await
                .context("Failed to get cluster status")
        }
        ClusterAction::GenerateConfig { output, template } => {
            info!(
                "Generating cluster config: output={:?}, template={}",
                output, template
            );
            cluster::handle_generate_config(config, output, template)
                .await
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
            )
            .await
            .context("Failed to install systemd service")
        }
        ClusterAction::UninstallService { service_name } => {
            info!("Uninstalling systemd service: {}", service_name);
            cluster::handle_uninstall_service(service_name)
                .await
                .context("Failed to uninstall systemd service")
        }
        ClusterAction::ServiceStatus { service_name } => {
            info!("Checking systemd service status: {}", service_name);
            cluster::handle_service_status(service_name)
                .await
                .context("Failed to check systemd service status")
        }
    }
}

/// Handle load balancer-related commands
async fn handle_lb_command(action: LoadBalancerAction, config: &voice_cli::Config) -> Result<()> {
    use voice_cli::cli::lb;

    match action {
        LoadBalancerAction::Init {
            config: config_path,
            port,
            force,
        } => {
            info!("Initializing load balancer configuration");
            lb::handle_lb_init(config_path, port, force)
                .await
                .context("Failed to initialize load balancer configuration")
        }
        LoadBalancerAction::Run {
            config: _,
            port,
            health_check_interval,
        } => {
            let port = port.unwrap_or(8090);
            let health_check_interval = health_check_interval.unwrap_or(10);
            info!(
                "Running load balancer: port={}, health_check_interval={}s",
                port, health_check_interval
            );
            lb::handle_lb_run(config, port, health_check_interval)
                .await
                .context("Failed to run load balancer")
        }
    }
}

/// Initialize console-only logging for CLI operations (before full config is loaded)
/// This uses a simple println-based approach to avoid interfering with proper logging setup
fn init_console_only_logging(verbose: bool) {
    // Initialize basic console logging for CLI operations
    // This ensures error messages are always visible
    let level = if verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    
    let subscriber = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(level)
        .with_ansi(true)
        .finish();
    
    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set global tracing subscriber");
    
    if verbose {
        info!("🔧 Verbose mode enabled - CLI operations will show debug output");
    }
}

/// Extract config path from server action
fn get_config_path_for_server_action(
    action: &ServerAction,
    default_config: &str,
) -> Option<PathBuf> {
    match action {
        ServerAction::Run { config } => config.clone(),
        _ => None,
    }
}

/// Extract config path from cluster action
fn get_config_path_for_cluster_action(
    action: &ClusterAction,
    default_config: &str,
) -> Option<PathBuf> {
    match action {
        ClusterAction::Run { config, .. } => config.clone(),
        _ => {
            // For other actions, check if default config is not the fallback
            if default_config != "config.yml" {
                Some(PathBuf::from(default_config))
            } else {
                None
            }
        }
    }
}

/// Extract config path from load balancer action
fn get_config_path_for_lb_action(
    action: &LoadBalancerAction,
    default_config: &str,
) -> Option<PathBuf> {
    match action {
        LoadBalancerAction::Run { config, .. } => config.clone(),
        _ => {
            // For other actions, check if default config is not the fallback
            if default_config != "config.yml" {
                Some(PathBuf::from(default_config))
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
