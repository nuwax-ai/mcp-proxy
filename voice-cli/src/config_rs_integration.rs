use crate::VoiceCliError;
use crate::models::Config;
use config::{Config as ConfigRs, Environment, File};
use serde::Deserialize;
use std::path::PathBuf;

// Instead of implementing TryFrom, we'll create a default config file and load it
fn create_default_config_source() -> Result<ConfigRs, VoiceCliError> {
    // Create a temporary config with defaults
    let default_config = Config::default();

    // Serialize to YAML and then parse back as config source
    let yaml_content = serde_yaml::to_string(&default_config)?;

    // Create config from YAML content
    let config_rs = ConfigRs::builder()
        .add_source(File::from_str(&yaml_content, config::FileFormat::Yaml))
        .build()?;

    Ok(config_rs)
}

/// Configuration settings that can be overridden via CLI arguments
#[derive(Debug, Deserialize, Clone)]
pub struct CliOverrides {
    /// Server host override
    pub host: Option<String>,
    /// Server port override
    pub port: Option<u16>,
    /// Log level override
    pub log_level: Option<String>,
    /// Models directory override
    pub models_dir: Option<String>,
    /// Default model override
    pub default_model: Option<String>,
    /// Transcription workers override
    pub transcription_workers: Option<usize>,
}

impl Default for CliOverrides {
    fn default() -> Self {
        Self {
            host: None,
            port: None,
            log_level: None,
            models_dir: None,
            default_model: None,
            transcription_workers: None,
        }
    }
}

/// Configuration loader using config-rs with proper hierarchy
pub struct ConfigRsLoader;

impl ConfigRsLoader {
    /// Load configuration with proper hierarchy: CLI args > env vars > config files
    pub fn load(
        config_path: Option<&PathBuf>,
        cli_overrides: &CliOverrides,
        service_type: Option<crate::config::ServiceType>,
    ) -> Result<Config, VoiceCliError> {
        let mut config_rs = ConfigRs::builder();

        // 1. Load default configuration (built-in defaults)
        let default_config_source = create_default_config_source()?;
        config_rs = config_rs.add_source(default_config_source);

        // 2. Load configuration from file if specified or from default location
        if let Some(path) = config_path {
            if path.exists() {
                config_rs = config_rs.add_source(File::from(path.clone()));
            }
        } else if let Some(service_type) = service_type {
            // Try to load service-specific default config
            let default_config_path =
                std::env::current_dir()?.join(service_type.default_config_filename());
            if default_config_path.exists() {
                config_rs = config_rs.add_source(File::from(default_config_path));
            }
        }

        // 3. Load environment variables (with proper prefix)
        config_rs = config_rs.add_source(
            Environment::with_prefix("VOICE_CLI")
                .prefix_separator("_")
                .separator("__")
                .try_parsing(true)
                .ignore_empty(true),
        );

        // 4. Build the config and debug what's being loaded
        let built_config = config_rs.build()?;

        // 5. Deserialize the built config
        let mut config: Config = built_config.try_deserialize()?;

        // 6. Apply CLI overrides (highest priority)
        Self::apply_cli_overrides(&mut config, cli_overrides);

        // 9. Apply service-specific settings
        if let Some(service_type) = service_type {
            Self::apply_service_specific_settings(&mut config, service_type)?;
        }

        // 6. Validate configuration
        config.validate()?;

        Ok(config)
    }

    /// Apply CLI argument overrides to configuration
    fn apply_cli_overrides(config: &mut Config, cli_overrides: &CliOverrides) {
        if let Some(host) = &cli_overrides.host {
            config.server.host = host.clone();
        }

        if let Some(port) = cli_overrides.port {
            config.server.port = port;
        }

        if let Some(log_level) = &cli_overrides.log_level {
            config.logging.level = log_level.clone();
        }

        if let Some(models_dir) = &cli_overrides.models_dir {
            config.whisper.models_dir = models_dir.clone();
        }

        if let Some(default_model) = &cli_overrides.default_model {
            config.whisper.default_model = default_model.clone();
        }

        if let Some(workers) = cli_overrides.transcription_workers {
            config.whisper.workers.transcription_workers = workers;
        }
    }

    /// Apply service-specific settings based on service type
    fn apply_service_specific_settings(
        config: &mut Config,
        service_type: crate::config::ServiceType,
    ) -> Result<(), VoiceCliError> {
        match service_type {
            crate::config::ServiceType::Server => {
                config.daemon.pid_file = "./voice-cli-server.pid".to_string();
            }
        }
        Ok(())
    }

    /// Manually merge environment variable overrides (config-rs adds underscore prefix)
    fn merge_environment_overrides(_config: &mut Config, _built_config: &ConfigRs) {
        use config::ValueKind;

        // Check if there are any underscore-prefixed values from environment variables
        if let ValueKind::Table(cache) = &_built_config.cache.kind {
            for (key, value) in cache {
                if key.starts_with('_') {
                    // This is an environment variable override
                    let clean_key = &key[1..]; // Remove the underscore prefix

                    // Handle specific environment variable overrides
                    // Add environment variable overrides here as needed
                    let _ = clean_key; // Avoid unused variable warning
                    let _ = value; // Avoid unused variable warning
                }
            }
        }
    }

    /// Generate CLI overrides from command line arguments
    pub fn generate_cli_overrides_from_args(
        args: &crate::cli::Cli,
    ) -> Result<CliOverrides, VoiceCliError> {
        let overrides = CliOverrides::default();

        match &args.command {
            crate::cli::Commands::Server { action } => {
                if let crate::cli::ServerAction::Run { .. } = action {
                    // Server run command can have port overrides
                    // These will be extracted from the action in the main handler
                }
            }
            _ => {}
        }

        Ok(overrides)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_config_loading_hierarchy() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("test_config.yml");

        // Create a test config file
        let test_config = Config::default();
        test_config.save(&config_path).unwrap();

        let cli_overrides = CliOverrides::default();
        let result = ConfigRsLoader::load(Some(&config_path), &cli_overrides, None);

        assert!(result.is_ok());
    }

    #[test]
    fn test_cli_overrides_application() {
        let mut config = Config::default();
        let cli_overrides = CliOverrides {
            port: Some(9090),
            log_level: Some("debug".to_string()),
            ..Default::default()
        };

        ConfigRsLoader::apply_cli_overrides(&mut config, &cli_overrides);

        assert_eq!(config.server.port, 9090);
        assert_eq!(config.logging.level, "debug");
    }
}
