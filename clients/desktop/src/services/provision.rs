//! 资源供给：把服务装进客户端 appdata（复用 deploy-installer 的 setup 管线），
//! 并补齐安装器不管的部分（按档 whisper 模型，hf-mirror 优先）。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use deploy_installer::cli::assets::patch_whisper_default_model;
use deploy_installer::cli::document_parser;
use deploy_installer::cli::voice_cli;
use deploy_installer::cli::{SetupArgs, VoiceCliSetupArgs};

use super::paths;

/// 进度事件（detail 描述阶段，done/total 字节级）。
#[derive(Clone, Debug)]
pub struct Progress {
    pub detail: String,
    pub done: u64,
    pub total: Option<u64>,
}

/// whisper ggml 模型源：镜像优先（国内可用），官方回退。
/// 服务自身 `model download` CLI 硬编码 HF 直链无镜像参数，故下载由客户端自己做。
const WHISPER_HOSTS: &[&str] = &["https://hf-mirror.com", "https://huggingface.co"];

/// 安装 voice-cli：setup（二进制/模板/ffmpeg）+ default_model 改为所选档 + 按档下模型。
/// 返回安装目录。
pub fn provision_voice_cli(model_tier: &str, progress: &dyn Fn(Progress)) -> Result<PathBuf> {
    let install_dir = paths::service_install_dir(paths::VOICE_CLI);

    let args = VoiceCliSetupArgs {
        install_dir: Some(install_dir.clone()),
        // 模型不走安装器（OSS 只有 large-v3/all 两档包；按档下载是客户端自己的事）
        use_prebuilt_models: false,
        skip_models: true,
        models: "large-v3".into(),
        oss_base: None,
        // 客户端托管形态固定 CPU/本平台 vendor 二进制（mac=Metal 内建；不碰 CUDA/Vulkan bundle）
        use_oss_cuda: false,
        skip_oss_cuda: true,
        use_oss_vulkan: false,
        skip_oss_vulkan: true,
        cuda_lib_dir: None,
        cudnn_lib_dir: None,
    };
    progress(Progress {
        detail: "安装 voice-cli（二进制/模板/ffmpeg）".into(),
        done: 0,
        total: None,
    });
    voice_cli::setup(&args, true, false)
        .with_context(|| format!("voice-cli setup → {}", install_dir.display()))?;

    // setup 把 default_model 钉成 large-v3；覆盖为用户所选档
    patch_whisper_default_model(&install_dir.join("config.yml"), model_tier)?;

    let models_dir = install_dir.join("models");
    ensure_whisper_model(&models_dir, model_tier, progress)?;
    Ok(install_dir)
}

/// 安装 document-parser：setup（二进制/模板/venv/mineru 模型）。mac 默认走 OSS 预编译 venv。
pub fn provision_document_parser(progress: &dyn Fn(Progress)) -> Result<PathBuf> {
    let install_dir = paths::service_install_dir(paths::DOCUMENT_PARSER);
    let args = SetupArgs {
        install_dir: Some(install_dir.clone()),
        use_prebuilt_venv: false,
        no_prebuilt_venv: false,
        venv_file: None,
        skip_models: false,
        oss_base: None,
    };
    progress(Progress {
        detail: "安装 document-parser（二进制/venv/模型）".into(),
        done: 0,
        total: None,
    });
    document_parser::setup(&args, true, false)
        .with_context(|| format!("document-parser setup → {}", install_dir.display()))?;
    Ok(install_dir)
}

/// 按档放好 ggml-<tier>.bin（幂等：在场即跳过）；镜像失败换官方源再试。
pub fn ensure_whisper_model(
    models_dir: &Path,
    tier: &str,
    progress: &dyn Fn(Progress),
) -> Result<()> {
    let dest = models_dir.join(format!("ggml-{tier}.bin"));
    if dest.exists() {
        return Ok(());
    }
    let mut last: Option<String> = None;
    for host in WHISPER_HOSTS {
        let url = format!("{host}/ggerganov/whisper.cpp/resolve/main/ggml-{tier}.bin");
        match download_to_file(&url, &dest, &|done, total| {
            progress(Progress {
                detail: format!("下载 {tier} 模型"),
                done,
                total,
            });
        }) {
            Ok(()) => return Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(part_path(&dest));
                last = Some(format!("{url}: {e:#}"));
            }
        }
    }
    bail!(
        "ggml-{tier}.bin 全部下载源失败：{}",
        last.unwrap_or_default()
    )
}

/// 下载到 .part 再原子改名；Content-Length 有则逐块回调进度。
/// 不设整体超时（大文件会被整段掐掉），只设连接超时——P0 取消手段是关应用。
fn download_to_file(url: &str, dest: &Path, progress: &dyn Fn(u64, Option<u64>)) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()
        .context("build download client")?;
    let mut resp = client.get(url).send().and_then(|r| r.error_for_status())?;
    let total = resp.content_length();
    let part = part_path(dest);
    if let Some(parent) = part.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut file =
        std::fs::File::create(&part).with_context(|| format!("create {}", part.display()))?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut done: u64 = 0;
    loop {
        let n = resp.read(&mut buf).context("read chunk")?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n])?;
        done += n as u64;
        progress(done, total);
    }
    file.sync_all().ok();
    std::fs::rename(&part, dest)
        .with_context(|| format!("rename {} → {}", part.display(), dest.display()))?;
    Ok(())
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}
