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

use crate::tts::TtsOptions;
use crate::tts::error::TtsError;
use crate::tts::synthesizer::Synthesizer;

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
    pub language: Option<String>,
    pub model: String,
}

/// 流式合成（在 `spawn_blocking` 中调用）。
///
/// `synth` 任意 [`Synthesizer`] 实现：生产用 `OfflineTts`（调用方先 `acquire_instance` +
/// `lock` 取 `&OfflineTts`），测试用 `MockSynthesizer`。泛型化使 callback/event 编排可单测。
///
/// `tx` 推事件给 WS forwarder；`cancel` 外部置位时 callback 中断 C 端合成。
/// 错误映射到 `TtsStreamEvent::Error` 后返回（不向上传播，便于 forwarder 收尾）。
pub fn synthesize_streaming<S: Synthesizer>(
    synth: &S,
    opts: TtsOptions,
    cfg: TtsStreamConfig,
    tx: mpsc::Sender<TtsStreamEvent>,
    cancel: Arc<AtomicBool>,
) -> Result<(), TtsError> {
    // 预清洗 interior NUL（CString::new 会 panic）
    let clean_text: String = cfg.text.trim().chars().filter(|c| *c != '\0').collect();
    if clean_text.is_empty() {
        return Err(TtsError::InvalidInput("text 清洗后为空".to_string()));
    }

    // 先发 Ready（用引擎报告的 sample_rate）
    let sample_rate = synth.sample_rate();
    // blocking_send：本函数运行在 spawn_blocking 线程，不能 await，用阻塞版
    if tx
        .blocking_send(TtsStreamEvent::Ready { sample_rate })
        .is_err()
    {
        return Ok(()); // 接收方已关闭（client 断开），直接结束
    }

    let gen_cfg = opts.to_generation_config();
    // callback：把引擎给的样本 chunk 经 mpsc 推出；cancel 置位时返回 false 中断。
    // 不依赖增量假设——即使 callback 给全集，也只是一次大 Audio 帧（等价整段合成）。
    let tx_cb = tx.clone();
    let cancel_cb = cancel.clone();
    let callback = move |samples: &[f32], progress: f32| -> bool {
        // Fail Fast：外部 cancel 立即中断合成循环
        if cancel_cb.load(Ordering::Relaxed) {
            return false;
        }
        // 推增量样本（to_vec 取所有权，跨 mpsc Send 安全）。
        // try_send 而非 blocking_send：channel 满且消费端停滞时 blocking_send
        // 会无限阻塞——cancel 检查点已过、后续 callback 永远不来，引擎锁被
        // 该会话无限持有（拖垮全部 TTS 入口）。Full → 丢弃本块继续（消费端
        // 停滞的会话本已超时，丢帧无损；cancel 在下一个 callback 生效，锁的
        // 释放从"无限"变为一个 chunk 周期）；Closed → 中断合成
        match tx_cb.try_send(TtsStreamEvent::Audio {
            samples: samples.to_vec(),
            progress,
        }) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(
                    "tts callback: channel full (consumer stalled?), dropping audio chunk"
                );
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                return false; // 接收方关闭 → 中断合成
            }
        }
        true
    };

    let result = synth.generate(&clean_text, &gen_cfg, Some(callback));

    match result {
        Some(total_samples) => {
            let _ = tx.blocking_send(TtsStreamEvent::Done {
                total_samples,
                sample_rate,
            });
            Ok(())
        }
        None => {
            let _ = tx.blocking_send(TtsStreamEvent::Error {
                message: format!(
                    "generate 返回 None（text 长度 {}，sid={}）",
                    clean_text.chars().count(),
                    opts.sid
                ),
            });
            Err(TtsError::SynthFailed("generate 返回 None".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sherpa_onnx::GenerationConfig;

    /// mock 引擎：按预设 chunks 顺序喂给 callback；fail=true 直接返回 None。
    struct MockSynthesizer {
        sample_rate: i32,
        chunks: Vec<Vec<f32>>,
        fail: bool,
    }

    impl Synthesizer for MockSynthesizer {
        fn sample_rate(&self) -> i32 {
            self.sample_rate
        }
        fn generate<F>(
            &self,
            _text: &str,
            _cfg: &GenerationConfig,
            mut callback: Option<F>,
        ) -> Option<usize>
        where
            F: FnMut(&[f32], f32) -> bool + 'static,
        {
            if self.fail {
                return None;
            }
            let n = self.chunks.len().max(1);
            let mut total = 0;
            for (i, chunk) in self.chunks.iter().enumerate() {
                let progress = (i + 1) as f32 / n as f32;
                if let Some(cb) = callback.as_mut()
                    && !cb(chunk, progress)
                {
                    return None; // callback 返回 false（cancel / 接收方关闭）→ 中断
                }
                total += chunk.len();
            }
            Some(total)
        }
    }

    fn cfg_with_text(text: &str) -> TtsStreamConfig {
        TtsStreamConfig {
            text: text.into(),
            sid: 0,
            speed: 1.0,
            language: None,
            model: "mock".into(),
        }
    }

    fn drain(rx: &mut mpsc::Receiver<TtsStreamEvent>) -> Vec<TtsStreamEvent> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    #[test]
    fn streaming_normal_ready_audio_done() {
        let synth = MockSynthesizer {
            sample_rate: 24000,
            chunks: vec![vec![0.1; 100], vec![0.2; 200]],
            fail: false,
        };
        let (tx, mut rx) = mpsc::channel::<TtsStreamEvent>(32);
        let cancel = Arc::new(AtomicBool::new(false));
        synthesize_streaming(
            &synth,
            TtsOptions::default(),
            cfg_with_text("hi"),
            tx,
            cancel,
        )
        .unwrap();
        let events = drain(&mut rx);
        assert_eq!(events.len(), 4);
        assert!(matches!(
            events[0],
            TtsStreamEvent::Ready { sample_rate: 24000 }
        ));
        assert!(matches!(events[1], TtsStreamEvent::Audio { .. }));
        assert!(matches!(events[2], TtsStreamEvent::Audio { .. }));
        // 100 + 200 = 300 samples
        assert!(matches!(
            events[3],
            TtsStreamEvent::Done {
                total_samples: 300,
                sample_rate: 24000
            }
        ));
    }

    #[test]
    fn streaming_empty_text_returns_error_no_events() {
        let synth = MockSynthesizer {
            sample_rate: 24000,
            chunks: vec![vec![0.0; 10]],
            fail: false,
        };
        let (tx, mut rx) = mpsc::channel::<TtsStreamEvent>(32);
        let cancel = Arc::new(AtomicBool::new(false));
        let err = synthesize_streaming(
            &synth,
            TtsOptions::default(),
            cfg_with_text("  "),
            tx,
            cancel,
        )
        .unwrap_err();
        assert!(matches!(err, TtsError::InvalidInput(_)));
        assert!(drain(&mut rx).is_empty(), "空文本不应推任何事件");
    }

    #[test]
    fn streaming_generate_none_emits_error() {
        let synth = MockSynthesizer {
            sample_rate: 24000,
            chunks: vec![],
            fail: true,
        };
        let (tx, mut rx) = mpsc::channel::<TtsStreamEvent>(32);
        let cancel = Arc::new(AtomicBool::new(false));
        let res = synthesize_streaming(
            &synth,
            TtsOptions::default(),
            cfg_with_text("hi"),
            tx,
            cancel,
        );
        assert!(res.is_err());
        let events = drain(&mut rx);
        // Ready（首事件）+ Error（generate None）
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], TtsStreamEvent::Ready { .. }));
        assert!(matches!(events[1], TtsStreamEvent::Error { .. }));
    }

    #[test]
    fn streaming_cancel_before_first_chunk_stops_synth() {
        let synth = MockSynthesizer {
            sample_rate: 24000,
            chunks: vec![vec![0.1; 100], vec![0.2; 200]],
            fail: false,
        };
        let (tx, mut rx) = mpsc::channel::<TtsStreamEvent>(32);
        let cancel = Arc::new(AtomicBool::new(true)); // 预置 cancel
        let res = synthesize_streaming(
            &synth,
            TtsOptions::default(),
            cfg_with_text("hi"),
            tx,
            cancel,
        );
        // callback 首次即返回 false → MockSynthesizer 中断 → None → Err
        assert!(res.is_err());
        let events = drain(&mut rx);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], TtsStreamEvent::Ready { .. }));
        assert!(matches!(events[1], TtsStreamEvent::Error { .. }));
    }
}
