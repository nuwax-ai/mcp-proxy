//! STT 流式 WebSocket 端点（LocalAgreement 2）。
//!
//! 协议：
//! - 客户端 → 服务端：首帧 JSON `{type:"start",sample_rate?,language?,model?,initial_prompt?}`；
//!   后续二进制帧 = PCM s16le / 16k / mono；`{type:"stop"}` 或关闭连接结束。
//! - 服务端 → 客户端：`{type:"ready",sample_rate}` → `{type:"partial",text,committed}`
//!   → `{type:"committed",text,committed}` → `{type:"done",committed_total}`。

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
use crate::stt::{
    SessionConfig, StreamEvent, StreamingSession, SttTranscribeOptions, WhisperDecoder,
};

/// 客户端 start 帧（`type` 等未知字段由 serde 默认忽略）
#[derive(Debug, Deserialize, Default)]
struct StreamStartFrame {
    sample_rate: Option<u32>,
    language: Option<String>,
    model: Option<String>,
    initial_prompt: Option<String>,
}

/// GET /api/v1/stream/transcribe（WebSocket 升级）
pub async fn ws_transcribe_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run_stream_session(socket, state))
}

async fn run_stream_session(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();

    // 1. 等 start 帧（10s 超时）
    let start = match tokio::time::timeout(Duration::from_secs(10), stream.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => parse_start(&t),
        _ => {
            warn!("stream session: 未收到合法 start 帧");
            return;
        }
    };

    // 当前仅支持 16k mono PCM（Fail Fast：非 16k 立即拒绝，避免错误转码）
    if let Some(sr) = start.sample_rate
        && sr != 16000
    {
        let _ = send_event(
            &mut sink,
            StreamEvent::Error {
                message: format!("仅支持 16000Hz PCM，收到 {sr}Hz"),
            },
        )
        .await;
        return;
    }

    // 2. 模型解析 + ensure_model + 构造 decoder
    let model_id = start
        .model
        .unwrap_or_else(|| state.config.whisper.default_model.clone());
    if let Err(e) = state.model_service.ensure_model(&model_id).await {
        let _ = send_event(
            &mut sink,
            StreamEvent::Error {
                message: format!("模型加载失败: {e}"),
            },
        )
        .await;
        return;
    }
    let model_path = match state.model_service.get_model_path(&model_id) {
        Ok(p) => p,
        Err(e) => {
            let _ = send_event(
                &mut sink,
                StreamEvent::Error {
                    message: format!("模型路径失败: {e}"),
                },
            )
            .await;
            return;
        }
    };

    let engine = &state.config.whisper.engine;
    let pool_size = engine.pool_size;
    let opts = SttTranscribeOptions {
        language: start.language.or_else(|| engine.default_language.clone()),
        initial_prompt: start
            .initial_prompt
            .or_else(|| engine.default_initial_prompt.clone()),
        ..Default::default()
    };
    let streaming_cfg = state.config.whisper.streaming.clone();
    // 提前 clone：opts 下方 move 进 decoder，session_cfg 仍需 language（granularity auto 推断依赖）
    let session_language = opts.language.clone();
    let session_initial_prompt = opts.initial_prompt.clone();

    let decoder = Arc::new(WhisperDecoder {
        model_id: model_id.clone(),
        model_path: model_path.clone(),
        pool_size,
        opts,
    });
    let session_cfg = SessionConfig {
        sample_rate: 16000,
        language: session_language,
        initial_prompt: session_initial_prompt,
        model_id,
        model_path,
        pool_size,
        streaming: streaming_cfg,
    };

    // 3. mpsc + StreamingSession
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);
    let cancel = Arc::new(AtomicBool::new(false));
    let mut session = StreamingSession::new(session_cfg, decoder.clone(), event_tx, cancel.clone());

    // 发 ready
    if send_event(&mut sink, StreamEvent::Ready { sample_rate: 16000 })
        .await
        .is_err()
    {
        return;
    }
    info!("stream session ready: model={}", decoder.model_id);

    // 4. 事件转发任务（mpsc rx → socket）。收到 Done 后退出。
    let forward = tokio::spawn(async move {
        let mut rx = event_rx;
        while let Some(evt) = rx.recv().await {
            let is_done = matches!(evt, StreamEvent::Done { .. });
            if send_event(&mut sink, evt).await.is_err() {
                break;
            }
            if is_done {
                break;
            }
        }
    });

    // 5. 主循环：收 PCM / stop，idle 超时退出
    let idle_timeout = Duration::from_secs(state.config.whisper.streaming.idle_timeout_sec);
    loop {
        match tokio::time::timeout(idle_timeout, stream.next()).await {
            Err(_) => {
                warn!("stream session idle timeout ({}s)", idle_timeout.as_secs());
                break;
            }
            Ok(None) => break,
            Ok(Some(Err(e))) => {
                warn!("stream session ws recv error: {e}");
                break;
            }
            Ok(Some(Ok(msg))) => match msg {
                Message::Binary(bytes) => {
                    let samples = pcm_s16le_to_f32(&bytes);
                    if let Err(e) = session.push_samples(&samples).await {
                        warn!("stream session push_samples failed: {e}");
                        break;
                    }
                }
                Message::Text(t) => {
                    if is_control_frame(&t, "stop") {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            },
        }
    }

    // 6. finish（flush + Done）→ forwarder 收 Done 退出
    cancel.store(true, Ordering::Release);
    if let Err(e) = session.finish().await {
        warn!("stream session finish failed: {e}");
    }
    let _ = forward.await;
}

fn parse_start(text: &str) -> StreamStartFrame {
    serde_json::from_str(text).unwrap_or_default()
}

/// 解析 WS 文本帧是否为控制帧（`{type:"<ty>"}`），避免 contains 误判（如 "nonstop"）
fn is_control_frame(text: &str, ty: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
        .is_some_and(|frame_ty| frame_ty.eq_ignore_ascii_case(ty))
}

async fn send_event(
    sink: &mut SplitSink<WebSocket, Message>,
    evt: StreamEvent,
) -> Result<(), axum::Error> {
    let json = serde_json::to_string(&evt).unwrap_or_else(|_| "{}".to_string());
    sink.send(Message::Text(json.into())).await
}

/// PCM s16le bytes → f32 samples（归一化到 [-1.0, 1.0]）
fn pcm_s16le_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect()
}
