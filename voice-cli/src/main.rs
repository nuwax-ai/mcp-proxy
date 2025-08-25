use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use tracing::{error, info, warn};
use voice_cli::{
    cli::{
        Cli, ClusterAction, Commands, DaemonAction, LoadBalancerAction, ModelAction, ServerAction,
    },
    config::{ConfigManager, ServiceConfigLoader, ServiceType},
    log_cluster_event,
    utils::{init_structured_logging, ClusterLoggingContext},
};

#[tokio::main]
async fn main() {
    // Parse command line arguments
    let cli = Cli::parse();

    // Initialize basic logging for CLI operations
    init_basic_logging(cli.verbose);

    // Load configuration based on command type
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
            match get_config_path_for_server_action(action, &cli.config) {
                Some(config_path) => {
                    match ServiceConfigLoader::load_service_config(
                        ServiceType::Server,
                        Some(&config_path),
                    ) {
                        Ok(config) => config,
                        Err(e) => {
                            error!("Failed to load server configuration: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                None => match ServiceConfigLoader::load_service_config(ServiceType::Server, None) {
                    Ok(config) => config,
                    Err(e) => {
                        error!("Failed to load server configuration: {}", e);
                        std::process::exit(1);
                    }
                },
            }
        }

        // For cluster commands, use cluster-specific config
        Commands::Cluster { action } => {
            match get_config_path_for_cluster_action(action, &cli.config) {
                Some(config_path) => {
                    match ServiceConfigLoader::load_service_config(
                        ServiceType::Cluster,
                        Some(&config_path),
                    ) {
                        Ok(config) => config,
                        Err(e) => {
                            error!("Failed to load cluster configuration: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                None => {
                    match ServiceConfigLoader::load_service_config(ServiceType::Cluster, None) {
                        Ok(config) => config,
                        Err(e) => {
                            error!("Failed to load cluster configuration: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
            }
        }

        // For load balancer commands, use load balancer-specific config
        Commands::Lb { action } => match get_config_path_for_lb_action(action, &cli.config) {
            Some(config_path) => {
                match ServiceConfigLoader::load_service_config(
                    ServiceType::LoadBalancer,
                    Some(&config_path),
                ) {
                    Ok(config) => config,
                    Err(e) => {
                        error!("Failed to load load balancer configuration: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            None => {
                match ServiceConfigLoader::load_service_config(ServiceType::LoadBalancer, None) {
                    Ok(config) => config,
                    Err(e) => {
                        error!("Failed to load load balancer configuration: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        },

        // For other commands, fall back to the original logic
        _ => {
            let config_path = PathBuf::from(&cli.config);
            let config_manager = match ConfigManager::new(config_path) {
                Ok(manager) => manager,
                Err(e) => {
                    error!("Failed to load configuration: {}", e);
                    std::process::exit(1);
                }
            };
            config_manager.config().await
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
        ServerAction::Start { config: _ } => {
            info!("Starting server in background mode");
            server::handle_server_start(config)
                .await
                .context("Failed to start server")
        }
        ServerAction::Stop => {
            info!("Stopping server");
            server::handle_server_stop(config)
                .await
                .context("Failed to stop server")
        }
        ServerAction::Restart { config: _ } => {
            info!("Restarting server");
            server::handle_server_restart(config)
                .await
                .context("Failed to restart server")
        }
        ServerAction::Status => {
            info!("Checking server status");
            server::handle_server_status(config)
                .await
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

/// Handle daemon-related commands (internal use)
async fn handle_daemon_command(action: DaemonAction, config: &voice_cli::Config) -> Result<()> {
    use voice_cli::cli::server;

    match action {
        DaemonAction::Serve => {
            // This is the internal command called by the daemon process
            server::handle_daemon_serve(config)
                .await
                .context("Failed to serve daemon")
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
                http_port.unwrap_or(8080),
                grpc_port.unwrap_or(50051),
                can_process_tasks,
                advertise_ip,
            )
            .await
            .context("Failed to run cluster node")
        }
        ClusterAction::Start {
            config: _,
            node_id,
            http_port,
            grpc_port,
            can_process_tasks,
            save_config,
            advertise_ip,
        } => {
            info!(
                "Starting cluster node: node_id={:?}, http_port={:?}, grpc_port={:?}, can_process_tasks={}, save_config={}, advertise_ip={:?}",
                node_id, http_port, grpc_port, can_process_tasks, save_config, advertise_ip
            );
            cluster::handle_cluster_start(
                config,
                node_id,
                http_port.unwrap_or(8080),
                grpc_port.unwrap_or(50051),
                can_process_tasks,
                save_config,
                advertise_ip,
            )
            .await
            .context("Failed to start cluster node")
        }
        ClusterAction::Stop => {
            info!("Stopping cluster node");
            cluster::handle_cluster_stop(config)
                .await
                .context("Failed to stop cluster node")
        }
        ClusterAction::Restart {
            config: _,
            node_id,
            http_port,
            grpc_port,
            can_process_tasks,
            save_config,
            advertise_ip,
        } => {
            info!(
                "Restarting cluster node: node_id={:?}, http_port={:?}, grpc_port={:?}, can_process_tasks={}, save_config={}, advertise_ip={:?}",
                node_id, http_port, grpc_port, can_process_tasks, save_config, advertise_ip
            );
            cluster::handle_cluster_restart(
                config,
                node_id,
                http_port.unwrap_or(8080),
                grpc_port.unwrap_or(50051),
                can_process_tasks,
                save_config,
                advertise_ip,
            )
            .await
            .context("Failed to restart cluster node")
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
                http_port.unwrap_or(8080),
                grpc_port.unwrap_or(50051),
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
        LoadBalancerAction::Start { config: _, port } => {
            let port = port.unwrap_or(8090);
            info!("Starting load balancer: port={}", port);
            lb::handle_lb_start(config, port)
                .await
                .context("Failed to start load balancer")
        }
        LoadBalancerAction::Stop => {
            info!("Stopping load balancer");
            lb::handle_lb_stop(config)
                .await
                .context("Failed to stop load balancer")
        }
        LoadBalancerAction::Restart { config: _, port } => {
            let port = port.unwrap_or(8090);
            info!("Restarting load balancer: port={}", port);
            lb::handle_lb_restart(config, port)
                .await
                .context("Failed to restart load balancer")
        }
        LoadBalancerAction::Status => {
            info!("Checking load balancer status");
            lb::handle_lb_status(config)
                .await
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

    tracing_subscriber::registry().with(console_layer).init();
}

/// Extract config path from server action
fn get_config_path_for_server_action(
    action: &ServerAction,
    default_config: &str,
) -> Option<PathBuf> {
    match action {
        ServerAction::Run { config }
        | ServerAction::Start { config }
        | ServerAction::Restart { config } => config.clone(),
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

/// Extract config path from cluster action
fn get_config_path_for_cluster_action(
    action: &ClusterAction,
    default_config: &str,
) -> Option<PathBuf> {
    match action {
        ClusterAction::Run { config, .. }
        | ClusterAction::Start { config, .. }
        | ClusterAction::Restart { config, .. } => config.clone(),
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
        LoadBalancerAction::Run { config, .. }
        | LoadBalancerAction::Start { config, .. }
        | LoadBalancerAction::Restart { config, .. } => config.clone(),
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
