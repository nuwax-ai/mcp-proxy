//! Shared helpers for `document-parser` / `voice-cli` CLI modules.

use crate::{
    DropIn, WhisperModelsPack, bundled_binary_path, default_document_parser_install_dir,
    default_voice_cli_install_dir, group_for_user, make_executable, resolve_service_user,
    restart_in_dir, status_in_dir, uninstall_in_dir,
};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use super::{ServiceAction, ServiceDirArgs};

pub const CONFIG_FILENAME: &str = "config.yml";
pub const WHISPER_DEFAULT_MODEL: &str = "large-v3";
pub const WHISPER_MODEL_FILE: &str = "ggml-large-v3.bin";

/// Model names inside `whisper-ggml-all-{version}.tar.gz`.
pub const WHISPER_ALL_MODEL_NAMES: &[&str] = &["tiny", "base", "small", "medium", "large-v3"];

/// macOS shared-mode libs that must sit next to `voice-cli` (`@loader_path` / `@rpath`).
#[cfg(target_os = "macos")]
const VOICE_CLI_REQUIRED_LIBS: &[&str] =
    &["libsherpa-onnx-c-api.dylib", "libonnxruntime.1.24.4.dylib"];

#[cfg(target_os = "macos")]
const VOICE_CLI_OPTIONAL_LIBS: &[&str] = &["libonnxruntime.dylib"];

#[cfg(not(target_os = "macos"))]
const VOICE_CLI_REQUIRED_LIBS: &[&str] = &[];

#[cfg(not(target_os = "macos"))]
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

/// Read first non-comment `port:` value from a YAML-ish config file.
pub fn read_server_port(config_path: &Path) -> Option<u16> {
    let content = fs::read_to_string(config_path).ok()?;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            continue;
        }
        if let Some(rest) = t.strip_prefix("port:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

pub fn canonicalize_install_dir(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub fn resolve_user_group(user: Option<String>) -> Result<(String, String)> {
    let user = resolve_service_user(user).context("resolve service user")?;
    let group = group_for_user(&user).context("resolve service group")?;
    Ok((user, group))
}

/// Resolve `.` or missing install dir to the service default under `$HOME`.
pub fn resolve_service_install_dir(service: &str, install_dir: &Path) -> PathBuf {
    if install_dir.as_os_str().is_empty() || install_dir == Path::new(".") {
        match service {
            "document-parser" => default_document_parser_install_dir(),
            "voice-cli" => default_voice_cli_install_dir(),
            _ => install_dir.to_path_buf(),
        }
    } else {
        install_dir.to_path_buf()
    }
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
const WHISPER_LARGE_V3_MIN_BYTES: u64 = 500_000_000;

/// Return true when `models/ggml-large-v3.bin` looks like a real model.
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

/// Apply `OSS_ACCESS_KEY_ID` / `OSS_ACCESS_KEY_SECRET` from the environment into `.env`.
/// Returns true when both keys are now configured (file or env).
pub fn apply_oss_keys_from_env(env_path: &Path) -> Result<bool> {
    let id = std::env::var("OSS_ACCESS_KEY_ID")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let secret = std::env::var("OSS_ACCESS_KEY_SECRET")
        .ok()
        .filter(|s| !s.trim().is_empty());

    let (Some(id), Some(secret)) = (id, secret) else {
        return Ok(oss_keys_configured(env_path));
    };

    let mut lines: Vec<String> = read_env_lines(env_path);
    upsert_env_line(&mut lines, "OSS_ACCESS_KEY_ID", &id);
    upsert_env_line(&mut lines, "OSS_ACCESS_KEY_SECRET", &secret);
    write_env_lines(env_path, &lines)?;
    Ok(oss_keys_configured(env_path))
}

/// 上传后端相关的环境变量键：OSS 凭证 + 自定义上传后端（nuwax 风格）配置。
///
/// 语义与 document-parser 侧 `load_custom_upload_config_from_env` 对齐：
/// `DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL` trim 后非空即启用自定义后端，
/// api_key 允许为空（无鉴权部署），path 兜底 `/api/v1/file/upload`。
pub const UPLOAD_ENV_KEYS: &[&str] = &[
    "OSS_ACCESS_KEY_ID",
    "OSS_ACCESS_KEY_SECRET",
    "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL",
    "DOCUMENT_PARSER_CUSTOM_UPLOAD_API_KEY",
    "DOCUMENT_PARSER_CUSTOM_UPLOAD_PATH",
];

/// 把上传后端配置（OSS 密钥与/或自定义上传后端变量）从环境落盘到 `.env`。
///
/// 与 [`apply_oss_keys_from_env`] 不同：各键**独立** upsert（环境里非空即写），
/// 不要求 OSS 成对出现——"是否配置完成"的判定交给 [`upload_backend_configured`]。
pub fn apply_upload_config_from_env(env_path: &Path) -> Result<()> {
    let mut lines = read_env_lines(env_path);
    let mut changed = false;
    for key in UPLOAD_ENV_KEYS {
        if let Some(value) = std::env::var(key).ok().filter(|s| !s.trim().is_empty()) {
            upsert_env_line(&mut lines, key, &value);
            changed = true;
        }
    }
    if changed {
        write_env_lines(env_path, &lines)?;
    }
    Ok(())
}

/// Whether `.document-parser.env` has non-empty OSS keys.
pub fn oss_keys_configured(env_path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(env_path) else {
        return false;
    };
    let values = parse_env_file_values(&content);
    values
        .get("OSS_ACCESS_KEY_ID")
        .is_some_and(|v| !v.is_empty())
        && values
            .get("OSS_ACCESS_KEY_SECRET")
            .is_some_and(|v| !v.is_empty())
}

/// Whether `.document-parser.env` enables the custom upload backend
/// (`DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL` non-empty).
pub fn custom_upload_configured(env_path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(env_path) else {
        return false;
    };
    parse_env_file_values(&content)
        .get("DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL")
        .is_some_and(|v| !v.is_empty())
}

/// 上传后端是否就绪：OSS 密钥或自定义上传后端**二选一**即可。
pub fn upload_backend_configured(env_path: &Path) -> bool {
    oss_keys_configured(env_path) || custom_upload_configured(env_path)
}

/// Read a single KEY's value from an `.env`-style file (quotes stripped, None if absent).
pub fn parse_env_file_value(env_path: &Path, key: &str) -> Option<String> {
    let content = fs::read_to_string(env_path).ok()?;
    parse_env_file_values(&content)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

/// Read `.env`-style lines, dropping quoted values (KEY=VALUE with optional quotes).
fn parse_env_file_values(content: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('#') || t.is_empty() {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            map.insert(k.trim().to_string(), v.to_string());
        }
    }
    map
}

fn read_env_lines(env_path: &Path) -> Vec<String> {
    if env_path.exists() {
        fs::read_to_string(env_path)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    } else {
        Vec::new()
    }
}

fn write_env_lines(env_path: &Path, lines: &[String]) -> Result<()> {
    let body = format!("{}\n", lines.join("\n"));
    crate::write_user_file(env_path, &body, Some(0o600))
        .map_err(|e| anyhow::anyhow!("write {}: {e}", env_path.display()))
}

fn upsert_env_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{key}=");
    if let Some(line) = lines.iter_mut().find(|l| {
        let t = l.trim();
        !t.starts_with('#') && t.starts_with(&prefix)
    }) {
        *line = format!("{key}={value}");
    } else {
        lines.push(format!("{key}={value}"));
    }
}

/// Default `/health` wait after install (voice-cli starts in seconds).
const VOICE_CLI_HEALTH_WAIT_SECS: u64 = 45;
/// document-parser runs MinerU env checks on first boot; allow longer.
const DOCUMENT_PARSER_HEALTH_WAIT_SECS: u64 = 120;

/// Poll `http://127.0.0.1:{port}{path}` until curl succeeds or timeout.
pub fn wait_for_health(port: u16, path: &str, timeout_secs: u64) -> bool {
    let url = format!("http://127.0.0.1:{port}{path}");
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    while Instant::now() < deadline {
        if Command::new("curl")
            .args(["-fsS", "-o", "/dev/null", &url])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
        thread::sleep(Duration::from_secs(2));
    }
    false
}

/// Print a short success banner after `install` completes.
pub fn print_install_success(service: &str, install_dir: &Path, port: u16) {
    let timeout_secs = match service {
        "document-parser" => DOCUMENT_PARSER_HEALTH_WAIT_SECS,
        _ => VOICE_CLI_HEALTH_WAIT_SECS,
    };
    print!("\n   Checking /health (up to {timeout_secs}s)");
    let healthy = wait_for_health(port, "/health", timeout_secs);
    if healthy {
        println!(" … OK");
    } else {
        println!(" … still starting (retry: curl -fsS http://127.0.0.1:{port}/health)");
    }

    println!("\n✅ {service} → http://127.0.0.1:{port}");
    println!("   API docs: http://127.0.0.1:{port}/api/docs");
    println!("   Dir: {}", install_dir.display());
    println!("   Ops: deploy-installer {service} service status");
}

/// Ensure required Whisper model files exist after an OSS extract.
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
            fs::copy(&src, &dst)
                .with_context(|| format!("copy {} → {}", src.display(), dst.display()))?;
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
            fs::copy(&src, &dst)
                .with_context(|| format!("copy {} → {}", src.display(), dst.display()))?;
            if !quiet {
                println!("  copied companion lib {name}");
            }
        }
    }
    Ok(())
}

fn maybe_copy_companion_libs(
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

/// Copy `vendor/<platform>/<service>` into `install_dir/<service>` when the bundle exists.
///
/// If the bundle is missing, keeps an existing binary in place; otherwise errors.
/// For `voice-cli`, also copies macOS `@rpath` companion dylibs from the same vendor dir.
pub fn ensure_bundled_binary(service: &str, install_dir: &Path, quiet: bool) -> Result<PathBuf> {
    let bundled = bundled_binary_path(service);
    let dst = install_dir.join(service);
    if bundled.exists() {
        fs::copy(&bundled, &dst)
            .with_context(|| format!("copy {} → {}", bundled.display(), dst.display()))?;
        make_executable(&dst)?;
        if !quiet {
            println!("  copied binary from {}", bundled.display());
        }
        maybe_copy_companion_libs(service, &bundled, install_dir, quiet)?;
    } else if !dst.exists() {
        bail!(
            "bundled binary not found at {} and no existing binary in {}",
            bundled.display(),
            install_dir.display()
        );
    } else {
        // Keep existing binary; still require companions (from vendor or already installed).
        maybe_copy_companion_libs(service, &bundled, install_dir, quiet)?;
    }
    Ok(dst)
}

/// Replace `install_dir/<service>` from the npm/vendor bundle and print a restart hint.
pub fn upgrade_bundled_binary(service: &str, install_dir: &Path) -> Result<()> {
    let bundled = bundled_binary_path(service);
    let dst = install_dir.join(service);
    if !bundled.exists() {
        bail!("bundled binary not found at {}", bundled.display());
    }
    fs::copy(&bundled, &dst)?;
    make_executable(&dst)?;
    maybe_copy_companion_libs(service, &bundled, install_dir, false)?;
    println!("✅ upgraded {} → {}", bundled.display(), dst.display());
    println!("   Run: deploy-installer {service} service restart");
    Ok(())
}

/// Shared uninstall / status / restart; `on_install` handles service-specific Install.
pub fn dispatch_service_action(
    service_name: &str,
    action: ServiceAction,
    on_install: impl FnOnce(&ServiceDirArgs) -> Result<()>,
) -> Result<()> {
    match action {
        ServiceAction::Install(args) => {
            let mut args = args;
            args.install_dir = resolve_service_install_dir(service_name, &args.install_dir);
            on_install(&args)
        }
        ServiceAction::Uninstall(args) => {
            let dir = resolve_service_install_dir(service_name, &args.install_dir);
            uninstall_in_dir(service_name, Some(dir)).context("uninstall failed")
        }
        ServiceAction::Status(args) => {
            let dir = resolve_service_install_dir(service_name, &args.install_dir);
            status_in_dir(service_name, Some(dir)).context("status failed")
        }
        ServiceAction::Restart(args) => {
            let dir = resolve_service_install_dir(service_name, &args.install_dir);
            restart_in_dir(service_name, Some(dir)).context("restart failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WhisperModelsPack;
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

    fn write_env(dir: &TempDir, content: &str) -> std::path::PathBuf {
        let p = dir.path().join(".document-parser.env");
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn upload_backend_configured_three_states() {
        let dir = TempDir::new().unwrap();
        // 皆无 → false
        let p = write_env(&dir, "# empty\n");
        assert!(!oss_keys_configured(&p));
        assert!(!custom_upload_configured(&p));
        assert!(!upload_backend_configured(&p));

        // 仅 OSS 成对 → true
        let p = write_env(&dir, "OSS_ACCESS_KEY_ID=ak\nOSS_ACCESS_KEY_SECRET=sk\n");
        assert!(oss_keys_configured(&p));
        assert!(!custom_upload_configured(&p));
        assert!(upload_backend_configured(&p));

        // 仅 custom（base_url 非空，api_key 可空）→ true
        let p = write_env(
            &dir,
            "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL=https://agent.example.com\nDOCUMENT_PARSER_CUSTOM_UPLOAD_API_KEY=\n",
        );
        assert!(!oss_keys_configured(&p));
        assert!(custom_upload_configured(&p));
        assert!(upload_backend_configured(&p));

        // 引号值剥壳
        let p = write_env(
            &dir,
            "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL='https://x.com'\n",
        );
        assert!(custom_upload_configured(&p));

        // base_url 空串 = 未启用（与 document-parser 侧语义一致）
        let p = write_env(&dir, "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL=\n");
        assert!(!custom_upload_configured(&p));

        // 文件不存在 → false
        assert!(!upload_backend_configured(&dir.path().join("nope.env")));
    }

    #[test]
    fn apply_env_upsert_is_idempotent_and_skips_commented() {
        let mut lines: Vec<String> = vec![
            "# OSS_ACCESS_KEY_ID=old".to_string(),
            "OSS_ACCESS_KEY_ID=first".to_string(),
            String::new(),
        ];
        upsert_env_line(&mut lines, "OSS_ACCESS_KEY_ID", "second");
        // 覆盖非注释行，不动注释行
        assert_eq!(lines[0], "# OSS_ACCESS_KEY_ID=old");
        assert_eq!(lines[1], "OSS_ACCESS_KEY_ID=second");

        // 新键追加
        upsert_env_line(
            &mut lines,
            "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL",
            "https://x",
        );
        assert!(
            lines
                .last()
                .is_some_and(|l| l == "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL=https://x")
        );
    }

    #[test]
    fn parse_env_file_value_reads_and_strips_quotes() {
        let dir = TempDir::new().unwrap();
        let p = write_env(&dir, "# comment\nFOO='quoted'\nBAR=plain\nBAZ=\"dq\"\n");
        assert_eq!(parse_env_file_value(&p, "FOO").as_deref(), Some("quoted"));
        assert_eq!(parse_env_file_value(&p, "BAR").as_deref(), Some("plain"));
        assert_eq!(parse_env_file_value(&p, "BAZ").as_deref(), Some("dq"));
        assert_eq!(parse_env_file_value(&p, "MISSING"), None);
    }

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
}
