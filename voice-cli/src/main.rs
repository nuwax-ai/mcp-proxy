use clap::Parser;
use std::path::PathBuf;
use tracing::{info, error, warn};
use voice_cli::{
    cli::{Cli, Commands, ServerAction, ModelAction, DaemonAction},
    config::ConfigManager,
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
    
    let config = config_manager.config();
    
    // Log configuration summary
    if cli.verbose {
        info!("{}", config_manager.get_summary());
    }
    
    // Validate environment
    if let Err(e) = config_manager.validate_environment() {
        warn!("Environment validation warning: {}", e);
    }
    
    // Route to appropriate handler
    let result = match cli.command {
        Commands::Server { action } => handle_server_command(action, config).await,
        Commands::Model { action } => handle_model_command(action, config).await,
        Commands::Daemon { action } => handle_daemon_command(action, config).await,
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
async fn handle_server_command(action: ServerAction, config: &voice_cli::Config) -> voice_cli::Result<()> {
    use voice_cli::cli::server;
    
    match action {
        ServerAction::Run => {
            info!("Running server in foreground mode");
            server::handle_server_run(config).await
        }
        ServerAction::Start => {
            info!("Starting server in background mode");
            server::handle_server_start(config).await
        }
        ServerAction::Stop => {
            info!("Stopping server");
            server::handle_server_stop(config).await
        }
        ServerAction::Restart => {
            info!("Restarting server");
            server::handle_server_restart(config).await
        }
        ServerAction::Status => {
            info!("Checking server status");
            server::handle_server_status(config).await
        }
    }
}

/// Handle model-related commands
async fn handle_model_command(action: ModelAction, config: &voice_cli::Config) -> voice_cli::Result<()> {
    use voice_cli::cli::model;
    
    match action {
        ModelAction::Download { model_name } => {
            info!("Downloading model: {}", model_name);
            model::handle_model_download(config, &model_name).await
        }
        ModelAction::List => {
            info!("Listing models");
            model::handle_model_list(config).await
        }
        ModelAction::Validate => {
            info!("Validating models");
            model::handle_model_validate(config).await
        }
        ModelAction::Remove { model_name } => {
            info!("Removing model: {}", model_name);
            model::handle_model_remove(config, &model_name).await
        }
    }
}

/// Handle daemon-related commands (internal use)
async fn handle_daemon_command(action: DaemonAction, config: &voice_cli::Config) -> voice_cli::Result<()> {
    use voice_cli::cli::server;
    
    match action {
        DaemonAction::Serve => {
            // This is the internal command called by the daemon process
            server::handle_daemon_serve(config).await
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