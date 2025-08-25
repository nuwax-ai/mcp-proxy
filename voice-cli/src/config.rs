use crate::models::Config;
use crate::VoiceCliError;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn, error};

/// Configuration change notification
#[derive(Debug, Clone)]
pub struct ConfigChangeNotification {
    pub old_config: Config,
    pub new_config: Config,
    pub changed_at: SystemTime,
}

/// Hot-reloadable configuration manager
pub struct ConfigManager {
    config_path: PathBuf,
    config: Arc<RwLock<Config>>,
    last_modified: Arc<RwLock<SystemTime>>,
    change_notifier: broadcast::Sender<ConfigChangeNotification>,
}

impl ConfigManager {
    pub fn new(config_path: PathBuf) -> crate::Result<Self> {
        let config = Config::load_or_create(&config_path)?;
        
        // Validate configuration
        config.validate()?;
        
        // Ensure required directories exist
        Self::ensure_directories(&config)?;
        
        // Get initial file modification time
        let last_modified = std::fs::metadata(&config_path)
            .map(|metadata| metadata.modified().unwrap_or(SystemTime::now()))
            .unwrap_or(SystemTime::now());
        
        // Create change notification channel
        let (change_notifier, _) = broadcast::channel(16);
        
        Ok(Self {
            config_path,
            config: Arc::new(RwLock::new(config)),
            last_modified: Arc::new(RwLock::new(last_modified)),
            change_notifier,
        })
    }

    /// Get a clone of the current configuration
    pub async fn config(&self) -> Config {
        self.config.read().await.clone()
    }
    
    /// Get a read guard to the configuration
    pub fn config_arc(&self) -> Arc<RwLock<Config>> {
        self.config.clone()
    }

    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    /// Manually reload configuration from file
    pub async fn reload(&self) -> crate::Result<()> {
        info!("Manually reloading configuration from {:?}", self.config_path);
        
        let old_config = self.config.read().await.clone();
        let new_config = Config::load_or_create(&self.config_path)?;
        new_config.validate()?;
        Self::ensure_directories(&new_config)?;
        
        // Update configuration
        {
            let mut config_guard = self.config.write().await;
            *config_guard = new_config.clone();
        }
        
        // Update last modified time
        if let Ok(metadata) = std::fs::metadata(&self.config_path) {
            if let Ok(modified) = metadata.modified() {
                let mut last_modified_guard = self.last_modified.write().await;
                *last_modified_guard = modified;
            }
        }
        
        // Notify listeners of configuration change
        let notification = ConfigChangeNotification {
            old_config,
            new_config,
            changed_at: SystemTime::now(),
        };
        
        if let Err(_) = self.change_notifier.send(notification) {
            // No receivers, which is fine
        }
        
        info!("Configuration reloaded successfully");
        Ok(())
    }

    /// Save current configuration to file
    pub async fn save(&self) -> crate::Result<()> {
        let config = self.config.read().await;
        config.save(&self.config_path)?;
        info!("Configuration saved to {:?}", self.config_path);
        Ok(())
    }

    /// Update configuration with a closure and save to file
    pub async fn update_config<F>(&self, updater: F) -> crate::Result<()>
    where
        F: FnOnce(&mut Config),
    {
        let old_config = {
            let config_guard = self.config.read().await;
            config_guard.clone()
        };
        
        let mut new_config = old_config.clone();
        updater(&mut new_config);
        new_config.validate()?;
        
        // Update configuration
        {
            let mut config_guard = self.config.write().await;
            *config_guard = new_config.clone();
        }
        
        // Save to file
        self.save().await?;
        
        // Notify listeners of configuration change
        let notification = ConfigChangeNotification {
            old_config,
            new_config,
            changed_at: SystemTime::now(),
        };
        
        if let Err(_) = self.change_notifier.send(notification) {
            // No receivers, which is fine
        }
        
        Ok(())
    }

    fn ensure_directories(config: &Config) -> crate::Result<()> {
        // Create models directory
        let models_dir = config.models_dir_path();
        if !models_dir.exists() {
            std::fs::create_dir_all(&models_dir)?;
            info!("Created models directory: {:?}", models_dir);
        }

        // Create logs directory
        let logs_dir = config.log_dir_path();
        if !logs_dir.exists() {
            std::fs::create_dir_all(&logs_dir)?;
            info!("Created logs directory: {:?}", logs_dir);
        }

        // Create daemon work directory if needed
        let work_dir = PathBuf::from(&config.daemon.work_dir);
        if !work_dir.exists() {
            std::fs::create_dir_all(&work_dir)?;
            info!("Created daemon work directory: {:?}", work_dir);
        }

        Ok(())
    }

    /// Generate a default configuration file with comments
    pub fn generate_default_config_with_comments(config_path: &PathBuf) -> crate::Result<()> {
        // Use embedded template from config.yml.template
        let config_yaml = include_str!("../templates/config.yml.template");

        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        std::fs::write(config_path, config_yaml)?;
        info!("Generated default configuration file at {:?}", config_path);
        
        Ok(())
    }

    /// Validate the current environment and configuration
    pub async fn validate_environment(&self) -> crate::Result<()> {
        let config = self.config.read().await;

        // Check if models directory is writable
        let models_dir = config.models_dir_path();
        if models_dir.exists() {
            let test_file = models_dir.join(".write_test");
            match std::fs::write(&test_file, "test") {
                Ok(_) => {
                    let _ = std::fs::remove_file(test_file);
                }
                Err(_) => {
                    return Err(VoiceCliError::Config(
                        format!("Models directory is not writable: {:?}", models_dir)
                    ));
                }
            }
        }

        // Check if logs directory is writable
        let logs_dir = config.log_dir_path();
        if logs_dir.exists() {
            let test_file = logs_dir.join(".write_test");
            match std::fs::write(&test_file, "test") {
                Ok(_) => {
                    let _ = std::fs::remove_file(test_file);
                }
                Err(_) => {
                    return Err(VoiceCliError::Config(
                        format!("Logs directory is not writable: {:?}", logs_dir)
                    ));
                }
            }
        }

        // Check if port is available (basic check)
        if let Err(_) = std::net::TcpListener::bind(format!("{}:{}", config.server.host, config.server.port)) {
            warn!("Port {}:{} may not be available", config.server.host, config.server.port);
        }

        info!("Environment validation passed");
        Ok(())
    }

    /// Get configuration summary for logging
    pub async fn get_summary(&self) -> String {
        let config = self.config.read().await;
        format!(
            "Config Summary:\n  Server: {}:{}\n  Max file size: {} MB\n  Models dir: {}\n  Logs dir: {}\n  Default model: {}",
            config.server.host,
            config.server.port,
            config.server.max_file_size / 1024 / 1024,
            config.whisper.models_dir,
            config.logging.log_dir,
            config.whisper.default_model
        )
    }
    
    /// Subscribe to configuration change notifications
    pub fn subscribe_to_changes(&self) -> broadcast::Receiver<ConfigChangeNotification> {
        self.change_notifier.subscribe()
    }
    
    /// Check if configuration file has been modified and reload if necessary
    pub async fn check_and_reload_if_changed(&self) -> crate::Result<bool> {
        // Check if file exists and get its modification time
        let current_modified = match std::fs::metadata(&self.config_path) {
            Ok(metadata) => metadata.modified().unwrap_or(SystemTime::now()),
            Err(_) => {
                // File doesn't exist, nothing to reload
                return Ok(false);
            }
        };
        
        let last_modified = *self.last_modified.read().await;
        
        // Check if file has been modified since last check
        if current_modified > last_modified {
            info!("Configuration file has been modified, reloading...");
            self.reload().await?;
            return Ok(true);
        }
        
        Ok(false)
    }
    
    /// Start automatic configuration file watching and hot reload
    /// Returns a task handle that can be used to stop the watcher
    pub fn start_hot_reload(&self, check_interval_secs: u64) -> tokio::task::JoinHandle<()> {
        let config_path = self.config_path.clone();
        let last_modified = self.last_modified.clone();
        let config = self.config.clone();
        let change_notifier = self.change_notifier.clone();
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(check_interval_secs));
            
            loop {
                interval.tick().await;
                
                // Check if file has been modified
                let current_modified = match std::fs::metadata(&config_path) {
                    Ok(metadata) => metadata.modified().unwrap_or(SystemTime::now()),
                    Err(e) => {
                        error!("Failed to check config file metadata: {}", e);
                        continue;
                    }
                };
                
                let last_modified_time = *last_modified.read().await;
                
                if current_modified > last_modified_time {
                    info!("Configuration file changed, hot reloading...");
                    
                    // Attempt to reload configuration
                    match Self::hot_reload_config(&config_path, &config, &last_modified, &change_notifier).await {
                        Ok(_) => {
                            info!("Configuration hot reloaded successfully");
                        }
                        Err(e) => {
                            error!("Failed to hot reload configuration: {}", e);
                        }
                    }
                }
            }
        })
    }
    
    /// Internal method to perform hot reload
    async fn hot_reload_config(
        config_path: &PathBuf,
        config: &Arc<RwLock<Config>>,
        last_modified: &Arc<RwLock<SystemTime>>,
        change_notifier: &broadcast::Sender<ConfigChangeNotification>,
    ) -> crate::Result<()> {
        let old_config = config.read().await.clone();
        
        // Load new configuration
        let new_config = Config::load_or_create(config_path)?;
        new_config.validate()?;
        Self::ensure_directories(&new_config)?;
        
        // Update configuration
        {
            let mut config_guard = config.write().await;
            *config_guard = new_config.clone();
        }
        
        // Update last modified time
        if let Ok(metadata) = std::fs::metadata(config_path) {
            if let Ok(modified) = metadata.modified() {
                let mut last_modified_guard = last_modified.write().await;
                *last_modified_guard = modified;
            }
        }
        
        // Notify listeners of configuration change
        let notification = ConfigChangeNotification {
            old_config,
            new_config,
            changed_at: SystemTime::now(),
        };
        
        if let Err(_) = change_notifier.send(notification) {
            // No receivers, which is fine
        }
        
        Ok(())
    }
}