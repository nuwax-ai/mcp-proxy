pub mod handlers;
pub mod middleware;
pub mod routes;
pub mod http_tracing;
pub mod middleware_config;
pub mod app_state;

use crate::models::Config;
use crate::services::StepContext;
use apalis::prelude::*;
use apalis::layers::retry::RetryPolicy;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;




async fn shutdown_signal() {
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
    println!("📝 Edit the configuration file and run:");
    println!("   voice-cli server run --config {:?}", output_path);

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
    
    // 如果启用了任务管理，添加 storage 作为 Extension
    if config.task_management.enabled {
        if let Some(storage) = &app_state.apalis_storage {
            app = app.layer(axum::Extension(storage.clone()));
        }
    }

    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    info!("Server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to bind to address: {}", e)))?;
    
    info!("TCP listener created successfully: {:?}", listener.local_addr());
    info!("Starting axum server...");

    let http = async {
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
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
        if config.task_management.enabled {
            if let Some(apalis_manager) = &app_state_for_monitor.apalis_manager {
                if let Some(storage) = &app_state_for_monitor.apalis_storage {
                    let manager = apalis_manager.lock().await;
                    let step_context = StepContext {
                        transcription_engine: Arc::new(crate::services::TranscriptionEngine::new(app_state_for_monitor.model_service.clone())),
                        audio_file_manager: Arc::new(
                            crate::services::AudioFileManager::new("./data/audio")
                                .map_err(|e| crate::VoiceCliError::Storage(format!("创建音频文件管理器失败: {}", e)))
                                .unwrap_or_else(|_| panic!("Failed to create audio file manager")),
                        ),
                        pool: manager.pool.clone(),
                    };

                    info!("Apalis 监控器开始运行，等待任务...");
                    Monitor::new()
                        .register(
                            WorkerBuilder::new("transcription-pipeline")
                                .data(step_context)
                                .enable_tracing()
                                .concurrency(config.task_management.max_concurrent_tasks)
                                .retry(RetryPolicy::retries(config.task_management.retry_attempts))
                                .backend(storage.clone())
                                .build_fn(crate::services::transcription_pipeline_worker)
                        )
                        .run()
                        .await
                        .unwrap();
                }
            }
        }
        Ok::<(), std::io::Error>(())
    };

    let _res = tokio::join!(http, monitor);

    Ok(())
}
