pub mod handlers;
pub mod http_tracing;
pub mod middleware;
pub mod middleware_config;
pub mod routes;
pub mod stt_stream;
pub mod tts_stream;

use crate::models::Config;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};

/// 解析 WS 文本帧是否为控制帧（`{type:"<ty>"}`），避免 contains 误判（如 "nonstop"）。
/// stt_stream（stop）/ tts_stream（cancel）共用。
pub fn is_control_frame(text: &str, ty: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
        .is_some_and(|frame_ty| frame_ty.eq_ignore_ascii_case(ty))
}

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

/// Initialize server configuration from code defaults (`Config::default()`).
pub async fn handle_server_init(config_path: Option<PathBuf>, force: bool) -> crate::Result<()> {
    let output_path = config_path.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("config.yml")
    });

    // 检查文件是否已存在
    if output_path.exists() && !force {
        println!("❌ Configuration file already exists: {:?}", output_path);
        println!("💡 Use --force to overwrite, or specify a different path with --config");
        return Ok(());
    }

    // 从代码默认值生成配置文件
    crate::config::ConfigTemplateGenerator::generate_config_file(
        crate::config::ServiceType::Server,
        &output_path,
    )?;

    println!(
        "✅ Server configuration initialized from code defaults: {:?}",
        output_path
    );

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

    // 配置 STT 全局 GPU 加速（幂等；macOS=metal/CoreML，Linux=cpu 或 --features cuda/vulkan）。
    // 设备由 config.whisper.engine.device 决定（默认 "auto"=平台 GPU）。
    crate::stt::accel::init_global_accel(&config.whisper.engine.device);

    // 初始化 sherpa FireRedASR2 标点恢复（幂等；punct=true 且模型存在才加载，否则 add_punct 透传）。
    // 仅 FireRedASR2-AED 无标点输出需要；Fun/Qwen 自带 LLM 标点，调用方按 kind 跳过。
    {
        let sh = &config.whisper.engine.sherpa;
        if sh.punct {
            let punc_onnx = match &sh.punct_model_dir {
                Some(d) => {
                    std::path::PathBuf::from(d).join(crate::stt::sherpa_punc::PUNCT_MODEL_FILE)
                }
                None => std::path::Path::new(&config.whisper.models_dir)
                    .join("punct")
                    .join(crate::stt::sherpa_punc::DEFAULT_PUNCT_DIR)
                    .join(crate::stt::sherpa_punc::PUNCT_MODEL_FILE),
            };
            crate::stt::sherpa_punc::init(
                Some(&punc_onnx),
                sh.provider.as_deref(),
                sh.num_threads,
                sh.debug,
            );
        }
    }

    // 初始化 ZipVoice 预置音色组（幂等；仅 backend=zipvoice 且 tts.enabled 才加载）。
    // 任意 profile 的 reference_wav 缺失/解码失败 → Fail Fast 启动报错（配置错误不应静默）。
    if config.tts.enabled
        && matches!(
            config.tts.engine.backend,
            crate::models::config::TtsBackend::ZipVoice
        )
    {
        crate::tts::init_reference_profiles(&config.tts.engine.zipvoice.voices)?;
    }

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
    // SO_REUSEADDR：bind 允许接管处于 TIME_WAIT 的端口——重启/升级场景下，
    // 健康探测的 curl 连接在旧实例死后残留 TIME_WAIT（Windows 默认拒绝 bind
    // 此类端口，os error 10048；53 实测升级间歇失败的根因）。unix 上同为
    // 服务端标准实践。注意：bind 失败前不打 "listening" 字样（排障误导）。
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .map_err(|e| crate::VoiceCliError::Config(format!("Failed to create socket on {addr}: {e}")))?;
    socket
        .set_reuse_address(true)
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to set SO_REUSEADDR: {e}")))?;
    socket
        .bind(&addr.into())
        .map_err(|e| {
            crate::VoiceCliError::Config(format!(
                "Failed to bind to address {addr}（端口被占用或无权限；若有旧实例正在关闭，稍候重试）: {e}"
            ))
        })?;
    socket
        .listen(1024)
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to listen on {addr}: {e}")))?;
    let listener = tokio::net::TcpListener::from_std(socket.into())
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to convert listener: {e}")))?;

    info!(
        "TCP listener created successfully: {:?}",
        listener.local_addr()
    );
    info!("Starting axum server...");

    // 预热 TTS 默认引擎（listen 前 await，确保首个请求命中缓存；代价：启动多 ~10-20s）。
    // warmup=true 且 enabled 才预热；失败仅 warn（不阻塞服务，首次请求将报错）。
    if config.tts.enabled && config.tts.engine.warmup {
        let engine_cfg = app_state.config.tts.engine.clone();
        let model_svc = app_state.tts_model_service.clone();
        let model_id = engine_cfg.default_model.clone();
        let model_id_log = model_id.clone(); // model_id 将 move 进 spawn_blocking，留一份给日志
        info!(model = %model_id_log, "🚀 预热 TTS 引擎（listen 前，~10-20s，完成后接请求）...");
        let started = std::time::Instant::now();
        match tokio::task::spawn_blocking(move || {
            crate::tts::acquire_instance(
                &model_svc,
                &model_id,
                &engine_cfg,
                engine_cfg.default_length_scale,
            )
        })
        .await
        {
            Ok(Ok(_)) => info!(
                model = %model_id_log,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "✅ TTS 预热完成（首个请求将命中缓存）"
            ),
            Ok(Err(e)) => warn!(
                model = %model_id_log,
                "⚠️ TTS 预热失败（不阻塞，首次请求将报错）: {e}"
            ),
            Err(e) => warn!("TTS 预热 join 失败: {e}"),
        }
    }

    let http = async {
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal_with_broadcast(shutdown_tx))
            .await
            .map_err(std::io::Error::other);

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
        if let Err(e) = app_state_for_monitor
            .lock_free_apalis_manager
            .shutdown()
            .await
        {
            warn!("Failed to shutdown Apalis manager gracefully: {}", e);
        }

        Ok::<(), std::io::Error>(())
    };

    let _res = tokio::join!(http, monitor);

    Ok(())
}
