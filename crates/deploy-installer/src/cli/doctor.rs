use anyhow::{Result, bail};
use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;
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

    // node 仅 npm 包的 JS 启动器（bin/deploy-installer.js）需要：经 npm 运行时 node 必然
    // 存在（JS 本身由 node 执行），原生二进制路径完全不依赖 node —— 缺失只降级为 WARN。
    check_command("node", &["--version"], true)?;
    check_command("curl", &["--version"], false)?;
    check_command("uv", &["--version"], true)?;
    check_ffmpeg();

    if check_bundled_binary("voice-cli").is_err() {
        // Windows 切片可能未携带 voice-cli（构建降级）——降级 WARN 不阻断
        if !cfg!(target_os = "windows") {
            failed = true;
        } else {
            println!(
                "  bundle voice-cli: WARN (not bundled in this Windows build — document-parser only)"
            );
        }
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
    #[cfg(target_os = "linux")]
    {
        if check_linux_syslibs().is_err() {
            failed = true;
        }
    }

    check_install_dir_path(&voice_dir);
    check_install_dir_path(&parser_dir);
    check_disk_space(&voice_dir);
    check_upload_backend(&parser_dir);

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
    } else if cfg!(target_os = "windows") {
        println!("  backend:    task scheduler (per-user S4U logon task)");
        check_windows_task_scheduler()?;
        check_windows_python()?;
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

/// Linux headless 环境的 X11/GL 基础库检查（MinerU/opencv 运行依赖）。
///
/// 缺失时置 FAIL 并打印双发行版安装命令；`ldconfig` 不可用（musl 等）降级为 WARN。
#[cfg(target_os = "linux")]
fn check_linux_syslibs() -> Result<()> {
    let status = crate::checks::check_required_linux_syslibs();
    if status.skipped {
        println!("  syslibs:     WARN (ldconfig unavailable — skipped X11/GL lib check)");
        return Ok(());
    }
    if status.missing.is_empty() {
        println!("  syslibs:     OK (libxcb/libGL/glib resolvable via ldconfig)");
        return Ok(());
    }
    println!(
        "  syslibs:     FAIL — missing: {}",
        status.missing.join(", ")
    );
    println!("{}", crate::checks::linux_syslibs_install_hint());
    Err(anyhow::anyhow!(
        "missing Linux system libs: {}",
        status.missing.join(", ")
    ))
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

/// 上传后端配置检查（WARN 级，不阻断 doctor）：OSS 密钥或 custom_upload 二选一。
///
/// 已安装且配置了 custom_upload 时，顺带对 base_url 做可达性探测——
/// 任何 HTTP 响应（含 4xx/5xx）都算"网络可达"，连接失败/超时仅 WARN
/// （内网 DNS、按需拉起的服务等场景避免误报阻断）。
fn check_upload_backend(parser_dir: &std::path::Path) {
    use crate::cli::env_config::{
        custom_upload_configured, oss_keys_configured, parse_env_file_value,
    };

    let env_path = parser_dir.join(".document-parser.env");
    if !env_path.exists() {
        println!(
            "  upload:     WARN (not installed yet — provide OSS keys or custom upload config at install time)"
        );
        return;
    }

    if oss_keys_configured(&env_path) {
        println!("  upload:     OK (OSS keys configured)");
        return;
    }

    if custom_upload_configured(&env_path) {
        let base_url = parse_env_file_value(&env_path, "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL")
            .unwrap_or_default();
        if probe_url_reachable(&base_url) {
            println!("  upload:     OK (custom backend configured, {base_url} reachable)");
        } else {
            println!(
                "  upload:     WARN (custom backend configured, but {base_url} unreachable — check network/firewall)"
            );
        }
        return;
    }

    println!(
        "  upload:     WARN (no upload backend in {} — set OSS_ACCESS_KEY_* or DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL, pick one)",
        env_path.display()
    );
}

/// 探测 URL 可达性：收到任意 HTTP 状态码（含 4xx/5xx）即 true，连接失败/超时 false。
fn probe_url_reachable(url: &str) -> bool {
    let Ok(output) = std::process::Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--max-time",
            "5",
            url,
        ])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let code = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // 000 = curl 未收到 HTTP 响应（连接层失败）
    !code.is_empty() && code != "000"
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
    // Documents/Desktop/iCloud 警告是 macOS TCC（服务目录访问限制）专属；
    // 其他平台一律 OK（用户目录下默认位置都合法）
    if cfg!(target_os = "macos") {
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
            return;
        }
    }
    println!("  install path {}: OK", path.display());
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
    // Windows：df 不存在，用 PowerShell 查盘符剩余空间
    #[cfg(windows)]
    {
        let path_str = path.to_str()?;
        // "C:\Users\..." → 盘符 "C"（Get-PSDrive -Name 不带冒号）
        let drive = path_str.split(['/', '\\']).next()?.trim_end_matches(':');
        if drive.is_empty() {
            return None;
        }
        let script = format!("(Get-PSDrive -Name '{drive}' -ErrorAction SilentlyContinue).Free");
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        return String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<u64>()
            .ok();
    }
    #[cfg(not(windows))]
    {
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
}

/// Windows：任务计划程序可用性（schtasks 查询 + powershell 探针）。
#[cfg(target_os = "windows")]
fn check_windows_task_scheduler() -> Result<()> {
    let schtasks_ok = Command::new("schtasks")
        .args(["/query", "/fo", "LIST"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let powershell_ok = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$PSVersionTable.PSVersion.Major",
        ])
        .output()
        .is_ok_and(|o| o.status.success());
    match (schtasks_ok, powershell_ok) {
        (true, true) => {
            println!("  scheduler:  OK (schtasks + powershell)");
            Ok(())
        }
        (false, _) => {
            println!("  scheduler:  FAIL (schtasks /query 失败——任务计划程序服务不可用)");
            Err(anyhow::anyhow!("schtasks not usable"))
        }
        (true, false) => {
            println!("  scheduler:  WARN (powershell 不可用：状态/结果探针将降级)");
            Ok(())
        }
    }
}

/// Windows：Python 运行时检查（uv-init 与 venv 依赖）。
#[cfg(target_os = "windows")]
fn check_windows_python() -> Result<()> {
    // py 启动器优先（python.org 安装自带）；失败再试裸 python（注意商店别名陷阱）
    let py_ok = Command::new("py")
        .args(["-3", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if py_ok {
        println!("  python:     OK (py -3)");
        return Ok(());
    }
    let python_ok = Command::new("python")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if python_ok {
        println!("  python:     OK (python)");
        return Ok(());
    }
    println!("  python:     FAIL — 未找到可用的 Python（uv-init 需要）");
    println!("    请从 python.org 安装 3.11–3.13（MinerU 尚不支持 3.14），");
    println!("    并在安装时勾选 Add to PATH；注意 Windows 商店别名不算有效安装。");
    Err(anyhow::anyhow!("no usable Python on Windows"))
}

// 非 Windows 桩：run() 的 cfg!(windows) 分支在所有平台都要能编译
#[cfg(not(target_os = "windows"))]
fn check_windows_task_scheduler() -> Result<()> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn check_windows_python() -> Result<()> {
    Ok(())
}

/// ffmpeg 检查（voice-cli 的 STT 音频解码依赖，WARN 级 + 各平台安装指引）。
///
/// Mac Mini 实测：无 ffmpeg 时 STT 任务失败于"ffmpeg 启动失败 No such file
/// or directory"——提前在 doctor 暴露并给出安装命令，避免用户转录时才发现。
fn check_ffmpeg() {
    let ok = Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        println!("  ffmpeg:     OK (voice-cli STT 音频解码可用)");
        return;
    }
    println!("  ffmpeg:     WARN (缺失——voice-cli 转录任务会失败；仅 document-parser 可忽略)");
    match std::env::consts::OS {
        "macos" => println!("    安装: brew install ffmpeg"),
        "linux" => println!(
            "    安装: sudo apt install ffmpeg  # Debian/Ubuntu；RHEL 系: sudo dnf install ffmpeg"
        ),
        "windows" => {
            println!("    安装: winget install Gyan.FFmpeg  (或从 ffmpeg.org 下载后加入 PATH)")
        }
        _ => {}
    }
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
    if crate::checks::macos_gui_session_present() {
        println!("  gui session: OK ({label})");
    } else {
        println!("  gui session: WARN — no {label} (desktop login required for LaunchAgent)");
        println!(
            "               SSH-only? Run the binary manually first, then login at the console."
        );
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn check_macos_gui_session() -> Result<()> {
    Ok(())
}

fn check_sudo() -> Result<()> {
    // 与 precheck 的 sudo_available 同源探测：daemon-reload 是 install 必经且
    // 幂等无害的命令，可正确识别"仅 systemctl/journalctl NOPASSWD"的最小权限配置
    let status = Command::new("sudo")
        .args(["-n", "systemctl", "daemon-reload"])
        .status();
    match status {
        Ok(s) if s.success() => {
            println!("  sudo:       OK (NOPASSWD systemctl)");
            Ok(())
        }
        _ => {
            println!("  sudo:       WARN (password may be required for service install)");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_command_optional_missing_bin_returns_ok() {
        // optional=true：命令不存在时降级为 WARN，不能让 doctor 直接失败
        assert!(check_command("definitely-missing-xyz", &["--version"], true).is_ok());
    }

    #[test]
    fn check_command_required_missing_bin_returns_err() {
        // optional=false：必需命令（如 curl）缺失时必须硬失败（Fail Fast）
        assert!(check_command("definitely-missing-xyz", &["--version"], false).is_err());
    }
}
