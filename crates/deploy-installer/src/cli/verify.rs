//! `deploy-installer <service> verify`：部署后自验（终端用户的本地冒烟，
//! 不依赖仓库/开发工具）。逐项 ✓/✗/WARN，任一 ✗ 以非零码退出。
//!
//! 全部走 curl 子进程（安装器既有惯例，零 HTTP 库依赖；curl 是 doctor 的
//! 必需命令）。HTTP 判定标准与 test-e2e 对齐：code=0000。

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, bail};

use crate::cli::common::{CONFIG_FILENAME, read_server_port};

fn null_sink() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

/// GET 期望 2xx（curl -fsS 静默丢弃 body）
fn curl_ok(url: &str, timeout_secs: u64) -> bool {
    Command::new("curl")
        .args([
            "-fsS",
            "-m",
            &timeout_secs.to_string(),
            "-o",
            null_sink(),
            url,
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// GET 拿 body（失败/非 2xx 返回 None）
fn curl_body(url: &str, timeout_secs: u64) -> Option<String> {
    let out = Command::new("curl")
        .args(["-fsS", "-m", &timeout_secs.to_string(), url])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// POST multipart 文件，拿响应 body
fn curl_post_file(
    url: &str,
    file: &Path,
    form_name: &str,
    extra: &[(&str, &str)],
) -> Option<String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsS", "-m", "300", "-X", "POST", url]);
    cmd.arg("-F")
        .arg(format!("{form_name}=@{}", file.display()));
    for (k, v) in extra {
        cmd.arg("-F").arg(format!("{k}={v}"));
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

fn code_ok(body: &str, label: &str) -> bool {
    let ok = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("code").and_then(|c| c.as_str()).map(|c| c == "0000"))
        .unwrap_or(false);
    if !ok {
        println!(
            "    response: {}",
            body.chars().take(200).collect::<String>()
        );
    }
    let _ = label;
    ok
}

/// 写一个最小合法 WAV（16k mono s16le 正弦波）到临时文件
fn write_sine_wav(path: &Path, seconds: f32) -> std::io::Result<()> {
    let sample_rate = 16000u32;
    let n = (sample_rate as f32 * seconds) as usize;
    let mut pcm = Vec::with_capacity(n * 2);
    for i in 0..n {
        let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sample_rate as f32).sin();
        let s = (v * 6000.0) as i16;
        pcm.extend_from_slice(&s.to_le_bytes());
    }
    let mut w = Vec::with_capacity(44 + pcm.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + pcm.len()) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    w.extend_from_slice(&pcm);
    std::fs::write(path, w)
}

fn resolve_port(install_dir: &Path, default_port: u16) -> u16 {
    read_server_port(&install_dir.join(CONFIG_FILENAME)).unwrap_or(default_port)
}

fn json_paths_nonempty(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("paths")
                .and_then(|p| p.as_object())
                .map(|o| !o.is_empty())
        })
        .unwrap_or(false)
}

fn mineru_models_present() -> bool {
    crate::home_dir()
        .map(|h| {
            h.join(".cache/modelscope/models/OpenDataLab--PDF-Extract-Kit-1.0/snapshots")
                .exists()
        })
        .unwrap_or(false)
}

/// `deploy-installer document-parser verify`
pub fn verify_document_parser(install_dir: Option<PathBuf>) -> Result<()> {
    let dir = install_dir.unwrap_or_else(crate::default_document_parser_install_dir);
    let port = resolve_port(&dir, 8087);
    let base = format!("http://127.0.0.1:{port}");
    println!(
        "==> document-parser verify → {base} (dir: {})",
        dir.display()
    );
    let mut failures = 0u32;

    // 1. /health
    if curl_ok(&format!("{base}/health"), 5) {
        println!("  health:        OK");
    } else {
        println!("  health:        FAIL (service not reachable on {base})");
        bail!(
            "document-parser not reachable — run `deploy-installer document-parser service status`"
        );
    }

    // 2. /ready
    if curl_ok(&format!("{base}/ready"), 5) {
        println!("  ready:         OK");
    } else {
        println!("  ready:         FAIL");
        failures += 1;
    }

    // 3. OpenAPI 文档（scalar 隐含 openapi.json；spec 路径非空即文档完整）
    match curl_body(&format!("{base}/api/docs/openapi.json"), 10) {
        Some(body) if json_paths_nonempty(&body) => println!("  api docs:      OK (openapi.json)"),
        _ => {
            println!("  api docs:      FAIL (openapi.json missing/invalid)");
            failures += 1;
        }
    }

    // 4. parse-sync 冒烟（MarkItDown 链路）
    let tmp = std::env::temp_dir().join(format!("dp-verify-{}.md", std::process::id()));
    std::fs::write(
        &tmp,
        "# deploy-installer verify\n\nMarkdown parse smoke test with sufficient content length.\n\n- item one\n- item two\n",
    )?;
    let parse_result = curl_post_file(
        &format!("{base}/api/v1/documents/parse-sync"),
        &tmp,
        "file",
        &[],
    );
    let _ = std::fs::remove_file(&tmp);
    match parse_result {
        Some(body) if code_ok(&body, "parse-sync") => println!("  parse-sync:    OK (markdown)"),
        _ => {
            println!("  parse-sync:    FAIL");
            failures += 1;
        }
    }

    // 5. MinerU 模型（PDF 就绪性提示，WARN 级）
    if mineru_models_present() {
        println!("  mineru models: OK (PDF 解析即装即用)");
    } else {
        println!(
            "  mineru models: WARN (未下载 — 首跑 PDF 将从 ModelScope 拉模型，慢网络建议重跑 install)"
        );
    }

    finish("document-parser", failures)
}

/// `deploy-installer voice-cli verify`
pub fn verify_voice_cli(install_dir: Option<PathBuf>) -> Result<()> {
    let dir = install_dir.unwrap_or_else(crate::default_voice_cli_install_dir);
    let port = resolve_port(&dir, 8077);
    let base = format!("http://127.0.0.1:{port}");
    println!("==> voice-cli verify → {base} (dir: {})", dir.display());
    let mut failures = 0u32;

    // 1. /health（自报版本）
    let health = curl_body(&format!("{base}/health"), 5);
    let version = health.as_deref().and_then(|b| {
        serde_json::from_str::<serde_json::Value>(b)
            .ok()
            .and_then(|v| {
                v.pointer("/data/version")
                    .and_then(|x| x.as_str())
                    .map(String::from)
            })
    });
    match &version {
        Some(v) => println!("  health:        OK (version {v})"),
        None => {
            println!("  health:        FAIL (service not reachable on {base})");
            bail!("voice-cli not reachable — run `deploy-installer voice-cli service status`");
        }
    }

    // 2. OpenAPI 文档
    match curl_body(&format!("{base}/api/docs/openapi.json"), 10) {
        Some(body) if json_paths_nonempty(&body) => println!("  api docs:      OK (openapi.json)"),
        _ => {
            println!("  api docs:      FAIL (openapi.json missing/invalid)");
            failures += 1;
        }
    }

    // 3. whisper 模型在场 → 转写冒烟；缺失 → WARN 跳过。
    // 模型选择：config 的 whisper.default_model（安装器 patch 为 large-v3）对应的
    // ggml 文件在场则用之；否则回退任一在场 ggml-*.bin（vulkan/cuda 档机器可能
    // 只放了 base——default 与实际不符时不误报）
    let models_dir = dir.join("models");
    let present_models: Vec<String> = std::fs::read_dir(&models_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    n.strip_prefix("ggml-")
                        .and_then(|s| s.strip_suffix(".bin"))
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();
    let Some(model) = read_whisper_default_model(&dir.join(CONFIG_FILENAME))
        .filter(|m| present_models.contains(m))
        .or_else(|| present_models.first().cloned())
    else {
        println!("  transcribe:    WARN (no models/ggml-*.bin — 重跑 install 下载，或手动放置)");
        return finish("voice-cli", failures);
    };

    let wav = std::env::temp_dir().join(format!("vc-verify-{}.wav", std::process::id()));
    write_sine_wav(&wav, 1.0)?;
    let body = curl_post_file(
        &format!("{base}/transcribe"),
        &wav,
        "file",
        &[("model", model.as_str())],
    );
    let _ = std::fs::remove_file(&wav);
    match body {
        Some(b) if code_ok(&b, "transcribe") => {
            println!("  transcribe:    OK (model={model}, STT 链路完整)")
        }
        _ => {
            println!("  transcribe:    FAIL");
            failures += 1;
        }
    }

    finish("voice-cli", failures)
}

/// 读 config.yml 的 `whisper.default_model`（安装器 patch 为 large-v3；
/// 简单行扫描，照 read_server_port 模式）
fn read_whisper_default_model(config_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(config_path).ok()?;
    let mut in_whisper = false;
    for line in content.lines() {
        let t = line.trim();
        if !line.starts_with(' ') && !line.starts_with('\t') && !t.is_empty() && !t.starts_with('#')
        {
            in_whisper = t.starts_with("whisper:");
            continue;
        }
        if in_whisper && let Some(rest) = t.strip_prefix("default_model:") {
            return Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

fn finish(service: &str, failures: u32) -> Result<()> {
    if failures == 0 {
        println!("\n✅ {service} deployment verified");
        Ok(())
    } else {
        println!("\n❌ {service} verify: {failures} check(s) failed (see above)");
        bail!("{service} verify failed");
    }
}
