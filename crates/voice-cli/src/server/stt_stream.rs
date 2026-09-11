//! STT 流式 WebSocket 端点（LocalAgreement 2）。
//!
//! **流式与批量 backend 解耦**：流式引擎由 `whisper.streaming.engine` 声明（默认 whisper，
//! 唯一支持 LA2 真流式；sherpa/sensevoice 是离线模型），与批量 `backend` 无关。
//! 故 `backend: fireredasr2` 时批量=fireredasr2、流式=whisper，一个进程两不耽误。
//! 流式模型优先级：start 帧 `model` > `whisper.streaming.model` > `whisper.default_model`。
//!
//! 协议：
//! - 客户端 → 服务端：首帧 JSON `{type:"start",sample_rate?,language?,model?,initial_prompt?}`；
//!   后续二进制帧 = PCM s16le / 16k / mono；`{type:"stop"}` 或关闭连接结束。
//! - 服务端 → 客户端：`{type:"ready",sample_rate}` → `{type:"partial",text,committed}`
//!   → `{type:"committed",text,committed}` → `{type:"done",committed_total}`。
//! - **最终文本取 `done.committed_total`**（完整 buffer 单次解码，干净）；中间 `committed` 是
//!   尽力而为的增量（LA2 无 word-timestamp，前文修正偶发重复，属固有残留）。

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
use crate::stt::{SessionConfig, StreamEvent, StreamingSession, SttTranscribeOptions};

/// 客户端 start 帧（`type` 等未知字段由 serde 默认忽略）
#[derive(Debug, Deserialize, Default)]
struct StreamStartFrame {
    sample_rate: Option<u32>,
    language: Option<String>,
    model: Option<String>,
    initial_prompt: Option<String>,
}

/// GET /api/v1/stream/transcribe（WebSocket 升级）
#[utoipa::path(
    get,
    path = "/api/v1/stream/transcribe",
    tag = "流式转录",
    summary = "STT 流式 WebSocket（LocalAgreement 2 增量识别）",
    description = "WebSocket 升级接口。连接后首帧发 JSON {type:\"start\",sample_rate?,language?,model?,initial_prompt?}；后续发 PCM s16le / 16k / mono 二进制帧；{type:\"stop\"} 或断开结束。服务端推 ready → partial → committed → done，**最终文本取 done.committed_total**。swagger 无法测 WebSocket，请用 wscat / python websockets 客户端（见 docs/API.md §4）。",
    responses(
        (status = 101, description = "Switching Protocols（WebSocket 升级成功）"),
        (status = 426, description = "Upgrade Required（客户端不支持 WebSocket）"),
    )
)]
pub async fn ws_transcribe_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run_stream_session(socket, state))
}

async fn run_stream_session(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();

    // 流式**恒走 whisper**（LA2 token 级真流式），与 `backend` **解耦**：
    // - 批量（/transcribe）用 backend（可 fireredasr2/sensevoice/sherpa，准+标点）
    // - 流式（本端点）只能 whisper（sherpa/sensevoice 是离线模型，无流式能力）
    // 故 backend=fireredasr2 时批量走 fireredasr2、流式仍走 whisper，两不耽误，一个进程即可。
    // （下方 model 解析 → get_or_init_whisper，本就独立于 backend。）
    if !matches!(
        state.config.whisper.engine.backend,
        crate::models::config::SttBackend::Whisper
    ) {
        tracing::info!(
            "流式端点：backend={:?} 非流式模型，流式恒走 whisper（与批量解耦）",
            state.config.whisper.engine.backend
        );
    }

    // 1. 等 start 帧（10s 超时）——所有失败路径都先推 error 事件再关闭
    //（客户端能区分"自己 JSON 写错/超时"与网络断开，观测性对齐 TTS 端点）
    let start = match tokio::time::timeout(Duration::from_secs(10), stream.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => match parse_start(&t) {
            Ok(f) => f,
            Err(e) => {
                warn!("stream session: start 帧解析失败: {e}");
                let _ = send_event(
                    &mut sink,
                    StreamEvent::Error {
                        message: format!("start 帧解析失败: {e}"),
                    },
                )
                .await;
                return;
            }
        },
        _ => {
            warn!("stream session: 未收到合法 start 帧");
            let _ = send_event(
                &mut sink,
                StreamEvent::Error {
                    message: "10 秒内未收到合法 start 帧".to_string(),
                },
            )
            .await;
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

    // 2. 流式引擎 + 模型解析（与批量 backend 解耦：流式用 whisper.streaming.engine/model）
    let streaming_cfg = state.config.whisper.streaming.clone();
    // 模型优先级：请求帧 model > whisper.streaming.model > whisper.default_model
    let model_id = start
        .model
        .or(streaming_cfg.model.clone())
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
    // 提前 clone：opts 下方 move 进 decoder，session_cfg 仍需 language（granularity auto 推断依赖）
    let session_language = opts.language.clone();
    let session_initial_prompt = opts.initial_prompt.clone();

    // 工厂按 streaming.engine 构造解码器（单一 dispatch 点；加新流式引擎见 build_streaming_decoder）
    let decoder = match crate::stt::build_streaming_decoder(
        streaming_cfg.engine,
        model_id.clone(),
        model_path.clone(),
        pool_size,
        opts,
    ) {
        Ok(d) => d,
        Err(e) => {
            let _ = send_event(
                &mut sink,
                StreamEvent::Error {
                    message: format!(
                        "流式解码器构造失败（engine={:?}）: {e}",
                        streaming_cfg.engine
                    ),
                },
            )
            .await;
            return;
        }
    };
    // 提前捕获日志用值（model_id / streaming_cfg 随后 move 进 session_cfg → session）
    let log_model_id = model_id.clone();
    let log_engine = streaming_cfg.engine;
    let session_cfg = SessionConfig {
        sample_rate: 16000,
        language: session_language,
        initial_prompt: session_initial_prompt,
        model_id,
        model_path,
        pool_size,
        streaming: streaming_cfg,
        output_script: engine.output_script,
    };

    // 2.5 握手期预热引擎池：模型懒加载失败（文件缺失/损坏）在 ready 之前暴露，
    // 客户端立即收到 error 而非 ready 后首帧静默断开（2026-09-10 深测 53 实况）。
    // 限时 120s：模型冷加载最慢 ~60s（大模型 + 慢盘），2 倍裕量——无超时的
    // prepare 挂死会让客户端在 ready 前无限等待（静默挂起只是换了个位置）
    {
        const STT_PREPARE_TIMEOUT: Duration = Duration::from_secs(120);
        let warm = decoder.clone();
        let prepared = tokio::time::timeout(
            STT_PREPARE_TIMEOUT,
            tokio::task::spawn_blocking(move || warm.prepare()),
        )
        .await;
        let failure = match prepared {
            Ok(Ok(Ok(()))) => None,
            Ok(Ok(Err(e))) => Some(format!("流式引擎预热失败: {e}")),
            Ok(Err(e)) => Some(format!("流式引擎预热任务失败: {e}")),
            Err(_) => Some(format!(
                "流式引擎预热超时（{STT_PREPARE_TIMEOUT:?}）——模型加载挂起，请检查模型文件与磁盘"
            )),
        };
        if let Some(message) = failure {
            warn!("stream session: {message}");
            let _ = send_event(&mut sink, StreamEvent::Error { message }).await;
            return;
        }
    }

    // 3. mpsc + StreamingSession
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);
    // 主循环错误上报通道：sink 已 move 进转发任务，主循环只能经 event 通道发事件
    let error_tx = event_tx.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut session = StreamingSession::new(session_cfg, decoder.clone(), event_tx, cancel.clone());

    // 发 ready
    if send_event(&mut sink, StreamEvent::Ready { sample_rate: 16000 })
        .await
        .is_err()
    {
        return;
    }
    info!(
        "stream session ready: model={}, engine={:?}",
        log_model_id, log_engine
    );

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
                        // 经 event 通道告知客户端再退出（sink 在转发任务里）；
                        // 之后 finish 大概率同样失败，客户端至少拿到失败原因
                        let _ = error_tx
                            .send(StreamEvent::Error {
                                message: format!("音频处理失败: {e}"),
                            })
                            .await;
                        break;
                    }
                }
                Message::Text(t) => {
                    if crate::server::is_control_frame(&t, "stop") {
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
    let finish_result = session.finish().await;
    // 先释放两侧 sender 再 join 转发任务：finish 失败时无 Done 事件，
    // forwarder 只能靠通道关闭退出——若等 forward.await 之后才 drop 会互相等死锁
    drop(session);
    if let Err(e) = finish_result {
        warn!("stream session finish failed: {e}");
        // 收尾失败客户端收不到 Done——经 event 通道补 error，避免只看到连接断开
        let _ = error_tx
            .send(StreamEvent::Error {
                message: format!("会话收尾失败: {e}"),
            })
            .await;
    }
    drop(error_tx);
    let _ = forward.await;
}

/// 解析 start 帧：非法 JSON 返回错误（调用方推 error 事件）——此前
/// `unwrap_or_default()` 会静默落空值走默认模型，客户端无从得知帧写错了
fn parse_start(text: &str) -> Result<StreamStartFrame, String> {
    serde_json::from_str(text).map_err(|e| format!("非法 JSON: {e}"))
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
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c) as f32 / 32768.0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stt::SttError;
    use crate::stt::local_agreement::SttSegment;
    use crate::stt::streaming_session::Decoder;

    #[test]
    fn parse_start_accepts_valid_and_rejects_garbage() {
        // 合法帧：字段齐全
        let f = parse_start(r#"{"type":"start","sample_rate":16000,"model":"base"}"#)
            .expect("合法 start 帧应解析成功");
        assert_eq!(f.sample_rate, Some(16000));
        assert_eq!(f.model.as_deref(), Some("base"));

        // 合法帧：未知字段忽略（前向兼容）
        assert!(parse_start(r#"{"type":"start","future_field":1}"#).is_ok());

        // 非法 JSON：显式报错（此前 unwrap_or_default 静默落空值）
        let err = parse_start("not-json").expect_err("非法 JSON 应报错");
        assert!(err.contains("非法 JSON"), "报错应说明原因: {err}");

        // 缺 type 字段仍可解析（字段全部可选，与旧语义一致）
        assert!(parse_start("{}").is_ok());
    }

    /// 默认 prepare() 是 no-op（新流式引擎可选实现预热；Whisper 已实现）
    #[test]
    fn decoder_default_prepare_is_noop() {
        struct NoopDecoder;
        impl Decoder for NoopDecoder {
            fn decode(&self, _samples: &[f32]) -> Result<Vec<SttSegment>, SttError> {
                Ok(vec![])
            }
        }
        assert!(NoopDecoder.prepare().is_ok());
    }
}
