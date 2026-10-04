//! 无头自检：`ai-toolbox --smoke`（voice-cli 链路）/ `--smoke-dp`（document-parser 链路）。
//! 无 GUI，线性走完 provision → spawn → 健康检查 → （dp：parse-sync 试跑）→ 停止。
//! 托管逻辑与 GUI 共用同一批 services 层函数。

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::services::manager::DEFAULT_STT_TIER;
use crate::services::paths;
use crate::services::provision;
use crate::services::supervisor;

pub fn run() -> Result<()> {
    // 1. provision voice-cli（small 档）
    println!("[smoke] provision voice-cli (tier={DEFAULT_STT_TIER}) ...");
    let last_pct = std::cell::Cell::new(u8::MAX);
    provision::provision_voice_cli(DEFAULT_STT_TIER, &|p| {
        if let Some(t) = p.total.filter(|t| *t > 0) {
            let pct = ((p.done as f64 / t as f64) * 10.0) as u8 * 10;
            if pct != last_pct.get() {
                last_pct.set(pct);
                println!("  {}  {pct}%", p.detail);
            }
        }
    })
    .context("provision voice-cli")?;

    // 2. spawn + 健康等待
    let install_dir: PathBuf = paths::service_install_dir(paths::VOICE_CLI);
    let port = supervisor::pick_free_port().context("pick port")?;
    let config = supervisor::VoiceCliRuntimeConfig::write(&install_dir, port, DEFAULT_STT_TIER)
        .context("write runtime config")?;
    let bin = install_dir.join(deploy_installer::binary_name(paths::VOICE_CLI));
    let argv = supervisor::voice_cli_argv(&bin, &config);
    let pid = supervisor::spawn_service(paths::VOICE_CLI, &argv, &install_dir)
        .context("spawn voice-cli")?;
    println!("[smoke] spawned pid={pid} port={port}");

    let mut ok = false;
    for _ in 0..45 {
        std::thread::sleep(Duration::from_secs(2));
        if supervisor::health_ok(port, "/health") {
            ok = true;
            break;
        }
    }
    if !ok {
        supervisor::stop_service(paths::VOICE_CLI).ok();
        bail!("voice-cli 90s 内未就绪（/health 未通过）");
    }
    let health = supervisor::fetch_health_data(port).context("fetch health")?;
    println!(
        "[smoke] healthy: version={} models_loaded={}",
        health
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("?"),
        health
            .get("models_loaded")
            .and_then(|m| m.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    );

    // 3. 停止（进程组）
    supervisor::stop_service(paths::VOICE_CLI).context("stop voice-cli")?;
    std::thread::sleep(Duration::from_secs(1));
    let alive = supervisor::health_ok(port, "/health");
    if alive {
        bail!("停止后 /health 仍可达——进程组终止未生效");
    }
    println!("[smoke] stopped cleanly ✓");
    println!("[smoke] ALL OK");
    Ok(())
}

/// document-parser 链路自检：provision（mac 预编译 venv）→ spawn → /ready → parse-sync 试跑 → 停止。
pub fn run_dp(pdf: Option<&std::path::Path>) -> Result<()> {
    println!("[smoke-dp] provision document-parser ...");
    let last_pct = std::cell::Cell::new(u8::MAX);
    provision::provision_document_parser(&|p| {
        if let Some(t) = p.total.filter(|t| *t > 0) {
            let pct = ((p.done as f64 / t as f64) * 10.0) as u8 * 10;
            if pct != last_pct.get() {
                last_pct.set(pct);
                println!("  {}  {pct}%", p.detail);
            }
        }
    })
    .context("provision document-parser")?;

    let install_dir: PathBuf = paths::service_install_dir(paths::DOCUMENT_PARSER);
    let port = supervisor::pick_free_port().context("pick port")?;
    let config = supervisor::write_dp_runtime_config(&install_dir, port)
        .context("write dp runtime config")?;
    let bin = install_dir.join(deploy_installer::binary_name(paths::DOCUMENT_PARSER));
    let argv = supervisor::document_parser_argv(&bin, &config);
    let pid = supervisor::spawn_service(paths::DOCUMENT_PARSER, &argv, &install_dir)
        .context("spawn document-parser")?;
    println!("[smoke-dp] spawned pid={pid} port={port}，等待就绪（首启可能建 venv，较久）...");

    let mut ready = false;
    for _ in 0..(15 * 30) {
        std::thread::sleep(Duration::from_secs(2));
        if supervisor::health_ok(port, "/ready") {
            ready = true;
            break;
        }
    }
    if !ready {
        supervisor::stop_service(paths::DOCUMENT_PARSER).ok();
        bail!("document-parser 30 分钟内未就绪（/ready 未通过）");
    }
    println!("[smoke-dp] ready ✓");

    if let Some(pdf) = pdf {
        println!("[smoke-dp] parse-sync: {}", pdf.display());
        let r = crate::api::document::parse_sync(port, pdf).context("parse-sync")?;
        println!(
            "[smoke-dp] parsed: {} 引擎 · {}ms · {} 字 · markdown 前 120 字：{}",
            r.engine.as_deref().unwrap_or("?"),
            r.processing_time_ms.unwrap_or(0),
            r.word_count.unwrap_or(0),
            r.markdown_content.chars().take(120).collect::<String>()
        );
    } else {
        println!("[smoke-dp] （未提供 PDF，跳过 parse-sync 试跑）");
    }

    supervisor::stop_service(paths::DOCUMENT_PARSER).context("stop document-parser")?;
    println!("[smoke-dp] ALL OK");
    Ok(())
}
