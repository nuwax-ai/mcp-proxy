//! 配置读取（环境变量 > 仓库根 .env.local > 默认值）与服务健康门控。

use std::time::Duration;

/// 仓库根 `.env.local`（gitignore）——远程目标机器的 IP:端口写这里，
/// 模板见 `.env.local.example`。加载一次（first-wins 不覆盖已设环境变量）。
fn load_env_local_once() {
    std::sync::OnceLock::<()>::new().get_or_init(|| {
        // 仓库根 = 本 crate 上两级（crates/test-e2e/）
        if let Ok(root) = std::env::var("CARGO_MANIFEST_DIR") {
            let path = std::path::Path::new(&root).join("../..").join(".env.local");
            // from_path 不覆盖已设的环境变量（env > .env.local）；文件不存在静默忽略
            let _ = dotenvy::from_path(path);
        }
    });
}

fn read_env(key: &str, default: &str) -> String {
    load_env_local_once();
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// document-parser 服务基址（默认本机 8087）
pub fn document_parser_url() -> String {
    read_env("E2E_DOCUMENT_PARSER_URL", "http://127.0.0.1:8087")
        .trim_end_matches('/')
        .to_string()
}

/// voice-cli 服务基址（默认本机 8077）
pub fn voice_cli_url() -> String {
    read_env("E2E_VOICE_CLI_URL", "http://127.0.0.1:8077")
        .trim_end_matches('/')
        .to_string()
}

/// WS 基址：http(s):// → ws(s)://
pub fn http_to_ws(base: &str) -> String {
    base.replacen("http://", "ws://", 1)
        .replacen("https://", "wss://", 1)
}

/// voice-cli 转写用的 whisper 模型（默认 base——小机器友好；131 装的是 base）
pub fn voice_model() -> String {
    read_env("E2E_VOICE_MODEL", "base")
}

/// uploadFromUrl 场景的公网测试文件 URL（未配置则该场景 SKIP）
pub fn asset_url() -> Option<String> {
    load_env_local_once();
    std::env::var("E2E_ASSET_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// TTS 全链路开关（默认关——多数部署 TTS disabled，只测协议行为）
pub fn tts_full_flow_enabled() -> bool {
    read_env("E2E_TTS", "0") == "1"
}

/// 服务门控结果
pub enum Gate {
    /// 服务可达，返回预构建的 HTTP 客户端与基址
    Ready {
        client: reqwest::Client,
        base: String,
    },
    /// 不可达——测试打印 SKIP 并 return（非失败）
    Unreachable { base: String },
}

/// 探测 `/health`（2s 超时）：不可达 → SKIP 语义。
/// 每个测试入口先 `let Gate::Ready { client, base } = probe_or_skip(...) else { return; }`
pub async fn probe_or_skip(base: String) -> Gate {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("build reqwest client");
    let ok = client
        .get(format!("{base}/health"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    if ok {
        Gate::Ready { client, base }
    } else {
        eprintln!("SKIP: service unreachable at {base} (set E2E_*_URL or start the service)");
        Gate::Unreachable { base }
    }
}

/// 统一的响应包装断言辅助：取 JSON `code` 字段（document-parser/voice-cli 惯例 "0000"=成功）
pub fn assert_ok_code(label: &str, body: &serde_json::Value) {
    assert_eq!(
        body.get("code").and_then(|c| c.as_str()),
        Some("0000"),
        "{label}: 期望 code=0000，实际 {body}"
    );
}
