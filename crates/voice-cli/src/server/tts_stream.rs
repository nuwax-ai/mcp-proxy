//! TTS 流式 WebSocket 端点（sherpa-onnx callback → 增量 PCM）。
//!
//! 协议：
//! - 客户端 → 服务端：首帧 JSON `{type:"start", text, sid?, speed?, length_scale?,
//!   language?, model?, format?}`；`{type:"cancel"}` 或关闭连接结束。
//! - 服务端 → 客户端：`{type:"ready", sample_rate}` → 二进制 PCM s16le 帧（多帧）
//!   → `{type:"done", total_samples, sample_rate}` / `{type:"error", message}`。
//!
//! 流式只发裸 PCM s16le（无 WAV 头：流式无法预知总长度）；`ready` 给出采样率，
//! 客户端可自行封装 WAV。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tracing::{info, warn};

use crate::server::handlers::AppState;
use crate::tts::{TtsOptions, TtsStreamConfig, TtsStreamEvent, synthesize_streaming};

/// 客户端 start 帧（未知字段 serde 默认忽略）。
#[derive(Debug, Deserialize, Default)]
struct TtsStreamStartFrame {
    text: Option<String>,
    sid: Option<i32>,
    speed: Option<f32>,
    length_scale: Option<f32>,
    language: Option<String>,
    model: Option<String>,
}

/// GET /api/v1/stream/tts（WebSocket 升级）
pub async fn ws_tts_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run_tts_stream_session(socket, state))
}

async fn run_tts_stream_session(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();

    // 1. 等 start 帧（10s 超时）
    let start = match tokio::time::timeout(Duration::from_secs(10), stream.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => {
            serde_json::from_str::<TtsStreamStartFrame>(&t).unwrap_or_default()
        }
        _ => {
            warn!("tts stream: 未收到合法 start 帧");
            return;
        }
    };

    // Fail Fast：TTS 未启用 / 文本缺失立即拒
    if !state.config.tts.enabled {
        let _ = send_event(
            &mut sink,
            TtsStreamEvent::Error {
                message: "TTS service is disabled".to_string(),
            },
        )
        .await;
        return;
    }
    let text = match start.text.as_deref().map(str::trim) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => {
            let _ = send_event(
                &mut sink,
                TtsStreamEvent::Error {
                    message: "start 帧 text 缺失或为空".to_string(),
                },
            )
            .await;
            return;
        }
    };

    let engine = &state.config.tts.engine;
    let model_id = start.model.unwrap_or_else(|| engine.default_model.clone());
    if let Err(e) = state.tts_model_service.ensure_model(&model_id) {
        let _ = send_event(
            &mut sink,
            TtsStreamEvent::Error {
                message: format!("模型未就绪: {e}"),
            },
        )
        .await;
        return;
    }

    let opts = TtsOptions {
        sid: start.sid.unwrap_or(engine.default_sid),
        speed: start.speed.unwrap_or(engine.default_speed),
        ..Default::default()
    };
    let stream_cfg = TtsStreamConfig {
        text,
        sid: opts.sid,
        speed: opts.speed,
        language: start.language,
        model: model_id.clone(),
    };
    let length_scale = start.length_scale.unwrap_or(engine.default_length_scale);

    // 2. mpsc 通道 + 取消标志。三个并发任务：
    //    - synth（spawn_blocking）：acquire_instance + 合成 + 推事件
    //    - forward：mpsc rx → WS sink（owns sink），Done/Error 时自然退出
    //    - watcher：读客户端 cancel/close → 置 cancel → callback 中断合成
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<TtsStreamEvent>(32);
    let cancel = Arc::new(AtomicBool::new(false));

    let model_svc = state.tts_model_service.clone();
    let engine_cfg = state.config.tts.engine.clone();
    let model_id_for_closure = model_id.clone();
    let cancel_for_synth = cancel.clone();
    let synth_handle = tokio::task::spawn_blocking(move || {
        // acquire_instance 失败 → 推 Error 事件（不向上传播，便于 forward 收尾）
        let inst = match crate::tts::acquire_instance(
            &model_svc,
            &model_id_for_closure,
            &engine_cfg,
            length_scale,
        ) {
            Ok(i) => i,
            Err(e) => {
                let _ = event_tx.blocking_send(TtsStreamEvent::Error {
                    message: format!("引擎初始化失败: {e}"),
                });
                return;
            }
        };
        // lock 引擎取 &OfflineTts（impl Synthesizer），交给 synthesize_streaming
        let guard = inst.lock().unwrap_or_else(|p| p.into_inner());
        // 错误已在内部映射成 TtsStreamEvent::Error 推给 forward；此处忽略返回
        let _ = synthesize_streaming(&*guard, opts, stream_cfg, event_tx, cancel_for_synth);
    });

    // forward：owns sink，收到 Done 退出
    let forward_handle = tokio::spawn(async move {
        let mut rx = event_rx;
        while let Some(evt) = rx.recv().await {
            let is_done = matches!(evt, TtsStreamEvent::Done { .. });
            if send_event(&mut sink, evt).await.is_err() {
                break;
            }
            if is_done {
                break;
            }
        }
    });

    // watcher：监听客户端 cancel/close，置 cancel 标志（callback 检测后中断合成）
    let cancel_w = cancel.clone();
    let watcher_handle = tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(Message::Text(t)) if t.to_ascii_lowercase().contains("cancel") => {
                    warn!("tts stream: 收到 cancel，中断合成");
                    cancel_w.store(true, Ordering::Release);
                    break;
                }
                Ok(Message::Close(_)) | Err(_) => {
                    cancel_w.store(true, Ordering::Release);
                    break;
                }
                _ => {}
            }
        }
    });

    // 3. 合成完成（forward 退出）→ 兜底 cancel + 清理两个任务。
    // forward_handle.await 必须有超时兜底：若 OfflineTts::create 在模型加载阶段挂住
    // （callback 尚未注册，cancel 标志无效），forward 会永远阻塞在 rx.recv()。
    // 注意：超时后 spawn_blocking 任务无法真正取消（blocking 线程不能被中断），
    // 但至少让 WS 会话能结束、客户端拿到关闭，而不是无限挂起。
    let synth_timeout = Duration::from_secs(state.config.tts.streaming.synth_timeout_sec.max(1));
    let forward_timed_out = tokio::time::timeout(synth_timeout, forward_handle)
        .await
        .is_err();
    if forward_timed_out {
        warn!(model = %model_id, timeout_sec = %synth_timeout.as_secs(), "tts stream forward 超时（模型加载挂死？）");
    }
    cancel.store(true, Ordering::Release); // 兜底：让 callback（若已注册）尽快退出
    // 给 synth 一个短窗口响应 cancel 后自然结束；不无限等（blocking 任务不可强杀）
    let _ = tokio::time::timeout(Duration::from_secs(5), synth_handle).await;
    watcher_handle.abort();

    info!(model = %model_id, timed_out = forward_timed_out, "tts stream session end");
}

async fn send_event(
    sink: &mut SplitSink<WebSocket, Message>,
    evt: TtsStreamEvent,
) -> Result<(), axum::Error> {
    let msg = match evt {
        TtsStreamEvent::Ready { sample_rate } => {
            let json = serde_json::json!({
                "type": "ready",
                "sample_rate": sample_rate,
            });
            Message::Text(json.to_string().into())
        }
        TtsStreamEvent::Audio { samples, progress } => {
            // f32 → PCM s16le bytes
            let pcm = crate::tts::to_pcm_bytes(&samples);
            // 进度随 PCM 一并上报（header 无法，故发 Text 进度紧邻 Binary）
            // 简化：仅发 Binary；progress 信息通过 done 总量推算
            let _ = progress;
            Message::Binary(pcm.into())
        }
        TtsStreamEvent::Done {
            total_samples,
            sample_rate,
        } => {
            let json = serde_json::json!({
                "type": "done",
                "total_samples": total_samples,
                "sample_rate": sample_rate,
            });
            Message::Text(json.to_string().into())
        }
        TtsStreamEvent::Error { message } => {
            let json = serde_json::json!({ "type": "error", "message": message });
            Message::Text(json.to_string().into())
        }
    };
    sink.send(msg).await
}
