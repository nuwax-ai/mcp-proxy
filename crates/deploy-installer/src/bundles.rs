use std::path::{Path, PathBuf};

#[derive(Debug, serde::Deserialize)]
struct DeployManifest {
    /// OSS tarball version (e.g. `0.2.1`). Decoupled from npm package `version`.
    #[serde(default, rename = "assetVersion")]
    asset_version: Option<String>,
    #[serde(default, rename = "optionalAssets")]
    optional_assets: OptionalAssets,
}

#[derive(Debug, Default, serde::Deserialize)]
struct OptionalAssets {
    #[serde(default)]
    venv: std::collections::HashMap<String, String>,
    #[serde(default, rename = "whisperLargeV3")]
    whisper_large_v3: std::collections::HashMap<String, String>,
    #[serde(default, rename = "whisperAll")]
    whisper_all: std::collections::HashMap<String, String>,
    #[serde(default, rename = "voiceCliCuda")]
    voice_cli_cuda: std::collections::HashMap<String, String>,
}

/// Which prebuilt Whisper ggml tarball to fetch from OSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhisperModelsPack {
    /// Default deploy: `ggml-large-v3.bin` only (~3GB).
    LargeV3,
    /// All supported ggml models (tiny … large-v3).
    All,
}

impl WhisperModelsPack {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "large-v3" | "large_v3" => Some(Self::LargeV3),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    /// Tarball file name for a deploy asset version.
    pub fn archive_filename(&self, version: &str) -> String {
        match self {
            Self::LargeV3 => format!("whisper-ggml-large-v3-{version}.tar.gz"),
            Self::All => format!("whisper-ggml-all-{version}.tar.gz"),
        }
    }
}

/// Vendor key for the current platform, e.g. `darwin-arm64`.
pub fn platform_vendor_key() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        (os, arch) => panic!("unsupported platform for deploy-installer: {os}-{arch}"),
    }
}

/// Root of bundled vendor assets (`NUWAX_DEPLOY_ROOT` or adjacent to the binary).
pub fn deploy_root() -> PathBuf {
    if let Ok(root) = std::env::var("NUWAX_DEPLOY_ROOT")
        && !root.is_empty()
    {
        return PathBuf::from(root);
    }
    // Walk up from executable to find workspace npm vendor (cargo run / dev)
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(PathBuf::from);
        for _ in 0..12 {
            let Some(d) = dir.clone() else { break };
            let candidate = d.join("npm/nuwax-deploy-installer/vendor");
            if candidate.is_dir() {
                return candidate;
            }
            let vendor = d.join("vendor");
            if vendor.is_dir() {
                return vendor;
            }
            let mut parent = d;
            if !parent.pop() {
                break;
            }
            dir = Some(parent);
        }
    }
    PathBuf::from("vendor")
}

/// Package version from `NUWAX_DEPLOY_VERSION` or workspace default.
pub fn deploy_version() -> String {
    std::env::var("NUWAX_DEPLOY_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
}

/// SemVer core used for OSS optional assets.
///
/// Prefer `assetVersion` in `vendor/templates/manifest.json` so beta npm releases
/// (e.g. `0.2.3-beta.2`) can reuse stable OSS tarballs (`0.2.1`). Falls back to
/// stripping prerelease from the package version when `assetVersion` is absent.
pub fn deploy_asset_version() -> String {
    manifest_asset_version().unwrap_or_else(|| {
        let v = deploy_version();
        v.split('-').next().unwrap_or(&v).to_string()
    })
}

fn load_manifest() -> Option<DeployManifest> {
    let path = deploy_root().join("templates/manifest.json");
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Read `assetVersion` from manifest when set and non-empty.
fn manifest_asset_version() -> Option<String> {
    let manifest = load_manifest()?;
    let v = manifest.asset_version?;
    if v.is_empty() { None } else { Some(v) }
}

/// `vendor/<platform>/document-parser` bundled binary.
pub fn bundled_binary_path(service: &str) -> PathBuf {
    deploy_root().join(platform_vendor_key()).join(service)
}

/// `vendor/templates/<service>/`
pub fn bundled_templates_dir(service: &str) -> PathBuf {
    deploy_root().join("templates").join(service)
}

/// Default install directory for document-parser on macOS.
pub fn default_document_parser_install_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join("document-parser");
    }
    PathBuf::from("./document-parser")
}

/// Default install directory for voice-cli on macOS.
pub fn default_voice_cli_install_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join("voice-cli");
    }
    PathBuf::from("./voice-cli")
}

/// Resolve optional prebuilt venv download URL from `vendor/templates/manifest.json`.
pub fn optional_venv_download_url() -> Option<String> {
    optional_asset_url(|assets| assets.venv.get(platform_vendor_key()))
}

fn optional_asset_url(pick: impl FnOnce(&OptionalAssets) -> Option<&String>) -> Option<String> {
    let manifest = load_manifest()?;
    let template = pick(&manifest.optional_assets)?;
    let version = deploy_asset_version();
    Some(template.replace("{version}", &version))
}

/// OSS tarball basename for prebuilt voice-cli + sherpa CUDA libs (Linux x86_64).
pub fn voice_cli_cuda_archive_filename(version: &str) -> String {
    format!("voice-cli-cuda-linux-x64-{version}.tar.gz")
}

/// Resolve optional voice-cli CUDA bundle URL from manifest (Linux x86_64 only).
pub fn optional_voice_cli_cuda_url() -> Option<String> {
    optional_asset_url(|assets| assets.voice_cli_cuda.get("linux-x64"))
}

/// Build voice-cli CUDA tarball URL from an OSS base directory.
pub fn voice_cli_cuda_download_url_from_base(base: &str) -> String {
    let version = deploy_asset_version();
    let name = voice_cli_cuda_archive_filename(&version);
    format!("{}/{}", base.trim_end_matches('/'), name)
}

/// Resolve optional prebuilt Whisper ggml tarball URL from manifest.
pub fn optional_whisper_download_url(pack: WhisperModelsPack) -> Option<String> {
    optional_asset_url(|assets| match pack {
        WhisperModelsPack::LargeV3 => assets.whisper_large_v3.get(platform_vendor_key()),
        WhisperModelsPack::All => assets.whisper_all.get(platform_vendor_key()),
    })
}

/// Build Whisper tarball URL from an OSS base directory and pack kind.
pub fn whisper_download_url_from_base(base: &str, pack: WhisperModelsPack) -> String {
    let version = deploy_asset_version();
    let name = pack.archive_filename(&version);
    format!("{}/{}", base.trim_end_matches('/'), name)
}

/// Copy a file if the source exists.
pub fn copy_if_exists(src: &Path, dst: &Path) -> std::io::Result<bool> {
    if !src.exists() {
        return Ok(false);
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst)?;
    Ok(true)
}

#[cfg(unix)]
pub fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
pub fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 读取仓库 manifest 的 assetVersion（测试期望跟随 manifest 而非硬编码，
    /// 提升 assetVersion 时无需同步改测试）。
    fn manifest_asset_version() -> String {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../npm/nuwax-deploy-installer/vendor/templates/manifest.json");
        let content = std::fs::read_to_string(path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        v["assetVersion"].as_str().unwrap().to_string()
    }

    #[test]
    fn asset_version_strips_prerelease_when_manifest_missing_asset_version() {
        // 构造无 assetVersion 的 manifest，验证回退到包版本剥离 prerelease
        let dir = tempfile::tempdir().unwrap();
        let templates = dir.path().join("templates");
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("manifest.json"),
            r#"{"version": "0.2.1", "optionalAssets": {}}"#,
        )
        .unwrap();
        // SAFETY: test-only env mutation; no concurrent env access in unit tests.
        unsafe {
            std::env::set_var("NUWAX_DEPLOY_ROOT", dir.path().display().to_string());
            std::env::set_var("NUWAX_DEPLOY_VERSION", "0.2.9-beta.2");
        }
        assert_eq!(deploy_asset_version(), "0.2.9");
        unsafe {
            std::env::set_var("NUWAX_DEPLOY_VERSION", "0.2.9");
        }
        assert_eq!(deploy_asset_version(), "0.2.9");
    }

    #[test]
    fn asset_version_from_manifest_overrides_newer_package_version() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../npm/nuwax-deploy-installer/vendor");
        let expected = manifest_asset_version();
        // SAFETY: test-only env mutation; no concurrent env access in unit tests.
        unsafe {
            std::env::set_var("NUWAX_DEPLOY_ROOT", root.display().to_string());
            std::env::set_var("NUWAX_DEPLOY_VERSION", "0.2.3-beta.2");
        }
        assert_eq!(
            deploy_asset_version(),
            expected,
            "manifest assetVersion should pin OSS filenames"
        );
        let url = optional_whisper_download_url(WhisperModelsPack::LargeV3).unwrap();
        assert!(
            url.contains(&format!("whisper-ggml-large-v3-{expected}.tar.gz")),
            "got {url}"
        );
        assert!(!url.contains("0.2.3"));
    }

    #[test]
    fn optional_venv_url_from_manifest() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../npm/nuwax-deploy-installer/vendor");
        let expected = manifest_asset_version();
        // SAFETY: test-only env mutation; no concurrent env access in unit tests.
        unsafe {
            std::env::set_var("NUWAX_DEPLOY_ROOT", root.display().to_string());
            std::env::set_var("NUWAX_DEPLOY_VERSION", "0.2.1-beta.2");
        }
        let url = optional_venv_download_url();
        assert!(
            url.is_some(),
            "manifest should provide darwin-arm64 venv URL"
        );
        let url = url.unwrap();
        assert!(
            url.contains(&format!("venv-macos-arm64-{expected}.tar.gz")),
            "beta package must reuse stable venv asset, got {url}"
        );
        assert!(!url.contains("beta"));
    }

    #[test]
    fn optional_whisper_large_v3_url_from_manifest() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../npm/nuwax-deploy-installer/vendor");
        let expected = manifest_asset_version();
        // SAFETY: test-only env mutation; no concurrent env access in unit tests.
        unsafe {
            std::env::set_var("NUWAX_DEPLOY_ROOT", root.display().to_string());
            std::env::set_var("NUWAX_DEPLOY_VERSION", "0.2.1-beta.2");
        }
        let url = optional_whisper_download_url(WhisperModelsPack::LargeV3);
        assert!(url.is_some(), "manifest should provide whisperLargeV3 URL");
        let url = url.unwrap();
        assert!(
            url.contains(&format!("whisper-ggml-large-v3-{expected}.tar.gz")),
            "got {url}"
        );
        assert!(!url.contains("beta"));
    }

    #[test]
    fn optional_voice_cli_cuda_url_from_manifest() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../npm/nuwax-deploy-installer/vendor");
        let expected = manifest_asset_version();
        // SAFETY: test-only env mutation; no concurrent env access in unit tests.
        unsafe {
            std::env::set_var("NUWAX_DEPLOY_ROOT", root.display().to_string());
            std::env::set_var("NUWAX_DEPLOY_VERSION", "0.2.1-beta.2");
        }
        let url = optional_voice_cli_cuda_url();
        assert!(url.is_some(), "manifest should provide voiceCliCuda URL");
        let url = url.unwrap();
        assert!(
            url.contains(&format!("voice-cli-cuda-linux-x64-{expected}.tar.gz")),
            "got {url}"
        );
        assert!(!url.contains("beta"));
    }
}
