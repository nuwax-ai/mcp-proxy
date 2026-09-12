use crate::error::{InstallerError, Result};
use std::process::{Command, Stdio};

fn run_sudo(args: &[&str]) -> Result<std::process::Output> {
    let output = Command::new("sudo")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| InstallerError::CommandFailed {
            cmd: format!("sudo {}", args.join(" ")),
            detail: e.to_string(),
        })?;
    Ok(output)
}

fn ensure_success(cmd: &str, output: &std::process::Output) -> Result<()> {
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(InstallerError::CommandFailed {
            cmd: cmd.to_string(),
            detail: format!(
                "status={:?} stderr={stderr} stdout={stdout}",
                output.status.code()
            ),
        })
    }
}

pub fn daemon_reload() -> Result<()> {
    let out = run_sudo(&["systemctl", "daemon-reload"])?;
    ensure_success("sudo systemctl daemon-reload", &out)
}

pub fn enable(name: &str) -> Result<()> {
    let out = run_sudo(&["systemctl", "enable", name])?;
    ensure_success(&format!("sudo systemctl enable {name}"), &out)
}

pub fn disable(name: &str) -> Result<()> {
    let out = run_sudo(&["systemctl", "disable", name])?;
    // disable may fail if not enabled; treat as soft
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("No such file") || stderr.contains("not found") {
            return Ok(());
        }
        return ensure_success(&format!("sudo systemctl disable {name}"), &out);
    }
    Ok(())
}

pub fn start(name: &str) -> Result<()> {
    let out = run_sudo(&["systemctl", "start", name])?;
    ensure_success(&format!("sudo systemctl start {name}"), &out)
}

/// Prefer restart for idempotent install (reload if already running).
pub fn restart(name: &str) -> Result<()> {
    let out = run_sudo(&["systemctl", "restart", name])?;
    ensure_success(&format!("sudo systemctl restart {name}"), &out)
}

pub fn stop(name: &str) -> Result<()> {
    let out = run_sudo(&["systemctl", "stop", name])?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("not loaded") || stderr.contains("not found") {
            return Ok(());
        }
        return ensure_success(&format!("sudo systemctl stop {name}"), &out);
    }
    Ok(())
}

pub fn is_active(name: &str) -> Result<String> {
    let out = run_sudo(&["systemctl", "is-active", name])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn is_enabled(name: &str) -> Result<String> {
    let out = run_sudo(&["systemctl", "is-enabled", name])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn cat_unit(name: &str) -> Result<String> {
    let out = run_sudo(&["systemctl", "cat", name])?;
    if !out.status.success() {
        return ensure_success(&format!("sudo systemctl cat {name}"), &out).map(|_| String::new());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn recent_logs(name: &str, lines: usize) -> Result<String> {
    let n = lines.to_string();
    let out = run_sudo(&["journalctl", "-u", name, "-n", &n, "--no-pager"])?;
    // journalctl may fail on non-systemd; return soft message
    if !out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// `sudo install -m644 -o root -g root <src> <dst>`
pub fn install_file(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    let out = run_sudo(&[
        "install",
        "-m",
        "644",
        "-o",
        "root",
        "-g",
        "root",
        &src.display().to_string(),
        &dst.display().to_string(),
    ])?;
    ensure_success(
        &format!("sudo install {} {}", src.display(), dst.display()),
        &out,
    )
}

pub fn mkdir_p(path: &std::path::Path) -> Result<()> {
    let out = run_sudo(&["mkdir", "-p", &path.display().to_string()])?;
    ensure_success(&format!("sudo mkdir -p {}", path.display()), &out)
}

pub fn remove_file(path: &std::path::Path) -> Result<()> {
    if !path.exists() {
        // May exist only as root-owned; still try rm
    }
    let out = run_sudo(&["rm", "-f", &path.display().to_string()])?;
    ensure_success(&format!("sudo rm -f {}", path.display()), &out)
}

pub fn remove_dir_all(path: &std::path::Path) -> Result<()> {
    let out = run_sudo(&["rm", "-rf", &path.display().to_string()])?;
    ensure_success(&format!("sudo rm -rf {}", path.display()), &out)
}
