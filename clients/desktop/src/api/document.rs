//! document-parser API：/parse-sync（multipart 仅 file；无 upload_* 参数→纯本地解析）。

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::voice::unwrap_envelope;

#[derive(Deserialize, Debug, Clone)]
pub struct SyncParseResponse {
    pub markdown_content: String,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub processing_time_ms: Option<u64>,
    #[serde(default)]
    pub word_count: Option<u64>,
    #[serde(default)]
    pub filename: Option<String>,
}

/// POST /api/v1/documents/parse-sync。阻塞调用——smol::unblock 包。
/// 服务端语义：408=超时（10 分钟）/ 413=超限（500MB）/ 信号量排队；均以 HttpResult 返回。
pub fn parse_sync(port: u16, file: &Path) -> Result<SyncParseResponse> {
    let file_name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".into());
    let bytes = std::fs::read(file).with_context(|| format!("read {}", file.display()))?;
    let part = reqwest::blocking::multipart::Part::bytes(bytes).file_name(file_name);
    let form = reqwest::blocking::multipart::Form::new().part("file", part);

    let resp = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        // 服务端 parse-sync 自身 10 分钟上限——客户端略放宽
        .timeout(Duration::from_secs(11 * 60))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
        .post(format!(
            "http://127.0.0.1:{port}/api/v1/documents/parse-sync"
        ))
        .multipart(form)
        .send()
        .context("POST parse-sync")?;
    let status = resp.status();
    let body = resp.text().context("read response body")?;
    if !status.is_success() {
        let detail =
            serde_json::from_str::<super::voice::HttpEnvelope<serde::de::IgnoredAny>>(&body)
                .map(|e| e.message)
                .unwrap_or_else(|_| body.clone());
        bail!("HTTP {status}：{detail}");
    }
    unwrap_envelope(&body)
}
