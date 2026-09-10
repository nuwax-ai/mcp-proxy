//! voice-cli 流式 WebSocket 客户端（tokio-tungstenite）。
//!
//! 协议见 voice-cli docs/API.md §4（STT）/ §8（TTS）：
//! - STT：发 `{type:"start",...}` → 收 `ready` → 发二进制 PCM 分片 → `{type:"stop"}`
//!   → 收 `partial*` / `committed*` → `done`（最终文本 `committed_total`）
//! - TTS：发 `{type:"start",text,...}` → `ready` → 二进制 PCM 增量帧 → `done`

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

/// 收到的一条服务端事件（文本 JSON 或二进制帧）
#[derive(Debug)]
pub enum ServerFrame {
    Text(serde_json::Value),
    Binary(Vec<u8>),
}

/// 收集到 done/error 或关闭/超时为止的会话结果
#[derive(Debug)]
pub struct SttSessionResult {
    /// 按到达顺序的文本事件 type 字段（ready/partial/committed/done/error）
    pub event_sequence: Vec<String>,
    /// done.committed_total（最终转写文本；正弦波输入可为空或幻听文本）
    pub committed_total: Option<String>,
    /// error.message（若有）
    pub error: Option<String>,
    /// 服务端发来的二进制帧总字节数（诊断用；STT 正常为 0）
    pub binary_bytes: usize,
}

/// 跑一轮完整 STT 流式会话：start → PCM 分片（interval 节拍）→ stop → 等 done
pub async fn stt_session(
    ws_base: &str,
    model: Option<&str>,
    pcm: &[u8],
) -> anyhow::Result<SttSessionResult> {
    let url = format!("{ws_base}/api/v1/stream/transcribe");
    let (mut ws, _resp) = tokio_tungstenite::connect_async(url).await?;

    let mut start = serde_json::json!({"type": "start", "sample_rate": 16000, "language": "zh"});
    if let Some(m) = model {
        start["model"] = serde_json::json!(m);
    }
    ws.send(Message::Text(start.to_string().into())).await?;

    let mut events: Vec<serde_json::Value> = Vec::new();
    let mut binary_bytes = 0usize;
    let mut sent_stop = false;

    let deadline = Duration::from_secs(120);
    let chunk = 3200usize; // 100ms
    let mut offset = 0usize;

    loop {
        let next = match tokio::time::timeout(deadline, ws.next()).await {
            Ok(Some(msg)) => msg,
            Ok(None) => break, // 服务端关闭
            Err(_) => anyhow::bail!("等待服务端事件超时（{deadline:?}）"),
        }?;
        match next {
            Message::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(t.as_str())?;
                let evt_type = v
                    .get("type")
                    .and_then(|x| x.as_str())
                    .unwrap_or("?")
                    .to_string();
                // ready 后开始按节拍送 PCM
                if evt_type == "ready" {
                    loop {
                        if offset >= pcm.len() {
                            if !sent_stop {
                                sent_stop = true;
                                ws.send(Message::Text(r#"{"type":"stop"}"#.into())).await?;
                            }
                            break;
                        }
                        let end = (offset + chunk).min(pcm.len());
                        ws.send(Message::Binary(pcm[offset..end].to_vec().into()))
                            .await?;
                        offset = end;
                        tokio::time::sleep(Duration::from_millis(30)).await;
                    }
                }
                let is_done = evt_type == "done";
                let is_error = evt_type == "error";
                events.push(v);
                if is_done || is_error {
                    break;
                }
            }
            Message::Binary(b) => binary_bytes += b.len(),
            Message::Close(_) => break,
            _ => {}
        }
    }

    let sequence: Vec<String> = events
        .iter()
        .map(|v| {
            v.get("type")
                .and_then(|x| x.as_str())
                .unwrap_or("?")
                .to_string()
        })
        .collect();
    let done = events
        .iter()
        .find(|v| v.get("type").and_then(|t| t.as_str()) == Some("done"));
    let error = events
        .iter()
        .find(|v| v.get("type").and_then(|t| t.as_str()) == Some("error"));
    Ok(SttSessionResult {
        event_sequence: sequence,
        committed_total: done
            .and_then(|d| d.get("committed_total"))
            .and_then(|t| t.as_str())
            .map(String::from),
        error: error
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .map(String::from),
        binary_bytes,
    })
}

/// TTS 协议探测：发 start（短文本）→ 收首个事件（ready/error）即返回。
/// 返回 (事件类型, 消息/采样率)；disabled 部署预期 ("error", Some("TTS service is disabled"))
pub async fn tts_protocol_probe(
    ws_base: &str,
    text: &str,
) -> anyhow::Result<(String, Option<String>)> {
    let url = format!("{ws_base}/api/v1/stream/tts");
    let (mut ws, _resp) = tokio_tungstenite::connect_async(url).await?;
    ws.send(Message::Text(
        serde_json::json!({"type": "start", "text": text})
            .to_string()
            .into(),
    ))
    .await?;
    loop {
        let next = match tokio::time::timeout(Duration::from_secs(30), ws.next()).await {
            Ok(Some(msg)) => msg,
            Ok(None) => anyhow::bail!("closed before any event"),
            Err(_) => anyhow::bail!("等待 TTS 首事件超时"),
        }?;
        match next {
            Message::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(t.as_str())?;
                let evt = v
                    .get("type")
                    .and_then(|x| x.as_str())
                    .unwrap_or("?")
                    .to_string();
                let msg = v
                    .get("message")
                    .or_else(|| v.get("sample_rate"))
                    .and_then(|m| {
                        m.as_str()
                            .map(String::from)
                            .or_else(|| m.as_u64().map(|n| n.to_string()))
                    });
                return Ok((evt, msg));
            }
            Message::Close(_) => anyhow::bail!("closed before any event"),
            _ => {}
        }
    }
}
