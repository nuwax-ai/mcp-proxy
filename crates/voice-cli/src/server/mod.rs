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

/// 端口是否已有**活跃** listener（Windows netstat 解析；探测不可用返回 false
/// ——保持 REUSEADDR 开启，不因探测失败放弃 TIME_WAIT 修复）。用于 SO_REUSEADDR
/// 的前置守卫：Windows 的 REUSEADDR 允许与活跃 listener 双绑（连接被随机分配），
/// 误配端口的两个服务会静默互偷连接——有活跃占用时不开 REUSEADDR，让 bind
/// 以 os error 10048 明确失败。
#[cfg(windows)]
fn port_listener_count(port: u16) -> usize {
    let Ok(out) = std::process::Command::new("netstat")
        .args(["-ano", "-p", "tcp"])
        .output()
    else {
        return 0;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let port_str = port.to_string();
    stdout
        .lines()
        .filter(|line| {
            let mut cols = line.split_whitespace();
            if cols.next() != Some("TCP") {
                return false;
            }
            let Some(local) = cols.next() else {
                return false;
            };
            cols.next(); // remote
            let Some(state) = cols.next() else {
                return false;
            };
            state == "LISTENING" && local.rsplit(':').next() == Some(port_str.as_str())
        })
        .count()
}

/// 端口是否被**存活进程** LISTENING（强杀后的尸体 LISTENING 在 netstat 有
/// 数秒残留——PID 已死，不应据此禁用 REUSEADDR：53 实测强杀→precheck 误判
/// →bind 10048 连环失败）。netstat 拿 PID 后与 tasklist 存活集求交。
#[cfg(windows)]
fn port_live_listener_exists(port: u16) -> bool {
    let Ok(out) = std::process::Command::new("netstat")
        .args(["-ano", "-p", "tcp"])
        .output()
    else {
        return false;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let port_str = port.to_string();
    let mut pids: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let mut cols = line.split_whitespace();
        if cols.next() != Some("TCP") {
            continue;
        }
        let Some(local) = cols.next() else { continue };
        cols.next();
        let Some(state) = cols.next() else { continue };
        let Some(pid) = cols.next() else { continue };
        if state == "LISTENING" && local.rsplit(':').next() == Some(port_str.as_str()) {
            pids.push(pid.to_string());
        }
    }
    if pids.is_empty() {
        return false;
    }
    let Ok(out) = std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()
    else {
        return true; // 探测不可用：保守认为占用
    };
    let tasks = String::from_utf8_lossy(&out.stdout);
    pids.iter().any(|pid| {
        tasks
            .lines()
            .any(|l| l.split(',').nth(1) == Some(&format!("\"{pid}\"")))
    })
}

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
    // SO_REUSEADDR：接管 TIME_WAIT 端口（重启/升级后健康探测的 curl 连接残留
    // TIME_WAIT，Windows 默认拒绝 bind，os error 10048）。**仅 Windows 上先探测
    // 活跃 listener**：Windows 的 SO_REUSEADDR 允许与活跃 listener 双绑同一端口
    //（不报错、连接随机分配——误配端口的两个服务会静默互偷连接；Linux 的
    // REUSEADDR 只复用 TIME_WAIT、双绑需 REUSEPORT，无此问题）。
    // 注意：bind 失败前不打 "listening" 字样（排障误导）。
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .map_err(|e| crate::VoiceCliError::Config(format!("Failed to create socket on {addr}: {e}")))?;
    #[cfg(windows)]
    let may_reuse = {
        let active = port_live_listener_exists(config.server.port);
        info!(
            port = config.server.port,
            active_listener = active,
            reuse = if active {
                "disabled (port in active use)"
            } else {
                "enabled (TIME_WAIT takeover)"
            },
            "bind precheck: SO_REUSEADDR decision"
        );
        !active
    };
    #[cfg(not(windows))]
    let may_reuse = true;
    if may_reuse {
        socket.set_reuse_address(true).map_err(|e| {
            crate::VoiceCliError::Config(format!("Failed to set SO_REUSEADDR: {e}"))
        })?;
    }
    socket
        .bind(&addr.into())
        .map_err(|e| {
            crate::VoiceCliError::Config(format!(
                "Failed to bind to address {addr}（端口被占用或无权限；若有旧实例正在关闭，稍候重试）: {e}"
            ))
        })?;
    // Windows 双绑终检（bind 后、listen 前）：python 等程序默认给 listener 设
    // SO_REUSEADDR，我方即使不设任何选项 bind 也能成功（53 实测）。此刻本
    // 进程尚未 listen——netstat 里同端口的**任何** LISTENING 都属于其它进程
    //（含 bind 前守卫漏掉的 REUSEADDR 型占用）。在对外服务开始前 Fail Fast，
    // 避免终检失败实例的 listener 短暂存活被健康探测误判为服务成功
    #[cfg(windows)]
    {
        let others = port_listener_count(config.server.port);
        if others > 0 {
            // 只告警不阻断：LISTENING 无法区分"无关进程双绑"（对方设了
            // SO_REUSEADDR，罕见误配）与"本服务旧实例优雅关闭中的残留"
            //（常见）——阻断会把正常 restart 误杀（53 实测 upgrade 假失败）
            warn!(
                "port {} still shows {} listener(s) besides us — if these belong to another service, connections may be split between processes",
                config.server.port, others
            );
        }
    }
    socket
        .listen(1024)
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to listen on {addr}: {e}")))?;
    // tokio 的 from_std 要求 non-blocking：socket2 默认 blocking，不设的话
    // accept 行为未定义（实测：listener 在听、runtime 活着、accept 永久挂死）
    socket
        .set_nonblocking(true)
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to set non-blocking: {e}")))?;
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
