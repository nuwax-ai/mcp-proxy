//! STT 流式会话：audio buffer + LA2 状态机 + 双解码调度。
//!
//! 客户端推送 PCM 帧（16k mono f32），本会话累积到 audio_buffer，
//! 每 `decode_interval_sec` 触发双解码（A=完整 buffer，B=裁剪尾部 `tail_trim_sec`），
//! decoder 返回 **带时间戳的 segments**，经 LA2（token-based：segment 内拆词对齐）取公共前缀，
//! 稳定后 commit；事件推送到 mpsc 供 WS handler 转发客户端。
//!
//! 解码抽象为 [`Decoder`] trait，便于单测注入 mock（真实实现 [`WhisperDecoder`]）。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::mpsc;
use tracing::warn;

use crate::models::config::StreamingConfig;
use crate::stt::local_agreement::{CompareGranularity, LaConfig, LocalAgreement, SttSegment};
use crate::stt::{EngineKey, SttError, SttTranscribeOptions, get_or_init_engine};

/// 服务端推送事件（WS 文本帧，JSON）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StreamEvent {
    /// 会话就绪（首帧，回传 sample_rate）
    Ready { sample_rate: u32 },
    /// 未确认尾部预览（committed + text = 实时完整文本）
    Partial { text: String, committed: String },
    /// 新确认的增量文本
    Committed { text: String, committed: String },
    /// 会话结束，committed 为最终完整文本
    Done { committed_total: String },
    /// 错误
    Error { message: String },
}

/// 会话配置（客户端 start 帧 + 服务端 config 合并）
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// 16000（当前仅支持 16k mono f32）
    pub sample_rate: u32,
    /// 语言（granularity auto 推断 + decoder 参数；由 start 帧或 default_language 填充）
    pub language: Option<String>,
    pub initial_prompt: Option<String>,
    pub model_id: String,
    pub model_path: PathBuf,
    pub pool_size: usize,
    pub streaming: StreamingConfig,
}

/// 会话级错误（分类超时 / 取消 / 引擎错误）
#[derive(Debug)]
pub enum SessionError {
    Cancelled,
    DecodeTimeout,
    Stt(SttError),
    Join(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "会话已取消"),
            Self::DecodeTimeout => write!(f, "解码超时"),
            Self::Stt(e) => write!(f, "STT 引擎错误: {e}"),
            Self::Join(e) => write!(f, "解码任务 join 失败: {e}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// 解码器抽象（sync：在 spawn_blocking 内调用，便于真实 / mock 替换）
pub trait Decoder: Send + Sync + 'static {
    /// 同步解码 samples → 带时间戳的 segments
    fn decode(&self, samples: &[f32]) -> Result<Vec<SttSegment>, SttError>;
}

/// transcribe-rs Whisper 解码器（真实实现）
pub struct WhisperDecoder {
    pub model_id: String,
    pub model_path: PathBuf,
    pub pool_size: usize,
    pub opts: SttTranscribeOptions,
}

impl Decoder for WhisperDecoder {
    fn decode(&self, samples: &[f32]) -> Result<Vec<SttSegment>, SttError> {
        let key = EngineKey::new(&self.model_id);
        let pool = get_or_init_engine(key, self.model_path.clone(), self.pool_size)?;
        let inst = pool.pick();
        let mut guard = inst.lock().unwrap_or_else(|p| p.into_inner());
        let result = guard.transcribe_with(samples, &self.opts.to_inference_params())?;
        // segments 无条件生成（transcribe-rs 0.3.11）；映射为 SttSegment
        let segs = result
            .segments
            .unwrap_or_default()
            .into_iter()
            .map(|s| SttSegment {
                start: s.start,
                end: s.end,
                text: s.text,
            })
            .collect();
        Ok(segs)
    }
}

/// 流式会话
pub struct StreamingSession<D: Decoder> {
    cfg: SessionConfig,
    decoder: Arc<D>,
    la: LocalAgreement,
    granularity: CompareGranularity,
    buffer: Vec<f32>,
    last_decoded_len: usize,
    /// 最后一次 A 解码（完整 buffer）全文；finish 时作为 Done.committed_total，
    /// 避免增量累积因前文修正导致的重复（Whisper 对完整 buffer 的单次解码最准）。
    last_full_text: String,
    event_tx: mpsc::Sender<StreamEvent>,
    cancel: Arc<AtomicBool>,
}

impl<D: Decoder> StreamingSession<D> {
    pub fn new(
        cfg: SessionConfig,
        decoder: Arc<D>,
        event_tx: mpsc::Sender<StreamEvent>,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        // granularity 一次解析：char/word 强制，auto/其他按 language 推断
        let granularity = CompareGranularity::resolve(
            &cfg.streaming.compare_granularity,
            cfg.language.as_deref(),
        );
        let la_cfg = LaConfig {
            min_agree_count: cfg.streaming.min_agree_count.max(1),
            granularity,
        };
        Self {
            cfg,
            decoder,
            la: LocalAgreement::new(la_cfg),
            granularity,
            buffer: Vec::new(),
            last_decoded_len: 0,
            last_full_text: String::new(),
            event_tx,
            cancel,
        }
    }

    /// 喂入 PCM samples（16k mono f32）。累积 `decode_interval_sec` 新音频后触发双解码。
    pub async fn push_samples(&mut self, samples: &[f32]) -> Result<(), SessionError> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(SessionError::Cancelled);
        }
        self.buffer.extend_from_slice(samples);

        let interval_samples =
            (self.cfg.streaming.decode_interval_sec * self.cfg.sample_rate as f32) as usize;
        if interval_samples > 0
            && self.buffer.len() >= interval_samples
            && self.buffer.len() - self.last_decoded_len >= interval_samples
        {
            self.try_decode().await?;
            self.last_decoded_len = self.buffer.len();
        }
        Ok(())
    }

    /// 双解码 + LA observe + 推事件
    async fn try_decode(&mut self) -> Result<(), SessionError> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(SessionError::Cancelled);
        }
        let a_samples = self.buffer.clone();
        let tail_samples =
            (self.cfg.streaming.tail_trim_sec * self.cfg.sample_rate as f32) as usize;
        let b_len = self.buffer.len().saturating_sub(tail_samples);
        if b_len == 0 {
            return Ok(());
        }
        let b_samples = self.buffer[..b_len].to_vec();

        let timeout = Duration::from_secs(self.cfg.streaming.decode_timeout_sec);
        // A、B 串行解码（简化；并行 join! 可降延迟，但需注意 pool 实例争用）
        let segs_a = decode_once(self.decoder.clone(), a_samples, timeout).await?;
        if self.cancel.load(Ordering::Acquire) {
            return Err(SessionError::Cancelled);
        }
        let segs_b = decode_once(self.decoder.clone(), b_samples, timeout).await?;

        // 记录最后一次完整 buffer（A）解码全文，finish 时作为最终结果（避免增量累积重复）
        self.last_full_text = segs_a
            .iter()
            .map(|s| s.text.trim())
            .collect::<Vec<_>>()
            .join(self.granularity.separator());

        let decision = self.la.observe(&segs_a, &segs_b);
        if !decision.newly_committed.is_empty() {
            let text = join_tokens(&decision.newly_committed, self.granularity);
            let committed = self.la.committed_text();
            self.send(StreamEvent::Committed { text, committed }).await;
        }
        let partial = join_tokens(&decision.partial, self.granularity);
        let committed = self.la.committed_text();
        self.send(StreamEvent::Partial {
            text: partial,
            committed,
        })
        .await;
        Ok(())
    }

    /// 会话结束：flush 全部剩余 + 推 Done
    pub async fn finish(&mut self) -> Result<(), SessionError> {
        let flushed = self.la.flush_remaining();
        // 最终结果优先用最后一次完整 buffer 解码（A），避免增量累积因前文修正导致的重复；
        // 从未解码时 fallback committed_text。
        let committed_total = if self.last_full_text.is_empty() {
            self.la.committed_text()
        } else {
            self.last_full_text.clone()
        };
        if !flushed.is_empty() {
            let text = join_tokens(&flushed, self.granularity);
            self.send(StreamEvent::Committed {
                text,
                committed: committed_total.clone(),
            })
            .await;
        }
        self.send(StreamEvent::Done { committed_total }).await;
        Ok(())
    }

    async fn send(&self, evt: StreamEvent) {
        if let Err(e) = self.event_tx.send(evt).await {
            warn!("streaming event send failed: {e}");
        }
    }
}

/// 单次解码（spawn_blocking + timeout + 取消）
async fn decode_once<D: Decoder>(
    decoder: Arc<D>,
    samples: Vec<f32>,
    timeout: Duration,
) -> Result<Vec<SttSegment>, SessionError> {
    let join = tokio::task::spawn_blocking(move || decoder.decode(&samples));
    match tokio::time::timeout(timeout, join).await {
        Ok(Ok(Ok(segs))) => Ok(segs),
        Ok(Ok(Err(e))) => Err(SessionError::Stt(e)),
        Ok(Err(e)) => Err(SessionError::Join(e.to_string())),
        Err(_) => Err(SessionError::DecodeTimeout),
    }
}

/// 按 granularity 拼接 raw token 为文本（token 间插分隔符）
fn join_tokens(tokens: &[String], granularity: CompareGranularity) -> String {
    let sep = granularity.separator();
    tokens.join(sep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mock 解码器：按调用顺序返回预设脚本（包装成单 segment，end 递增模拟音频推进）
    struct MockDecoder {
        scripts: Vec<String>,
        call_idx: Mutex<usize>,
    }

    impl MockDecoder {
        fn new(scripts: Vec<String>) -> Self {
            Self {
                scripts,
                call_idx: Mutex::new(0),
            }
        }
    }

    impl Decoder for MockDecoder {
        fn decode(&self, _samples: &[f32]) -> Result<Vec<SttSegment>, SttError> {
            let mut idx = self.call_idx.lock().expect("mock idx lock");
            let i = *idx;
            *idx += 1;
            let text = self
                .scripts
                .get(i)
                .cloned()
                .unwrap_or_else(|| self.scripts.last().cloned().unwrap_or_default());
            // end 固定 1.0（测试只验证 commit/done 流程；LA 两次 observe 同输入即可 commit）
            Ok(vec![SttSegment {
                start: 0.0,
                end: 1.0,
                text,
            }])
        }
    }

    fn test_cfg() -> SessionConfig {
        SessionConfig {
            sample_rate: 16000,
            language: None,
            initial_prompt: None,
            model_id: "tiny".to_string(),
            model_path: PathBuf::from("/dummy"),
            pool_size: 1,
            streaming: StreamingConfig::default(),
        }
    }

    #[tokio::test]
    async fn test_streaming_session_commit_and_done() {
        // decode_interval=0.5s → 8000 samples 触发；每次 try_decode 调 A、B 两次 decode
        // 两次 push 触发两次 try_decode → 4 次 decode，全返 "hello"
        // LA：第 1 次 streak=1，第 2 次 streak=2 → commit "hello"
        let decoder = Arc::new(MockDecoder::new(vec![
            "hello".to_string(),
            "hello".to_string(),
            "hello".to_string(),
            "hello".to_string(),
        ]));
        let (tx, mut rx) = mpsc::channel(32);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut session = StreamingSession::new(test_cfg(), decoder, tx, cancel);

        let chunk = vec![0.0f32; 8000];
        session.push_samples(&chunk).await.unwrap();
        session.push_samples(&chunk).await.unwrap();
        session.finish().await.unwrap();

        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            let is_done = matches!(e, StreamEvent::Done { .. });
            events.push(e);
            if is_done {
                break;
            }
        }
        let has_committed = events
            .iter()
            .any(|e| matches!(e, StreamEvent::Committed { text, .. } if text.trim() == "hello"));
        let committed_total = events.iter().find_map(|e| match e {
            StreamEvent::Done { committed_total } => Some(committed_total.clone()),
            _ => None,
        });
        assert!(has_committed, "应有 Committed(text=hello): {:?}", events);
        assert_eq!(committed_total.as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn test_cancel_aborts() {
        let decoder = Arc::new(MockDecoder::new(vec!["x".to_string()]));
        let (tx, _rx) = mpsc::channel(32);
        let cancel = Arc::new(AtomicBool::new(true)); // 已取消
        let mut session = StreamingSession::new(test_cfg(), decoder, tx, cancel);
        let err = session.push_samples(&[0.0f32; 100]).await.unwrap_err();
        assert!(matches!(err, SessionError::Cancelled));
    }
}
