use crate::models::Config;
use crate::VoiceCliError;
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::time::timeout;
use tracing::{info, warn};

pub mod process_manager;
pub mod health_checker;

pub use process_manager::ProcessManager;
pub use health_checker::HealthChecker;

/// Daemon service for managing the voice-cli HTTP server
pub struct DaemonService {
    config: Config,
    process_manager: ProcessManager,
    health_checker: HealthChecker,
}

impl DaemonService {
    pub fn new(config: Config) -> Self {
        let process_manager = ProcessManager::new(&config);
        let health_checker = HealthChecker::new(&config);
        
        Self {
            config,
            process_manager,
            health_checker,
        }
    }

    /// Start the HTTP server as a daemon process
    pub async fn start_daemon(&self) -> crate::Result<()> {
        info!("Starting voice-cli daemon...");

        // Check if already running
        if self.is_running()? {
            return Err(VoiceCliError::Daemon("Daemon is already running".to_string()));
        }

        // Start the daemon process directly (not through CLI)
        let pid = self.spawn_daemon_process()?;
        info!("Daemon started with PID: {}", pid);

        // Wait for the service to become healthy
        self.wait_for_healthy_startup().await?;
        
        info!("Daemon started successfully and is responding to health checks");
        Ok(())
    }

    /// Stop the daemon process
    pub async fn stop_daemon(&self) -> crate::Result<()> {
        info!("Stopping voice-cli daemon...");

        if let Some(pid) = self.process_manager.read_pid()? {
            self.process_manager.terminate_process(pid)?;
            
            // Wait for graceful shutdown
            self.wait_for_shutdown(pid).await?;
            
            self.process_manager.cleanup_pid_file()?;
            info!("Daemon stopped successfully");
        } else {
            info!("No running daemon found");
        }

        Ok(())
    }

    /// Restart the daemon process
    pub async fn restart_daemon(&self) -> crate::Result<()> {
        info!("Restarting voice-cli daemon...");

        if self.is_running()? {
            self.stop_daemon().await?;
        }

        self.start_daemon().await?;
        Ok(())
    }

    /// Get daemon status
    pub async fn get_status(&self) -> crate::Result<DaemonStatus> {
        let pid = self.process_manager.read_pid()?;
        
        let status = match pid {
            Some(pid) if self.process_manager.is_process_running(pid) => {
                match self.health_checker.check_health().await {
                    Ok(health) => DaemonStatus::Running { pid, health: Some(health) },
                    Err(_) => DaemonStatus::Running { pid, health: None },
                }
            }
            Some(_pid) => {
                // PID file exists but process is not running
                self.process_manager.cleanup_pid_file()?;
                DaemonStatus::Stopped
            }
            None => DaemonStatus::Stopped,
        };

        Ok(status)
    }

    /// Check if daemon is running
    pub fn is_running(&self) -> crate::Result<bool> {
        let pid = self.process_manager.read_pid()?;
        Ok(pid.map_or(false, |pid| self.process_manager.is_process_running(pid)))
    }

    /// Spawn the actual daemon process that runs the HTTP server
    fn spawn_daemon_process(&self) -> crate::Result<u32> {
        let current_exe = std::env::current_exe()?;
        
        // Create the daemon process that directly runs the server
        // This avoids the recursive CLI call issue
        let mut cmd = Command::new(current_exe);
        cmd.args(&["--config", &self.get_config_path()?, "daemon", "serve"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("RUST_LOG", &self.config.logging.level);

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Create new process group for proper daemonization
            cmd.process_group(0);
        }

        let child = cmd.spawn()
            .map_err(|e| VoiceCliError::Daemon(format!("Failed to spawn daemon: {}", e)))?;
        
        let pid = child.id();
        self.process_manager.save_pid(pid)?;
        
        Ok(pid)
    }

    /// Wait for the daemon to become healthy after startup
    async fn wait_for_healthy_startup(&self) -> crate::Result<()> {
        let max_wait = Duration::from_secs(30);
        let check_interval = Duration::from_millis(500);
        
        let result: Result<Result<(), crate::VoiceCliError>, tokio::time::error::Elapsed> = timeout(max_wait, async {
            loop {
                match self.health_checker.check_health().await {
                    Ok(_) => return Ok(()),
                    Err(_) => {
                        tokio::time::sleep(check_interval).await;
                    }
                }
            }
        }).await;

        match result {
            Ok(_) => Ok::<(), crate::VoiceCliError>(()),
            Err(_) => {
                // Cleanup on failure
                if let Some(pid) = self.process_manager.read_pid()? {
                    let _ = self.process_manager.terminate_process(pid);
                }
                self.process_manager.cleanup_pid_file()?;
                Err(VoiceCliError::Daemon(
                    "Daemon failed to become healthy within 30 seconds".to_string()
                ))
            }
        }
    }

    /// Wait for graceful shutdown
    async fn wait_for_shutdown(&self, pid: u32) -> crate::Result<()> {
        let max_wait = Duration::from_secs(10);
        let check_interval = Duration::from_millis(100);
        
        let result = timeout(max_wait, async {
            while self.process_manager.is_process_running(pid) {
                tokio::time::sleep(check_interval).await;
            }
        }).await;

        if result.is_err() {
            warn!("Process {} did not shutdown gracefully, force killing", pid);
            self.process_manager.force_kill_process(pid)?;
        }

        Ok(())
    }

    fn get_config_path(&self) -> crate::Result<String> {
        let config_path = std::env::current_dir()?.join("config.yml");
        config_path.to_str()
            .ok_or_else(|| VoiceCliError::Config("Invalid config path".to_string()))
            .map(|s| s.to_string())
    }
}

#[derive(Debug)]
pub enum DaemonStatus {
    Running { 
        pid: u32, 
        health: Option<crate::models::HealthResponse> 
    },
    Stopped,
}

impl DaemonStatus {
    pub fn is_running(&self) -> bool {
        matches!(self, DaemonStatus::Running { .. })
    }
}