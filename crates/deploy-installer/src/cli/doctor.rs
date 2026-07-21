use anyhow::{Result, bail};
use std::path::Path;
use std::process::Command;

use crate::platform_vendor_key;

pub fn run() -> Result<()> {
    println!("==> deploy-installer doctor");
    println!(
        "  platform:   {} ({})",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("  vendor key: {}", platform_vendor_key());

    check_command("node", &["--version"], false)?;
    check_command("uv", &["--version"], true)?;
    check_command("curl", &["--version"], true)?;

    if cfg!(target_os = "macos") {
        println!("  backend:    launchd (LaunchAgent)");
    } else {
        println!("  backend:    systemd");
        check_sudo()?;
    }

    println!("\n✅ doctor checks passed (warnings above are OK for optional tools)");
    Ok(())
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

#[allow(dead_code)]
fn path_exists(path: &Path) -> bool {
    path.exists()
}
