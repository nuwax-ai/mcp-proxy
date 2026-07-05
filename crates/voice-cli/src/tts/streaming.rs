//! TTS 流式合成（sherpa-onnx callback → mpsc 增量样本）。
//!
//! # 协议（与 `server/tts_stream.rs` WS handler 配合）
//! - 输入：一次性文本（`start` 帧携带 text/sid/speed/length_scale/language/model/format）
//! - 输出事件（经 mpsc，由 WS handler 转发）：
//!   - `Ready { sample_rate }`：合成开始（首事件）
//!   - `Audio { samples, progress }`：增量 PCM f32 样本（可能多帧）
//!   - `Done { total_samples, sample_rate }`：合成完成
//!   - `Error { message }`：失败
//!
//! # callback 粒度风险（plan §3.3）
//! sherpa-onnx 的 progress callback 由 C++ 决定触发频率：
//! - **理想**：callback 给增量 chunk → 真低延迟流式
//! - **降级**：callback 给累积全集 → 等价"整段合成 + 一次推送"（首字节延迟 = 合成总时长）
//!
//! 两种情况本实现都正确处理（callback 每次 to_vec 推一帧，不依赖增量假设）。
//!
//! # 取消
//! callback 返回 `false` 中断 C 端合成（真取消）。WS 关闭 / `cancel` 帧 →
//! `cancel` AtomicBool 置位 → callback 检测到后返回 false。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::mpsc;

use crate::tts::error::TtsError;
use crate::tts::{TtsKey, TtsLoadParams, TtsModelService, TtsOptions, get_or_init_tts};

/// 流式合成事件（WS handler 转成 WS 帧）。
#[derive(Debug)]
pub enum TtsStreamEvent {
    /// 合成就绪（首事件），携带采样率
    Ready { sample_rate: i32 },
    /// 增量样本（f32 PCM，归一化 [-1,1]）+ 进度 [0,1]
    Audio { samples: Vec<f32>, progress: f32 },
    /// 合成完成（末事件）
    Done {
        total_samples: usize,
        sample_rate: i32,
    },
    /// 失败
    Error { message: String },
}

/// 流式合成参数。
#[derive(Debug, Clone)]
pub struct TtsStreamConfig {
    pub text: String,
    pub sid: i32,
    pub speed: f32,
    pub length_scale: f32,
    pub language: Option<String>,
    pub model: String,
}

/// 同步流式合成（在 `spawn_blocking` 中调用）。
///
/// `tx` 推事件给 WS forwarder；`cancel` 外部置位时 callback 中断 C 端合成。
/// 错误映射到 `TtsStreamEvent::Error` 后返回（不向上传播，便于 forwarder 收尾）。
pub fn synthesize_streaming(
    tts_model_service: &TtsModelService,
    load_params: TtsLoadParams,
    opts: TtsOptions,
    cfg: TtsStreamConfig,
    tx: mpsc::Sender<TtsStreamEvent>,
    cancel: Arc<AtomicBool>,
) -> Result<(), TtsError> {
    // 模型校验已在调用方 ensure_model 完成；这里再 resolve paths（防竞态）
    let paths = tts_model_service.resolve_paths(&cfg.model)?;
    let load_params = TtsLoadParams {
        paths,
        ..load_params
    };

    let pool = get_or_init_tts(TtsKey::new(&cfg.model), load_params)?;
    let inst = pool.pick();
    let guard = inst.lock().unwrap_or_else(|p| p.into_inner());

    // 预清洗 interior NUL（CString::new 会 panic）
    let clean_text: String = cfg.text.trim().chars().filter(|c| *c != '\0').collect();
    if clean_text.is_empty() {
        return Err(TtsError::InvalidInput("text 清洗后为空".to_string()));
    }

    // 先发 Ready（用引擎报告的 sample_rate；从已有的 inst 取）
    let sample_rate = guard.sample_rate();
    // blocking_send：本函数运行在 spawn_blocking 线程，不能 await，用阻塞版
    if tx
        .blocking_send(TtsStreamEvent::Ready { sample_rate })
        .is_err()
    {
        return Ok(()); // 接收方已关闭（client 断开），直接结束
    }

    let gen_cfg = opts.to_generation_config();
    // callback：把 C 端给的样本 chunk 经 mpsc 推出；cancel 置位时返回 false 中断。
    // 不依赖增量假设——即使 callback 给全集，也只是一次大 Audio 帧（等价整段合成）。
    let tx_cb = tx.clone();
    let cancel_cb = cancel.clone();
    let callback = move |samples: &[f32], progress: f32| -> bool {
        // Fail Fast：外部 cancel 立即中断 C 端循环
        if cancel_cb.load(Ordering::Relaxed) {
            return false;
        }
        // 推增量样本（to_vec 取所有权，跨 mpsc Send 安全）
        if tx_cb
            .blocking_send(TtsStreamEvent::Audio {
                samples: samples.to_vec(),
                progress,
            })
            .is_err()
        {
            return false; // 接收方关闭 → 中断合成
        }
        true
    };

    let result = guard.generate_with_config(&clean_text, &gen_cfg, Some(callback));
    drop(guard);

    match result {
        Some(audio) => {
            // Done：用 GeneratedAudio 全集长度（权威值）
            let _ = tx.blocking_send(TtsStreamEvent::Done {
                total_samples: audio.samples().len(),
                sample_rate,
            });
            Ok(())
        }
        None => {
            let _ = tx.blocking_send(TtsStreamEvent::Error {
                message: format!(
                    "generate_with_config 返回 None（text 长度 {}，sid={}）",
                    clean_text.chars().count(),
                    opts.sid
                ),
            });
            Err(TtsError::SynthFailed(
                "generate_with_config 返回 None".to_string(),
            ))
        }
    }
}
