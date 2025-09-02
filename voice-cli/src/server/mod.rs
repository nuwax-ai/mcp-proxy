pub mod handlers;
pub mod middleware;
pub mod routes;
pub mod http_tracing;
pub mod middleware_config;
pub mod app_state;

use crate::models::Config;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};





async fn shutdown_signal_with_broadcast(shutdown_tx: broadcast::Sender<()>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("Signal received, starting graceful shutdown");
    let _ = shutdown_tx.send(());
}

/// Initialize server configuration
pub async fn handle_server_init(config_path: Option<PathBuf>, force: bool) -> crate::Result<()> {
    let output_path = config_path.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("server-config.yml")
    });

    // 检查文件是否已存在
    if output_path.exists() && !force {
        println!("❌ Configuration file already exists: {:?}", output_path);
        println!("💡 Use --force to overwrite, or specify a different path with --config");
        return Ok(());
    }

    // 生成配置文件
    crate::config::ConfigTemplateGenerator::generate_config_file(crate::config::ServiceType::Server, &output_path)?;

    println!("✅ Server configuration initialized: {:?}", output_path);
    
    // 初始化 Python 虚拟环境和 TTS 依赖
    if let Err(e) = init_python_tts_environment().await {
        warn!("Failed to initialize Python TTS environment: {}", e);
        println!("⚠️  Python TTS environment initialization failed: {}", e);
        println!("💡 You can manually initialize it later with: uv venv && uv add index-tts torch torchaudio numpy soundfile");
    } else {
        println!("✅ Python TTS environment initialized successfully");
    }

    println!("📝 Edit the configuration file and run:");
    println!("   voice-cli server run --config {:?}", output_path);

    Ok(())
}

/// Initialize Python virtual environment and TTS dependencies using uv
pub async fn init_python_tts_environment() -> crate::Result<()> {
    use std::process::Command;
    use std::time::Duration;
    use tokio::time::sleep;

    println!("🐍 Initializing Python TTS environment with uv...");

    // Check if uv is available
    let uv_check = Command::new("uv").arg("--version").output();
    match uv_check {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            println!("✅ Found uv: {}", version.trim());
        }
        Ok(_) => {
            return Err(crate::VoiceCliError::Config(
                "uv command found but failed to get version".to_string(),
            ));
        }
        Err(e) => {
            return Err(crate::VoiceCliError::Config(format!(
                "uv not found. Please install uv first: https://docs.astral.sh/uv/getting-started/installation/ - Error: {}",
                e
            )));
        }
    }

    // Create virtual environment if it doesn't exist
    let venv_path = PathBuf::from(".venv");
    if !venv_path.exists() {
        println!("📦 Creating Python virtual environment...");
        let output = Command::new("uv")
            .arg("venv")
            .output()
            .map_err(|e| crate::VoiceCliError::Config(format!("Failed to create virtual environment: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::VoiceCliError::Config(format!(
                "Failed to create virtual environment: {}",
                stderr
            )));
        }
        println!("✅ Virtual environment created successfully");
    } else {
        println!("✅ Virtual environment already exists");
    }

    // Install TTS dependencies
    println!("📚 Installing TTS dependencies...");
    
    // Install index-tts from GitHub
    println!("   Installing index-tts...");
    let output = Command::new("uv")
        .arg("add")
        .arg("index-tts")
        .output()
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to install index-tts: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("⚠️  index-tts installation warning: {}", stderr);
        println!("   stdout: {}", stdout);
        // Continue despite warnings - the package might still work
    } else {
        println!("✅ index-tts installed successfully");
    }

    // Install additional dependencies
    let dependencies = ["torch", "torchaudio", "numpy", "soundfile"];
    for dep in &dependencies {
        println!("   Installing {}...", dep);
        let output = Command::new("uv")
            .arg("add")
            .arg(dep)
            .output()
            .map_err(|e| crate::VoiceCliError::Config(format!("Failed to install {}: {}", dep, e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("⚠️  {} installation warning: {}", dep, stderr);
        } else {
            println!("✅ {} installed successfully", dep);
        }
    }

    // Test the installation
    println!("🧪 Testing TTS installation...");
    let test_script = r#"
import sys
try:
    import index_tts
    print("index-tts imported successfully")
    print(f"index-tts version: {getattr(index_tts, '__version__', 'unknown')}")
except ImportError as e:
    print(f"Failed to import index-tts: {e}")
    sys.exit(1)
except Exception as e:
    print(f"Error testing index-tts: {e}")
    sys.exit(1)
"#;

    let output = Command::new("uv")
        .arg("run")
        .arg("python")
        .arg("-c")
        .arg(test_script)
        .output()
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to test TTS installation: {}", e)))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("✅ TTS installation test passed:");
        for line in stdout.lines() {
            println!("   {}", line);
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("⚠️  TTS installation test failed:");
        println!("   stderr: {}", stderr);
        println!("   stdout: {}", stdout);
        return Err(crate::VoiceCliError::Config(format!(
            "TTS installation test failed"
        )));
    }

    println!("✅ Python TTS environment setup complete!");
    Ok(())
}

/// Run server in foreground mode (direct HTTP server)
pub async fn handle_server_run(config: &Config) -> crate::Result<()> {
    info!("Starting voice-cli server in foreground mode...");

    // Initialize logging - keep the guard alive for the duration of the process
    info!("About to initialize logging...");
    crate::utils::init_logging(config)?;
    info!("Logging initialized successfully");

    let config_arc = Arc::new(config.clone());
    let app_state = handlers::AppState::new(config_arc.clone()).await?;
    let mut app = routes::create_routes_with_state(app_state.clone()).await?;
    
    // Clone app_state for use in monitor
    let app_state_for_monitor = app_state.clone();
    
    // Create shutdown channel for monitor task
    let (shutdown_tx, _) = broadcast::channel(1);
    let mut shutdown_rx = shutdown_tx.subscribe();
    
    // 添加 storage 作为 Extension
    app = app.layer(axum::Extension(app_state.apalis_storage.clone()));

    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    info!("Server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to bind to address: {}", e)))?;
    
    info!("TCP listener created successfully: {:?}", listener.local_addr());
    info!("Starting axum server...");

    let http = async {
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal_with_broadcast(shutdown_tx))
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
        
        info!("Axum server completed, performing graceful shutdown...");
        
        // Perform graceful shutdown of application state
        app_state.shutdown().await;
        
        // Perform global cleanup operations
        crate::utils::perform_shutdown_cleanup().await;
        
        info!("Graceful shutdown completed with result: {:?}", result);
        result
    };

    let monitor = async {
        // Wait for shutdown signal
        let _ = shutdown_rx.recv().await;
        info!("Monitor task received shutdown signal, stopping Apalis manager...");
        
        // Gracefully shutdown the Apalis manager
        if let Err(e) = app_state_for_monitor.lock_free_apalis_manager.shutdown().await {
            warn!("Failed to shutdown Apalis manager gracefully: {}", e);
        }
        
        Ok::<(), std::io::Error>(())
    };

    let _res = tokio::join!(http, monitor);

    Ok(())
}
