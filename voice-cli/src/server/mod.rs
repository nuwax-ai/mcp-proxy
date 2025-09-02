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
    
    // 检查并创建 tts_service.py 文件
    if let Err(e) = create_tts_service_file().await {
        warn!("Failed to create tts_service.py: {}", e);
        println!("⚠️  TTS service file creation failed: {}", e);
    } else {
        println!("✅ TTS service file created successfully");
    }
    
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

/// Create tts_service.py file if it doesn't exist
pub async fn create_tts_service_file() -> crate::Result<()> {
    use std::fs;
    
    let current_dir = std::env::current_dir()
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to get current directory: {}", e)))?;
    
    let tts_service_path = current_dir.join("tts_service.py");
    
    // 如果文件已存在，跳过创建
    if tts_service_path.exists() {
        info!("tts_service.py already exists, skipping creation");
        return Ok(());
    }
    
    // 从模板文件加载 tts_service.py 内容
    let tts_service_content = include_str!("../../templates/tts_service.py.template");
    
    // 写入文件
    fs::write(&tts_service_path, tts_service_content)
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to create tts_service.py: {}", e)))?;
    
    // 设置文件权限为可执行
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tts_service_path.metadata()
            .map_err(|e| crate::VoiceCliError::Config(format!("Failed to get file permissions: {}", e)))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&tts_service_path, perms)
            .map_err(|e| crate::VoiceCliError::Config(format!("Failed to set file permissions: {}", e)))?;
    }
    
    info!("tts_service.py created successfully: {:?}", tts_service_path);
    Ok(())
}

/// Initialize Python virtual environment and TTS dependencies using uv
pub async fn init_python_tts_environment() -> crate::Result<()> {
    use std::process::Command;

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

    // Get the path to the pyproject.toml file (in the voice-cli crate directory)
    let project_dir = std::env::current_dir()
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to get current directory: {}", e)))?;
    
    // Check if pyproject.toml exists in current directory
    let pyproject_path = project_dir.join("pyproject.toml");
    let work_dir = if pyproject_path.exists() {
        project_dir.clone()
    } else {
        // Try to find it in the crate directory
        let crate_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let crate_pyproject = crate_path.join("pyproject.toml");
        if crate_pyproject.exists() {
            crate_path
        } else {
            return Err(crate::VoiceCliError::Config(
                "pyproject.toml not found in current directory or crate directory".to_string()
            ));
        }
    };
    
    println!("   Using project directory: {:?}", work_dir);

    // Create virtual environment if it doesn't exist
    let venv_path = work_dir.join(".venv");
    if !venv_path.exists() {
        println!("📦 Creating Python virtual environment...");
        let mut cmd = Command::new("uv");
        cmd.arg("venv")
           .current_dir(&work_dir);
        
        let output = cmd.output()
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
    
    // Install audio processing dependencies
    let dependencies = ["torch", "torchaudio", "numpy", "soundfile"];
    for dep in &dependencies {
        println!("   Installing {}...", dep);
        let mut cmd = Command::new("uv");
        cmd.arg("add")
           .arg(dep)
           .current_dir(&work_dir);
        
        let output = cmd.output()
            .map_err(|e| crate::VoiceCliError::Config(format!("Failed to install {}: {}", dep, e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("⚠️  {} installation warning: {}", dep, stderr);
        } else {
            println!("✅ {} installed successfully", dep);
        }
    }

    // Test the installation (check if audio libraries are available)
    println!("🧪 Testing TTS installation...");
    let test_script = r#"
import sys
try:
    import torch
    import torchaudio
    import numpy as np
    import soundfile as sf
    print("Audio processing libraries imported successfully")
    print(f"PyTorch version: {torch.__version__}")
    print(f"Torchaudio version: {torchaudio.__version__}")
    print(f"NumPy version: {np.__version__}")
    print(f"SoundFile version: {sf.__version__}")
    
    # Test if index-tts is available (optional)
    try:
        import index_tts
        print("index-tts is available - using real TTS")
        HAS_REAL_TTS = True
    except ImportError:
        print("index-tts not available - using mock TTS implementation")
        HAS_REAL_TTS = False
    
except ImportError as e:
    print(f"Failed to import audio libraries: {e}")
    sys.exit(1)
except Exception as e:
    print(f"Error testing audio libraries: {e}")
    sys.exit(1)
"#;

    let mut cmd = Command::new("uv");
    cmd.arg("run")
       .arg("python")
       .arg("-c")
       .arg(test_script)
       .current_dir(&work_dir);
    
    let output = cmd.output()
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
