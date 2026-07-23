use anyhow::{Context, Result};
use flate2;
use futures::StreamExt;
use once_cell::sync::Lazy;
use std::path::Path;
use tar;
use tokio::io::AsyncWriteExt;

use super::catalog::dir_has_onnx;

// ────────────────────────────────────────────────────────────────────────────
// 从 URL 下载模型 bundle（.tar.gz）
// ────────────────────────────────────────────────────────────────────────────

/// 从 HTTP(S) URL 下载模型 tar.gz 包并解压到缓存目录。
///
/// 模型包要求：tar.gz 内顶层为 `models--<ns>--<repo>/` 目录结构
///（与 hf-hub 缓存格式一致），解压后 fastembed 初始化时发现文件已存在即跳过 HF 下载。
///
/// # 参数
/// * `url` - 模型包下载地址（http/https）
/// * `cache_dir` - 缓存根目录（如 `.fastembed_cache`）
pub async fn download_model_from_url(url: &str, cache_dir: &Path) -> Result<()> {
    let url_trimmed = url.trim();
    if url_trimmed.is_empty() {
        anyhow::bail!("模型 URL 不能为空");
    }

    let start = std::time::Instant::now();

    // 确保缓存目录存在
    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("无法创建缓存目录: {}", cache_dir.display()))?;

    // 下载到临时文件
    let tmp_dir = tempfile::tempdir().context("无法创建临时目录")?;
    let tmp_file = tmp_dir.path().join("model.tar.gz");

    tracing::info!("⬇️ 从 {} 下载模型包...", url_trimmed);
    download_file(url_trimmed, &tmp_file).await?;
    let dl_elapsed = start.elapsed();
    let dl_size = std::fs::metadata(&tmp_file).map(|m| m.len()).unwrap_or(0);
    tracing::info!(
        "下载完成: {} ({:.1} MB), 耗时 {:?}",
        tmp_file.display(),
        dl_size as f64 / (1024.0 * 1024.0),
        dl_elapsed
    );

    // 解压到缓存目录
    tracing::info!("📦 解压模型包到 {} ...", cache_dir.display());
    extract_tar_gz(&tmp_file, cache_dir)?;
    let extract_elapsed = start.elapsed() - dl_elapsed;
    tracing::info!("解压完成, 耗时 {:?}", extract_elapsed);

    // 验证：检查缓存目录下是否存在 .onnx 文件
    let onnx_found = dir_has_onnx(cache_dir);
    if !onnx_found {
        // 不做 hard error：模型包可能用子目录结构（snapshots/<hash>/），
        // 上游 dir_has_onnx 会递归一层，只要模型包结构正确即可匹配。
        // 如果仍没找到，只 warn — 让后续 fastembed 初始化时自行报错。
        tracing::warn!(
            "模型包解压后未在 {} 下直接找到 .onnx 文件，请确认包结构与 hf-hub 缓存格式一致",
            cache_dir.display()
        );
    }

    let total = start.elapsed();
    tracing::info!("✅ 模型包拉取完成，总耗时 {:?}", total);

    Ok(())
}

/// 全局 HTTP 客户端（连接池复用，避免每次下载都创建新连接）
static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1800))
        .build()
        .expect("Failed to create HTTP client")
});

/// 从 HTTP(S) URL 下载文件到本地路径
async fn download_file(url: &str, dest: &Path) -> Result<()> {
    let response = HTTP_CLIENT
        .get(url)
        .send()
        .await
        .with_context(|| format!("下载请求失败: {}", url))?;

    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("下载失败，HTTP {}: {}", status.as_u16(), url);
    }

    let total_size = response.content_length();
    let mut downloaded: u64 = 0;
    let mut last_log_pct: u8 = 0;

    let mut file = tokio::fs::File::create(dest)
        .await
        .with_context(|| format!("无法创建文件: {}", dest.display()))?;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("下载数据流读取失败")?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        // log progress every 10%
        if let Some(total) = total_size
            && total > 0
        {
            let pct = ((downloaded as f64 / total as f64) * 100.0) as u8;
            let pct_rounded = (pct / 10) * 10;
            if pct_rounded > last_log_pct {
                last_log_pct = pct_rounded;
                tracing::info!(
                    "download: {}% ({:.1} / {:.1} MB)",
                    pct_rounded,
                    downloaded as f64 / (1024.0 * 1024.0),
                    total as f64 / (1024.0 * 1024.0)
                );
            }
        }
    }

    file.flush().await?;

    // integrity: compare actual size with Content-Length
    if let Some(expected) = total_size {
        let actual = tokio::fs::metadata(dest).await?.len();
        if actual != expected {
            anyhow::bail!("download truncated: expected {expected} bytes, got {actual}");
        }
    }

    // ensure 100% is logged
    if let Some(total) = total_size
        && total > 0
        && last_log_pct < 100
    {
        tracing::info!(
            "download: 100% ({:.1} / {:.1} MB)",
            total as f64 / (1024.0 * 1024.0),
            total as f64 / (1024.0 * 1024.0)
        );
    }

    Ok(())
}

/// 解压 tar.gz 到目标目录
pub(crate) fn extract_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("无法打开压缩包: {}", archive_path.display()))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    // tar::Entry::unpack 已内置路径穿越安全检查，无需额外的 .. 字符串过滤
    let entries = archive.entries().context("读取压缩包条目失败")?;
    let mut file_count: usize = 0;
    let mut dir_count: usize = 0;

    for entry in entries {
        let mut entry = entry.context("读取压缩包条目失败")?;
        let path = entry.path().context("获取条目路径失败")?;

        // 路径穿越保护：用 Path::components 规范化路径，检查是否包含父目录引用
        // 比 contains("..") 更精确（不会误杀 foo..bar 这类合法文件名）
        if path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            tracing::warn!("跳过路径穿越条目: {:?}", path);
            continue;
        }

        let target = dest_dir.join(&*path);

        // 创建父目录
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("无法创建目录: {}", parent.display()))?;
        }

        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&target)
                .with_context(|| format!("无法创建目录: {}", target.display()))?;
            dir_count += 1;
        } else {
            entry
                .unpack(&target)
                .with_context(|| format!("无法解压文件: {}", target.display()))?;
            file_count += 1;
        }
    }

    tracing::info!("解压了 {} 个目录, {} 个文件", dir_count, file_count);
    Ok(())
}
