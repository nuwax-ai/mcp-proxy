use std::path::{Path, PathBuf};

#[derive(Debug, serde::Deserialize)]
struct DeployManifest {
    #[serde(default, rename = "optionalAssets")]
    optional_assets: OptionalAssets,
}

#[derive(Debug, Default, serde::Deserialize)]
struct OptionalAssets {
    #[serde(default)]
    venv: std::collections::HashMap<String, String>,
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
    if let Ok(root) = std::env::var("NUWAX_DEPLOY_ROOT") {
        if !root.is_empty() {
            return PathBuf::from(root);
        }
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

/// SemVer core used for OSS optional assets (`0.2.1-beta.2` → `0.2.1`).
///
/// Prebuilt venv tarballs are published once per stable X.Y.Z and reused by beta builds.
pub fn deploy_asset_version() -> String {
    let v = deploy_version();
    v.split('-').next().unwrap_or(&v).to_string()
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

/// Resolve optional prebuilt venv download URL from `vendor/templates/manifest.json`.
pub fn optional_venv_download_url() -> Option<String> {
    let path = deploy_root().join("templates/manifest.json");
    let content = std::fs::read_to_string(path).ok()?;
    let manifest: DeployManifest = serde_json::from_str(&content).ok()?;
    let template = manifest.optional_assets.venv.get(platform_vendor_key())?;
    let version = deploy_asset_version();
    Some(template.replace("{version}", &version))
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

    #[test]
    fn asset_version_strips_prerelease() {
        unsafe {
            std::env::set_var("NUWAX_DEPLOY_VERSION", "0.2.1-beta.2");
        }
        assert_eq!(deploy_asset_version(), "0.2.1");
        unsafe {
            std::env::set_var("NUWAX_DEPLOY_VERSION", "0.2.1");
        }
        assert_eq!(deploy_asset_version(), "0.2.1");
    }

    #[test]
    fn optional_venv_url_from_manifest() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../npm/nuwax-deploy-installer/vendor");
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
            url.contains("venv-macos-arm64-0.2.1.tar.gz"),
            "beta package must reuse stable venv asset, got {url}"
        );
        assert!(!url.contains("beta"));
    }
}
