use crate::config::{ConfigTemplateGenerator, ServiceConfigLoader, ServiceType};
use crate::load_balancer::VoiceCliLoadBalancer;
use crate::models::Config;
use crate::models::MetadataStore;
use crate::{Result, VoiceCliError};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tracing::{error, info, warn};

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
        let mut config = ServiceConfigLoader::load_service_config(
            ServiceType::LoadBalancer,
            Some(&output_path),
        )?;
        config.load_balancer.port = port;
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

/// Handle load balancer start command (background mode)
pub async fn handle_lb_start(config: &Config, port: u16) -> Result<()> {
    info!("Starting load balancer in background mode");

    // Check if already running
    if is_lb_running(port).await? {
        return Err(VoiceCliError::Config(
            "Load balancer is already running".to_string(),
        ));
    }

    // Get the current executable path
    let current_exe = std::env::current_exe().map_err(|e| {
        VoiceCliError::Config(format!("Failed to get current executable path: {}", e))
    })?;

    // Build the daemon command
    let mut cmd = Command::new(&current_exe);
    cmd.args(&[
        "lb",
        "run",
        "--port",
        &port.to_string(),
        "--config",
        &get_config_path_from_args(),
    ]);

    // Start as background process
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null());

    // Spawn the background process
    let child = cmd
        .spawn()
        .map_err(|e| VoiceCliError::Config(format!("Failed to start load balancer: {}", e)))?;

    // Write PID file for management
    let pid_file = &config.load_balancer.pid_file;
    std::fs::write(pid_file, child.id().to_string())
        .map_err(|e| VoiceCliError::Config(format!("Failed to write PID file: {}", e)))?;

    info!("Load balancer started with PID: {}", child.id());
    info!("PID file: {}", pid_file);

    // Wait a moment and check if it's actually running
    tokio::time::sleep(Duration::from_secs(2)).await;

    if is_lb_running(port).await? {
        info!("Load balancer is running successfully on port {}", port);
        Ok(())
    } else {
        Err(VoiceCliError::Config(
            "Load balancer failed to start".to_string(),
        ))
    }
}

/// Handle load balancer stop command
pub async fn handle_lb_stop(config: &Config) -> Result<()> {
    info!("Stopping load balancer");

    let pid_file = &config.load_balancer.pid_file;

    // Read PID from file
    let pid_str = std::fs::read_to_string(pid_file).map_err(|e| {
        VoiceCliError::Config(format!(
            "Failed to read PID file: {}. Load balancer may not be running",
            e
        ))
    })?;

    let pid: u32 = pid_str
        .trim()
        .parse()
        .map_err(|e| VoiceCliError::Config(format!("Invalid PID in file: {}", e)))?;

    // Attempt to stop the process
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;

        let pid = Pid::from_raw(pid as i32);

        // First try SIGTERM for graceful shutdown
        match signal::kill(pid, Signal::SIGTERM) {
            Ok(_) => {
                info!("Sent SIGTERM to process {}", pid);

                // Wait for graceful shutdown
                for _ in 0..10 {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    if signal::kill(pid, None).is_err() {
                        // Process is gone
                        break;
                    }
                }

                // If still running, force kill
                if signal::kill(pid, None).is_ok() {
                    warn!("Process {} still running, sending SIGKILL", pid);
                    let _ = signal::kill(pid, Signal::SIGKILL);
                }
            }
            Err(e) => {
                return Err(VoiceCliError::Config(format!(
                    "Failed to stop process {}: {}",
                    pid, e
                )));
            }
        }
    }

    #[cfg(windows)]
    {
        // Windows implementation
        let output = Command::new("taskkill")
            .args(&["/PID", &pid.to_string(), "/F"])
            .output()
            .map_err(|e| VoiceCliError::Config(format!("Failed to kill process: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VoiceCliError::Config(format!(
                "Failed to stop process {}: {}",
                pid, stderr
            )));
        }
    }

    // Remove PID file
    let _ = std::fs::remove_file(pid_file);

    info!("Load balancer stopped successfully");
    Ok(())
}

/// Handle load balancer restart command
pub async fn handle_lb_restart(config: &Config, port: u16) -> Result<()> {
    info!("Restarting load balancer");

    // Try to stop if running (ignore errors if not running)
    let _ = handle_lb_stop(config).await;

    // Wait a moment for cleanup
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Start again
    handle_lb_start(config, port).await
}

/// Handle load balancer status command
pub async fn handle_lb_status(config: &Config) -> Result<()> {
    info!("Checking load balancer status");

    let pid_file = &config.load_balancer.pid_file;

    // Check if PID file exists
    if !std::path::Path::new(pid_file).exists() {
        println!("Load balancer is not running (no PID file found)");
        return Ok(());
    }

    // Read PID from file
    let pid_str = std::fs::read_to_string(pid_file)
        .map_err(|e| VoiceCliError::Config(format!("Failed to read PID file: {}", e)))?;

    let pid: u32 = pid_str
        .trim()
        .parse()
        .map_err(|e| VoiceCliError::Config(format!("Invalid PID in file: {}", e)))?;

    // Check if process is actually running
    let is_running = check_process_running(pid);

    if is_running {
        println!("Load balancer is running (PID: {})", pid);

        // Check if the port is responding (simplified check)
        let port = config.load_balancer.port;

        if is_port_responding(port).await {
            println!("Load balancer is responding on port {}", port);
        } else {
            println!(
                "Load balancer process is running but not responding on port {}",
                port
            );
        }
    } else {
        println!("Load balancer is not running (process {} not found)", pid);

        // Clean up stale PID file
        let _ = std::fs::remove_file(pid_file);
    }

    Ok(())
}

/// Check if load balancer is running on the specified port
async fn is_lb_running(port: u16) -> Result<bool> {
    Ok(is_port_responding(port).await)
}

/// Check if a process with the given PID is running
fn check_process_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::sys::signal;
        use nix::unistd::Pid;

        let pid = Pid::from_raw(pid as i32);
        signal::kill(pid, None).is_ok()
    }

    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .args(&["/FI", &format!("PID eq {}", pid)])
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.contains(&pid.to_string())
            }
            Err(_) => false,
        }
    }
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

    #[test]
    fn test_check_process_running() {
        // Test with current process (should be running)
        let current_pid = std::process::id();
        assert!(check_process_running(current_pid));

        // Test with invalid PID (should not be running)
        assert!(!check_process_running(99999));
    }

    #[tokio::test]
    async fn test_is_port_responding() {
        // Test with a port that's definitely not in use
        assert!(!is_port_responding(65534).await);
    }
}
