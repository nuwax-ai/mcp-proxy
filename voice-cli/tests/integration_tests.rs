use tempfile::TempDir;
use voice_cli::{ConfigManager, Config};

#[tokio::test]
async fn test_config_manager_creation() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("test_config.yml");
    
    let config_manager = ConfigManager::new(config_path);
    assert!(config_manager.is_ok());
    
    let manager = config_manager.unwrap();
    let config = manager.config().await;
    
    // Validate default configuration
    assert_eq!(config.server.port, 8080);
    assert_eq!(config.whisper.default_model, "base");
    assert!(config.whisper.auto_download);
}

#[tokio::test]
async fn test_model_service_creation() {
    let config = Config::default();
    let model_service = voice_cli::ModelService::new(config);
    
    // Test basic functionality
    let downloaded_models = model_service.list_downloaded_models().await;
    assert!(downloaded_models.is_ok());
}

#[test]
fn test_audio_format_detection() {
    use voice_cli::models::AudioFormat;
    
    assert!(AudioFormat::from_filename("test.mp3").is_supported());
    assert!(AudioFormat::from_filename("test.wav").is_supported());
    assert!(!AudioFormat::from_filename("test.xyz").is_supported());
    
    assert!(AudioFormat::from_filename("test.wav").needs_conversion() == false);
    assert!(AudioFormat::from_filename("test.mp3").needs_conversion() == true);
}

#[test]
fn test_cli_parsing() {
    use clap::Parser;
    use voice_cli::cli::Cli;
    
    // Test server run command
    let args = vec!["voice-cli", "server", "run"];
    let cli = Cli::try_parse_from(args);
    assert!(cli.is_ok());
    
    // Test model download command
    let args = vec!["voice-cli", "model", "download", "base"];
    let cli = Cli::try_parse_from(args);
    assert!(cli.is_ok());
    
    // Test with config option
    let args = vec!["voice-cli", "--config", "custom.yml", "server", "status"];
    let cli = Cli::try_parse_from(args);
    assert!(cli.is_ok());
}