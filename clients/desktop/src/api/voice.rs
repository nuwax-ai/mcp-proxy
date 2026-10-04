//! voice-cli API：统一 HttpResult 信封解包 + /transcribe（multipart）。

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// 两服务统一响应信封：code=="0000" 成功，否则取 message 报错。
#[derive(Deserialize, Debug)]
pub struct HttpEnvelope<T> {
    pub code: String,
    pub message: String,
    pub data: Option<T>,
}

pub fn unwrap_envelope<T>(body: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let env: HttpEnvelope<T> = serde_json::from_str(body).context("parse response json")?;
    if env.code != "0000" {
        bail!("服务错误 {}: {}", env.code, env.message);
    }
    env.data
        .ok_or_else(|| anyhow::anyhow!("响应缺少 data 字段"))
}

#[derive(Deserialize, Debug, Clone)]
pub struct TranscriptionResponse {
    pub text: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub duration: Option<f32>,
    #[serde(default)]
    pub processing_time: Option<f32>,
}

/// 引擎固定 5 枚举（服务端 stt_common.rs 的大小写不敏感解析对齐）。
pub const ENGINES: &[&str] = &[
    "whisper",
    "fireredasr2",
    "sensevoice",
    "funasrnano",
    "qwen3asr",
];

/// POST /transcribe：multipart `file` + `engine`。阻塞调用——用 smol::unblock 包。
pub fn transcribe(port: u16, file: &Path, engine: &str) -> Result<TranscriptionResponse> {
    let file_name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "audio".into());
    let bytes = std::fs::read(file).with_context(|| format!("read {}", file.display()))?;
    let part = reqwest::blocking::multipart::Part::bytes(bytes).file_name(file_name);
    let form = reqwest::blocking::multipart::Form::new()
        .part("file", part)
        .text("engine", engine.to_string());

    let resp = client()
        .post(format!("http://127.0.0.1:{port}/transcribe"))
        .multipart(form)
        .send()
        .context("POST /transcribe")?;
    let status = resp.status();
    let body = resp.text().context("read response body")?;
    if !status.is_success() {
        // 错误体也是 HttpResult JSON——尽量解出 message
        if let Ok(msg) = serde_json::from_str::<HttpEnvelope<serde::de::IgnoredAny>>(&body) {
            bail!("HTTP {status}：{}", msg.message);
        }
        bail!("HTTP {status}：{body}");
    }
    unwrap_envelope(&body)
}

fn client() -> reqwest::blocking::Client {
    // 转写可能长音频大文件——整体超时放宽到 30 分钟
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30 * 60))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}
