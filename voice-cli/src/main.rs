use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use tracing::{error, info};
use voice_cli::{
    cli::{
        Cli, Commands, ModelAction, ServerAction,
    },
    config::ServiceType,
    config_rs_integration::ConfigRsLoader,
};

#[tokio::main]
async fn main() {
    // Parse command line arguments
    let cli = Cli::parse();

    // Don't initialize logging here - let each service initialize its own logging
    // based on configuration files

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



/// Extract config path from server action
fn get_config_path_for_server_action(
    action: &ServerAction,
    _default_config: &str,
) -> Option<PathBuf> {
    match action {
        ServerAction::Run { config } => config.clone(),
        _ => None,
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
