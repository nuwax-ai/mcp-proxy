//! 部署资产定义：voice-cli 伴生库（macOS dylib / Linux .so / Windows DLL）、
//! Linux GPU bundle 文件清单与在场判定、Whisper 预编译模型包探测与落位。
//! （归档下载/解压见 [`super::tarball`]；Linux 三档决策与 GPU 预检见
//! [`super::linux_gpu`]。）

use super::common::copy_file_atomic;
use crate::{WhisperModelsPack, make_executable};
use anyhow::{Result, bail};
use std::fs;
use std::path::Path;

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

/// Linux vendor CPU 版伴生 .so（RPATH=$ORIGIN 同目录解析）。历史上 Linux 只走
/// CUDA bundle（自带 .so），vendor companion 清单为空；0.2.12 的 CUDA 预检
/// CPU 回退路径首次真正安装 vendor Linux 二进制——实测缺清单导致启动 127。
#[cfg(target_os = "linux")]
const VOICE_CLI_REQUIRED_LIBS: &[&str] = &["libsherpa-onnx-c-api.so", "libonnxruntime.so"];

#[cfg(target_os = "linux")]
const VOICE_CLI_OPTIONAL_LIBS: &[&str] = &["libsherpa-onnx-cxx-api.so"];

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
const VOICE_CLI_REQUIRED_LIBS: &[&str] = &[];

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
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

/// Vulkan bundle 档位 marker：Vulkan 二进制静态链 ggml-vulkan，伴生 .so 与 CPU
/// vendor 版完全相同——按文件无法区分两档（CUDA 靠 providers_cuda.so 天然区分），
/// bundle 内必须自带标记（pack 脚本写入 "vulkan {VERSION}"，非空才有效）。
pub const VOICE_CLI_VULKAN_BUNDLE_MARKER: &str = ".voice-cli-vulkan";

/// CUDA bundle 版本 marker（安装器在 bundle 落位后写入 "cuda {VERSION}"）。
/// 与 vulkan marker 不同：不在 bundle tar 内、由安装器写入（CUDA tar 格式不改动，
/// 存量 bundle 无需重打）；档位判定仍靠 providers_cuda.so，此 marker 仅用于
/// 升级时的版本比对——旧安装无此文件视为版本未知，升级时重下刷新。
pub const VOICE_CLI_CUDA_BUNDLE_MARKER: &str = ".voice-cli-cuda";

/// Linux Vulkan OSS bundle: binary + sherpa/onnx CPU .so + 档位 marker。
/// （whisper/ggml-vulkan 全静态链入二进制；运行时 Vulkan 依赖只有系统
/// libvulkan.so.1，无需 bundle 自带。sherpa 与 vulkan feature 正交——TTS/ASR 仍 CPU。）
#[cfg(target_os = "linux")]
pub const VOICE_CLI_VULKAN_BUNDLE_FILES: &[&str] = &[
    "voice-cli",
    "libsherpa-onnx-c-api.so",
    "libonnxruntime.so",
    VOICE_CLI_VULKAN_BUNDLE_MARKER,
];

#[cfg(not(target_os = "linux"))]
pub const VOICE_CLI_VULKAN_BUNDLE_FILES: &[&str] = &[];

/// Return true when a voice-cli CUDA OSS bundle is fully present in `install_dir`.
pub fn voice_cli_cuda_bundle_present(install_dir: &Path) -> bool {
    !VOICE_CLI_CUDA_BUNDLE_FILES.is_empty()
        && VOICE_CLI_CUDA_BUNDLE_FILES.iter().all(|name| {
            let path = install_dir.join(name);
            path.exists() && fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false)
        })
}

/// Return true when a voice-cli Vulkan OSS bundle is fully present in `install_dir`.
pub fn voice_cli_vulkan_bundle_present(install_dir: &Path) -> bool {
    !VOICE_CLI_VULKAN_BUNDLE_FILES.is_empty()
        && VOICE_CLI_VULKAN_BUNDLE_FILES.iter().all(|name| {
            let path = install_dir.join(name);
            path.exists() && fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false)
        })
}

/// 读 bundle 档位 marker 的版本（内容形如 "vulkan 0.2.14" / "cuda 0.2.14"，
/// 由 pack 脚本或安装器写入）。marker 缺失/非两段格式返回 None（版本未知，
/// 调用方应按"需要重新下载"处理，而非沿用旧 bundle）。
pub fn voice_cli_bundle_marker_version(install_dir: &Path, marker: &str) -> Option<String> {
    let content = fs::read_to_string(install_dir.join(marker)).ok()?;
    content.split_whitespace().nth(1).map(str::to_string)
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
            make_executable(&dst)?;
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
    fn bundle_marker_version_parses_two_segment_content() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".voice-cli-vulkan"), "vulkan 0.2.14").unwrap();
        assert_eq!(
            voice_cli_bundle_marker_version(dir.path(), VOICE_CLI_VULKAN_BUNDLE_MARKER),
            Some("0.2.14".to_string())
        );
    }

    #[test]
    fn bundle_marker_version_none_on_missing_or_malformed() {
        let dir = TempDir::new().unwrap();
        // 无 marker（旧安装 / CPU 档）→ None
        assert_eq!(
            voice_cli_bundle_marker_version(dir.path(), VOICE_CLI_CUDA_BUNDLE_MARKER),
            None
        );
        // 单段内容（旧格式只要求非空）→ None（版本未知）
        std::fs::write(dir.path().join(VOICE_CLI_CUDA_BUNDLE_MARKER), "cuda").unwrap();
        assert_eq!(
            voice_cli_bundle_marker_version(dir.path(), VOICE_CLI_CUDA_BUNDLE_MARKER),
            None
        );
    }

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

    #[test]
    #[cfg(target_os = "linux")]
    fn voice_cli_vulkan_bundle_requires_all_files() {
        let dir = TempDir::new().unwrap();
        assert!(!voice_cli_vulkan_bundle_present(dir.path()));
        // 缺 marker → 不算 vulkan 档（与 CPU 安装按文件不可区分，marker 是唯一判据）
        for name in ["voice-cli", "libsherpa-onnx-c-api.so", "libonnxruntime.so"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        assert!(!voice_cli_vulkan_bundle_present(dir.path()));
        std::fs::write(
            dir.path().join(VOICE_CLI_VULKAN_BUNDLE_MARKER),
            b"vulkan 0.2.12",
        )
        .unwrap();
        assert!(voice_cli_vulkan_bundle_present(dir.path()));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn voice_cli_vulkan_bundle_rejects_empty_marker() {
        let dir = TempDir::new().unwrap();
        for name in ["voice-cli", "libsherpa-onnx-c-api.so", "libonnxruntime.so"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        // 空 marker（只 touch 未写内容，present 判定要求 len>0）不算 vulkan 档
        std::fs::write(dir.path().join(VOICE_CLI_VULKAN_BUNDLE_MARKER), b"").unwrap();
        assert!(!voice_cli_vulkan_bundle_present(dir.path()));
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn voice_cli_vulkan_bundle_not_applicable_off_linux() {
        let dir = TempDir::new().unwrap();
        assert!(!voice_cli_vulkan_bundle_present(dir.path()));
    }
}
