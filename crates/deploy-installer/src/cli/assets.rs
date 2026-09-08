//! 部署资产：tar 下载/解压、Whisper 预编译模型探测与落位、voice-cli 伴生库
//! （macOS dylib / Linux .so / Windows DLL）与 Linux CUDA bundle 探测。

use super::common::copy_file_atomic;
use crate::{DropIn, WhisperModelsPack, make_executable};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const WHISPER_DEFAULT_MODEL: &str = "large-v3";
pub const WHISPER_MODEL_FILE: &str = "ggml-large-v3.bin";
pub const WHISPER_ALL_MODEL_NAMES: &[&str] = &["tiny", "base", "small", "medium", "large-v3"];
/// Model names inside `whisper-ggml-all-{version}.tar.gz`.
/// macOS shared-mode libs that must sit next to `voice-cli` (`@loader_path` / `@rpath`).
#[cfg(target_os = "macos")]
const VOICE_CLI_REQUIRED_LIBS: &[&str] =
    &["libsherpa-onnx-c-api.dylib", "libonnxruntime.1.24.4.dylib"];

#[cfg(target_os = "macos")]
const VOICE_CLI_OPTIONAL_LIBS: &[&str] = &["libonnxruntime.dylib"];

#[cfg(target_os = "windows")]
const VOICE_CLI_REQUIRED_LIBS: &[&str] = &["sherpa-onnx-c-api.dll", "onnxruntime.dll"];

#[cfg(target_os = "windows")]
const VOICE_CLI_OPTIONAL_LIBS: &[&str] = &[
    "sherpa-onnx-cxx-api.dll",
    "onnxruntime_providers_shared.dll",
];

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const VOICE_CLI_REQUIRED_LIBS: &[&str] = &[];

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const VOICE_CLI_OPTIONAL_LIBS: &[&str] = &[];

/// Linux CUDA OSS bundle: binary + sherpa/onnx shared libs (flat extract into install_dir).
#[cfg(target_os = "linux")]
pub const VOICE_CLI_CUDA_BUNDLE_FILES: &[&str] = &[
    "voice-cli",
    "libsherpa-onnx-c-api.so",
    "libonnxruntime.so",
    "libonnxruntime_providers_cuda.so",
    "libonnxruntime_providers_shared.so",
];

#[cfg(not(target_os = "linux"))]
pub const VOICE_CLI_CUDA_BUNDLE_FILES: &[&str] = &[];

/// Return true when a voice-cli CUDA OSS bundle is fully present in `install_dir`.
pub fn voice_cli_cuda_bundle_present(install_dir: &Path) -> bool {
    !VOICE_CLI_CUDA_BUNDLE_FILES.is_empty()
        && VOICE_CLI_CUDA_BUNDLE_FILES.iter().all(|name| {
            let path = install_dir.join(name);
            path.exists() && fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false)
        })
}

/// Linux CUDA 运行时预检（voice-cli CUDA bundle 的启动前置）。
///
/// bundle 不含 libcublas——它依赖系统 CUDA 工具包（/usr/local/cuda/lib64）或
/// ldconfig 可达的 CUDA 库；nvidia-smi 只证明驱动。两者任一缺失时 CUDA bundle
/// 装上也无法启动（131 无 GPU 机实测：libcublas.so.12 not found 崩溃循环）。
pub fn linux_cuda_runtime_available() -> (bool, bool) {
    // nvidia-smi 可执行且退出成功
    let has_smi = std::process::Command::new("nvidia-smi")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    // libcublas：CUDA 工具包目录存在，或 ldconfig 缓存可解析
    let cublas_in_toolkit = std::path::Path::new("/usr/local/cuda/lib64/libcublas.so.12").exists();
    let cublas_in_ldconfig = std::process::Command::new("ldconfig")
        .arg("-p")
        .output()
        .ok()
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).contains("libcublas.so.12"));
    (has_smi, cublas_in_toolkit || cublas_in_ldconfig)
}

/// 纯函数：预检结果 → 提示文案（四分支单测；空串 = 预检通过无需提示）。
pub fn cuda_preflight_report(has_smi: bool, cublas_ok: bool) -> String {
    match (has_smi, cublas_ok) {
        (true, true) => String::new(),
        (true, false) => "检测到 NVIDIA 驱动（nvidia-smi）但缺 CUDA 工具包库（libcublas.so.12）——\n  安装: sudo apt install cuda-toolkit-12-6（或 nvidia-cuda-toolkit）".into(),
        (false, true) => "未检测到 nvidia-smi（无 NVIDIA 驱动/GPU）——CUDA bundle 无法使用 GPU\n  如需 GPU 加速: 安装 NVIDIA 驱动后重装".into(),
        (false, false) => "未检测到 NVIDIA GPU 与 CUDA 运行时（nvidia-smi、libcublas 均缺失）——\n  CUDA bundle 在本机无法启动；GPU 加速需: NVIDIA 驱动 + cuda-toolkit".into(),
    }
}

/// systemd drop-in for CUDA/cuDNN `LD_LIBRARY_PATH` (install_dir first for bundled .so).
pub fn build_cuda_sherpa_drop_in(
    install_dir: &Path,
    cuda_lib_dir: Option<&Path>,
    cudnn_lib_dir: Option<&Path>,
) -> Option<DropIn> {
    if cuda_lib_dir.is_none() && cudnn_lib_dir.is_none() {
        return None;
    }
    let mut parts: Vec<String> = vec![install_dir.display().to_string()];
    if let Some(p) = cudnn_lib_dir {
        parts.push(p.display().to_string());
    }
    if let Some(p) = cuda_lib_dir {
        parts.push(p.display().to_string());
    }
    let ld = parts.join(":");
    Some(DropIn {
        name: "cuda-sherpa".into(),
        content: format!("[Service]\nEnvironment=LD_LIBRARY_PATH={ld}\n"),
    })
}

/// Default NVIDIA CUDA toolkit lib dir when present on the host.
pub fn default_cuda_lib_dir() -> Option<PathBuf> {
    for candidate in ["/usr/local/cuda/lib64", "/usr/local/cuda/lib"] {
        let p = PathBuf::from(candidate);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

/// Best-effort cuDNN lib discovery (explicit env, then document-parser venv).
pub fn detect_cudnn_lib_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CUDNN_LIB_DIR") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let venv_lib = PathBuf::from(home).join("document-parser/venv/lib");
        if let Ok(entries) = fs::read_dir(&venv_lib) {
            for entry in entries.flatten() {
                let cudnn = entry.path().join("site-packages/nvidia/cudnn/lib");
                if cudnn.is_dir() {
                    return Some(cudnn);
                }
            }
        }
    }
    None
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
        bail!("failed to download {label} from {url}");
    }
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

/// Set `whisper.default_model` in a YAML config (line-based, whisper section only).
pub fn patch_whisper_default_model(config_path: &Path, model: &str) -> Result<()> {
    if !config_path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(config_path)?;
    let mut out = String::new();
    let mut in_whisper = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('#') && trimmed.ends_with(':') && !trimmed.contains(' ') {
            in_whisper = trimmed == "whisper:";
        }
        if in_whisper && trimmed.starts_with("default_model:") {
            let indent = line.len().saturating_sub(line.trim_start().len());
            out.push_str(&format!(
                "{:indent$}default_model: \"{model}\"\n",
                "",
                indent = indent
            ));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    fs::write(config_path, out)?;
    Ok(())
}

/// Minimum bytes for a plausible `ggml-large-v3.bin` (reject tiny smoke-test stubs).
/// large-v3 模型体积下限：识别占位/截断文件（真实 ~3GB）。
const WHISPER_LARGE_V3_MIN_BYTES: u64 = 500_000_000;

pub fn whisper_large_v3_present(install_dir: &Path) -> bool {
    whisper_model_present(install_dir, "large-v3")
}

fn whisper_model_present(install_dir: &Path, name: &str) -> bool {
    let model = install_dir.join("models").join(format!("ggml-{name}.bin"));
    let Ok(meta) = fs::metadata(&model) else {
        return false;
    };
    if name == "large-v3" {
        meta.len() >= WHISPER_LARGE_V3_MIN_BYTES
    } else {
        meta.len() > 0
    }
}

/// Return true when the requested OSS Whisper pack is already on disk.
pub fn whisper_pack_satisfied(install_dir: &Path, pack: WhisperModelsPack) -> bool {
    match pack {
        WhisperModelsPack::LargeV3 => whisper_large_v3_present(install_dir),
        WhisperModelsPack::All => WHISPER_ALL_MODEL_NAMES
            .iter()
            .all(|name| whisper_model_present(install_dir, name)),
    }
}

pub fn ensure_whisper_pack_models(install_dir: &Path, pack: WhisperModelsPack) -> Result<()> {
    if whisper_pack_satisfied(install_dir, pack) {
        return Ok(());
    }
    match pack {
        WhisperModelsPack::LargeV3 => ensure_whisper_large_v3_model(install_dir),
        WhisperModelsPack::All => {
            let missing: Vec<String> = WHISPER_ALL_MODEL_NAMES
                .iter()
                .filter(|name| !whisper_model_present(install_dir, name))
                .map(|name| format!("ggml-{name}.bin"))
                .collect();
            bail!(
                "missing whisper models: {} — upload whisper-ggml-all-{{version}}.tar.gz to OSS \
                 or use --oss-base",
                missing.join(", ")
            );
        }
    }
}

/// Ensure `models/ggml-large-v3.bin` exists after a Whisper OSS extract.
pub fn ensure_whisper_large_v3_model(install_dir: &Path) -> Result<()> {
    if whisper_large_v3_present(install_dir) {
        return Ok(());
    }
    let model = install_dir.join("models").join(WHISPER_MODEL_FILE);
    bail!(
        "missing or too small {} — upload whisper-ggml-large-v3-{{version}}.tar.gz to OSS \
         or use --oss-base",
        model.display()
    );
}

/// Copy sherpa/onnxruntime shared libs that must live beside `voice-cli`.
fn copy_voice_cli_companion_libs(src_dir: &Path, install_dir: &Path, quiet: bool) -> Result<()> {
    for name in VOICE_CLI_REQUIRED_LIBS {
        let src = src_dir.join(name);
        let dst = install_dir.join(name);
        if src.exists() {
            copy_file_atomic(&src, &dst)?;
            make_executable(&dst)?;
            if !quiet {
                println!("  copied companion lib {name}");
            }
        } else if !dst.exists() {
            bail!(
                "required companion lib missing: {} (and not already in {}) — \
                 re-run assemble / copy libsherpa-onnx-c-api + libonnxruntime next to voice-cli",
                src.display(),
                install_dir.display()
            );
        }
    }
    for name in VOICE_CLI_OPTIONAL_LIBS {
        let src = src_dir.join(name);
        let dst = install_dir.join(name);
        if src.exists() {
            copy_file_atomic(&src, &dst)?;
            if !quiet {
                println!("  copied companion lib {name}");
            }
        }
    }
    Ok(())
}

pub(crate) fn maybe_copy_companion_libs(
    service: &str,
    bundled_bin: &Path,
    install_dir: &Path,
    quiet: bool,
) -> Result<()> {
    if service != "voice-cli" {
        return Ok(());
    }
    let Some(src_dir) = bundled_bin.parent() else {
        return Ok(());
    };
    copy_voice_cli_companion_libs(src_dir, install_dir, quiet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn whisper_pack_large_v3_rejects_tiny_stub() {
        let dir = TempDir::new().unwrap();
        let model = dir.path().join("models/ggml-large-v3.bin");
        std::fs::create_dir_all(model.parent().unwrap()).unwrap();
        std::fs::write(&model, b"stub").unwrap();
        assert!(!whisper_pack_satisfied(
            dir.path(),
            WhisperModelsPack::LargeV3
        ));
    }

    #[test]
    fn whisper_pack_all_needs_every_model_file() {
        let dir = TempDir::new().unwrap();
        let models = dir.path().join("models");
        std::fs::create_dir_all(&models).unwrap();
        for name in ["tiny", "base", "small", "medium"] {
            std::fs::write(models.join(format!("ggml-{name}.bin")), b"x").unwrap();
        }
        // large-v3 missing
        assert!(!whisper_pack_satisfied(dir.path(), WhisperModelsPack::All));

        std::fs::write(models.join("ggml-large-v3.bin"), b"stub").unwrap();
        // large-v3 too small
        assert!(!whisper_pack_satisfied(dir.path(), WhisperModelsPack::All));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn voice_cli_cuda_bundle_requires_all_files() {
        let dir = TempDir::new().unwrap();
        assert!(!voice_cli_cuda_bundle_present(dir.path()));
        for name in VOICE_CLI_CUDA_BUNDLE_FILES {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        assert!(voice_cli_cuda_bundle_present(dir.path()));
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn voice_cli_cuda_bundle_not_applicable_off_linux() {
        let dir = TempDir::new().unwrap();
        assert!(!voice_cli_cuda_bundle_present(dir.path()));
    }

    // ===== 上传后端二选一 / env 落盘 / 本地解压 =====

    #[cfg(unix)]
    #[test]
    fn extract_tarball_at_extracts_local_venv_archive() {
        let work = TempDir::new().unwrap();
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

    #[test]
    fn cuda_preflight_report_all_branches() {
        use super::cuda_preflight_report;
        assert_eq!(cuda_preflight_report(true, true), "");
        assert!(cuda_preflight_report(true, false).contains("cuda-toolkit"));
        assert!(cuda_preflight_report(false, true).contains("nvidia-smi"));
        assert!(cuda_preflight_report(false, false).contains("均缺失"));
    }
}
