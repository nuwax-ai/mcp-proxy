//! 归档下载/解压：curl 下载 + tar 解压 + voice-cli GPU bundle 的原子落位。
//!
//! 三个入口的分工：`download_and_extract_tarball` 通用下载+直解（venv /
//! whisper 模型——目标目录无运行中文件，直解安全）；`extract_tarball_at`
//! 本地归档解压（`--venv-file` 离线安装入口）；`download_and_extract_bundle_atomic`
//! voice-cli GPU bundle 专用（会覆盖运行中的二进制/.so，必须 rename 顶替）。

use super::common::copy_file_atomic;
use crate::make_executable;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;
use std::process::Command;

/// curl 下载到 `archive`（两个下载入口共用；失败时删除半截文件）。
fn curl_download(url: &str, archive: &Path, quiet: bool, label: &str) -> Result<()> {
    let mut curl = Command::new("curl");
    curl.args(["-fL"]);
    if quiet {
        curl.args(["-s", "-S"]);
    } else {
        curl.arg("-#");
    }
    let status = curl
        .args([url, "-o", &archive.display().to_string()])
        .status()
        .with_context(|| format!("curl download {label}"))?;
    if !status.success() {
        let _ = fs::remove_file(archive);
        bail!("failed to download {label} from {url}");
    }
    Ok(())
}

/// Download a tarball with curl and extract into `install_dir`.
pub fn download_and_extract_tarball(
    url: &str,
    install_dir: &Path,
    archive_basename: &str,
    quiet: bool,
    label: &str,
) -> Result<()> {
    let archive = install_dir.join(archive_basename);
    if !quiet {
        println!("  {label}: downloading {url}");
    }
    curl_download(url, &archive, quiet, label)?;
    extract_tarball_at(&archive, install_dir, quiet, label)?;
    let _ = fs::remove_file(&archive);
    Ok(())
}

/// Extract an existing local tarball into `install_dir`（离线安装入口，
/// 由 `--venv-file` 等本地包参数复用；下载场景经 [`download_and_extract_tarball`]）。
pub fn extract_tarball_at(
    archive: &Path,
    install_dir: &Path,
    quiet: bool,
    label: &str,
) -> Result<()> {
    let status = Command::new("tar")
        .args([
            "-xzf",
            &archive.display().to_string(),
            "-C",
            &install_dir.display().to_string(),
        ])
        .status()
        .with_context(|| format!("extract {label}"))?;
    if !status.success() {
        bail!("failed to extract {label} archive: {}", archive.display());
    }
    if !quiet {
        println!("  {label}: extracted to {}", install_dir.display());
    }
    Ok(())
}

/// 下载 voice-cli GPU bundle 并**原子落位**：先解压到 `install_dir` 下的临时
/// staging 子目录，再对已知成员逐个 [`copy_file_atomic`]（rename 顶替）就位。
///
/// 为什么不直接 `tar -xzf -C install_dir`：tar 对**正在运行**的二进制/.so 是
/// 原地截断写，Linux 以 ETXTBSY 拒绝（rename 顶替运行中文件才合法——语义同
/// `copy_file_atomic` 的注释）。档位互切（如 vulkan 机上 `install
/// --use-oss-cuda`）会覆盖运行中的 voice-cli 与伴生 .so，直解即炸。成员清单
/// 已知（`VOICE_CLI_*_BUNDLE_FILES`），逐成员校验缺失即报错。
pub fn download_and_extract_bundle_atomic(
    url: &str,
    install_dir: &Path,
    members: &[&str],
    archive_basename: &str,
    quiet: bool,
    label: &str,
) -> Result<()> {
    let archive = install_dir.join(archive_basename);
    if !quiet {
        println!("  {label}: downloading {url}");
    }
    curl_download(url, &archive, quiet, label)?;

    let staging = install_dir.join(format!(".bundle-staging-{}", std::process::id()));
    // 残留 staging（上次中断）先清：同 pid 重入或手工残留都不影响本次落位
    let _ = fs::remove_dir_all(&staging);
    let place = (|| -> Result<()> {
        fs::create_dir_all(&staging)
            .with_context(|| format!("create staging dir {}", staging.display()))?;
        let status = Command::new("tar")
            .args([
                "-xzf",
                &archive.display().to_string(),
                "-C",
                &staging.display().to_string(),
            ])
            .status()
            .with_context(|| format!("extract {label}"))?;
        if !status.success() {
            bail!("failed to extract {label} archive: {}", archive.display());
        }
        for name in members {
            let src = staging.join(name);
            let dst = install_dir.join(name);
            if !src.exists() {
                bail!("{label} archive missing member {name}");
            }
            // fs::copy 保留权限位（tar 里的可执行位随之就位），make_executable 兜底
            copy_file_atomic(&src, &dst)?;
            make_executable(&dst)?;
        }
        Ok(())
    })();
    // 归档与 staging 无论成败都清掉（成员已原子就位，回滚不需要它们）
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_file(&archive);
    place?;
    if !quiet {
        println!("  {label}: extracted to {}", install_dir.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== 上传后端二选一 / env 落盘 / 本地解压 =====

    #[cfg(unix)]
    #[test]
    fn extract_tarball_at_extracts_local_venv_archive() {
        let work = tempfile::TempDir::new().unwrap();
        let staging = work.path().join("staging");
        let install = work.path().join("install");
        std::fs::create_dir_all(staging.join("venv/bin")).unwrap();
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(staging.join("venv/bin/python"), b"#!/bin/sh\n").unwrap();

        let archive = work.path().join("venv-test.tar.gz");
        let status = std::process::Command::new("tar")
            .args(["-czf", &archive.display().to_string(), "-C"])
            .arg(&staging)
            .arg("venv")
            .status()
            .unwrap();
        assert!(status.success(), "打包测试归档失败");

        extract_tarball_at(&archive, &install, true, "venv-test").unwrap();
        assert!(
            install.join("venv/bin/python").is_file(),
            "解压后应存在 venv/bin/python"
        );

        // 损坏归档 → Err
        let bad = work.path().join("bad.tar.gz");
        std::fs::write(&bad, b"not a tarball").unwrap();
        assert!(extract_tarball_at(&bad, &install, true, "bad").is_err());
    }
}
