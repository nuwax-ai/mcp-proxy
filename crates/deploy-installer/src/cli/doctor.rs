use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::platform_vendor_key;
use crate::{
    bundled_binary_path, default_document_parser_install_dir, default_voice_cli_install_dir,
    deploy_root,
};

/// Minimum free disk for voice-cli model + venv (~5GB).
const MIN_FREE_BYTES: u64 = 5 * 1024 * 1024 * 1024;

#[cfg(target_os = "macos")]
const VOICE_CLI_REQUIRED_LIBS: &[&str] =
    &["libsherpa-onnx-c-api.dylib", "libonnxruntime.1.24.4.dylib"];

pub fn run() -> Result<()> {
    println!("==> deploy-installer doctor");
    println!(
        "  platform:   {} ({})",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("  vendor key: {}", platform_vendor_key());
    println!("  deploy root: {}", deploy_root().display());

    let voice_dir = default_voice_cli_install_dir();
    let parser_dir = default_document_parser_install_dir();
    println!("  defaults:   voice-cli → {}", voice_dir.display());
    println!("              document-parser → {}", parser_dir.display());

    let mut failed = false;

    check_command("node", &["--version"], false)?;
    check_command("curl", &["--version"], false)?;
    check_command("uv", &["--version"], true)?;

    if check_bundled_binary("voice-cli").is_err() {
        failed = true;
    }
    if check_bundled_binary("document-parser").is_err() {
        failed = true;
    }
    #[cfg(target_os = "macos")]
    {
        if check_voice_cli_companion_libs().is_err() {
            failed = true;
        }
    }

    check_install_dir_path(&voice_dir);
    check_install_dir_path(&parser_dir);
    check_disk_space(&voice_dir);

    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        if let Some(url) = crate::optional_voice_cli_cuda_url() {
            println!("  oss cuda:   OK ({url})");
        } else {
            println!("  oss cuda:   WARN (no voiceCliCuda URL in manifest.json)");
        }
    }

    if cfg!(target_os = "macos") {
        println!("  backend:    launchd (LaunchAgent)");
        check_macos_gui_session()?;
    } else {
        println!("  backend:    systemd");
        check_sudo()?;
    }

    if failed {
        bail!(
            "doctor found blocking issues (see FAIL above) — reinstall npm package or run assemble script"
        );
    }

    println!("\n✅ doctor checks passed (warnings above are OK for optional tools)");
    Ok(())
}

fn check_bundled_binary(service: &str) -> Result<()> {
    let path = bundled_binary_path(service);
    if path.exists() {
        println!("  bundle {service}: OK ({})", path.display());
        Ok(())
    } else {
        println!(
            "  bundle {service}: FAIL — missing {} (reinstall nuwax-deploy-installer@beta)",
            path.display()
        );
        Err(anyhow::anyhow!("missing bundled binary for {service}"))
    }
}

#[cfg(target_os = "macos")]
fn check_voice_cli_companion_libs() -> Result<()> {
    let vendor_dir = bundled_binary_path("voice-cli")
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| deploy_root().join(platform_vendor_key()));
    let mut missing = Vec::new();
    for name in VOICE_CLI_REQUIRED_LIBS {
        let path = vendor_dir.join(name);
        if path.exists() {
            println!("  bundle lib {name}: OK");
        } else {
            println!("  bundle lib {name}: FAIL — missing {}", path.display());
            missing.push(*name);
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "missing voice-cli companion libs: {}",
            missing.join(", ")
        ))
    }
}

fn check_install_dir_path(path: &Path) {
    let s = path.to_string_lossy();
    let restricted = ["/Documents/", "/Desktop/", "/Library/Mobile Documents/"];
    if restricted.iter().any(|seg| s.contains(seg))
        || s.ends_with("/Documents")
        || s.ends_with("/Desktop")
    {
        println!(
            "  install path {}: WARN — Documents/Desktop/iCloud may block LaunchAgent \
             (use ~/voice-cli or ~/document-parser)",
            path.display()
        );
    } else {
        println!("  install path {}: OK", path.display());
    }
}

fn check_disk_space(path: &Path) {
    let check_path = if path.exists() {
        path.to_path_buf()
    } else if let Some(parent) = path.parent() {
        parent.to_path_buf()
    } else {
        path.to_path_buf()
    };

    match available_bytes(&check_path) {
        Some(free) if free >= MIN_FREE_BYTES => {
            println!(
                "  disk space:  OK ({:.1} GB free at {})",
                free as f64 / (1024.0 * 1024.0 * 1024.0),
                check_path.display()
            );
        }
        Some(free) => {
            println!(
                "  disk space:  WARN ({:.1} GB free, recommend ≥ 5 GB for voice-cli model + venv)",
                free as f64 / (1024.0 * 1024.0 * 1024.0)
            );
        }
        None => println!("  disk space:  WARN (could not detect free space)"),
    }
}

/// Available bytes at `path` via `df -k` (portable on macOS/Linux).
fn available_bytes(path: &Path) -> Option<u64> {
    let path_str = path.to_str()?;
    let output = Command::new("df").args(["-k", path_str]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().nth(1)?;
    let available_k: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(available_k * 1024)
}

fn check_command(bin: &str, args: &[&str], optional: bool) -> Result<()> {
    match Command::new(bin).args(args).output() {
        Ok(out) if out.status.success() => {
            let ver = String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .unwrap_or("ok")
                .to_string();
            println!("  {bin}:       OK ({ver})");
            Ok(())
        }
        Ok(out) => {
            let msg = format!("exit {:?}", out.status.code());
            if optional {
                println!("  {bin}:       WARN ({msg}) — optional");
                Ok(())
            } else {
                bail!("{bin} check failed: {msg}");
            }
        }
        Err(e) => {
            if optional {
                println!("  {bin}:       WARN (not found: {e}) — optional");
                Ok(())
            } else {
                bail!("{bin} not found: {e}");
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn check_macos_gui_session() -> Result<()> {
    let uid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if uid.is_empty() {
        println!("  gui session: WARN (could not detect uid)");
        return Ok(());
    }
    let label = format!("gui/{uid}");
    let out = Command::new("launchctl").args(["print", &label]).output();
    match out {
        Ok(o) if o.status.success() => {
            println!("  gui session: OK ({label})");
            Ok(())
        }
        _ => {
            println!("  gui session: WARN — no {label} (desktop login required for LaunchAgent)");
            println!(
                "               SSH-only? Run the binary manually first, then login at the console."
            );
            Ok(())
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn check_macos_gui_session() -> Result<()> {
    Ok(())
}

fn check_sudo() -> Result<()> {
    let status = Command::new("sudo").args(["-n", "true"]).status();
    match status {
        Ok(s) if s.success() => {
            println!("  sudo:       OK (NOPASSWD)");
            Ok(())
        }
        _ => {
            println!("  sudo:       WARN (password may be required for service install)");
            Ok(())
        }
    }
}
