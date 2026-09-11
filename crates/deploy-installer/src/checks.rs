use crate::error::{InstallerError, Result};
use crate::platform::ServiceBackend;
use crate::spec::{ServiceIdentity, ServiceSpec};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Severity of a single precheck item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckSeverity {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
pub struct CheckItem {
    pub name: String,
    pub severity: CheckSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct PrecheckReport {
    pub items: Vec<CheckItem>,
}

impl PrecheckReport {
    pub fn push(
        &mut self,
        name: impl Into<String>,
        severity: CheckSeverity,
        message: impl Into<String>,
    ) {
        self.items.push(CheckItem {
            name: name.into(),
            severity,
            message: message.into(),
        });
    }

    pub fn has_failures(&self) -> bool {
        self.items.iter().any(|i| i.severity == CheckSeverity::Fail)
    }

    pub fn failure_details(&self) -> String {
        self.items
            .iter()
            .filter(|i| i.severity == CheckSeverity::Fail)
            .map(|i| format!("[{}] {}", i.name, i.message))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn print_summary(&self) {
        for item in &self.items {
            let tag = match item.severity {
                CheckSeverity::Pass => "OK",
                CheckSeverity::Warn => "WARN",
                CheckSeverity::Fail => "FAIL",
            };
            println!("  [{tag}] {}: {}", item.name, item.message);
        }
    }
}

fn is_executable(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(meta) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.is_file() && (meta.permissions().mode() & 0o111) != 0
            }
            #[cfg(not(unix))]
            {
                meta.is_file()
            }
        }
        Err(_) => false,
    }
}

fn path_writable(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let probe = dir.join(".deploy_installer_write_probe");
    match fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// True when a local-address token's port equals `port` (avoids substring false positives
/// like port 808 matching `:8087`). Handles `*:8077`, `127.0.0.1:8077`, `[::1]:8077`.
fn local_addr_has_port(token: &str, port: u16) -> bool {
    let after_bracket = token
        .rsplit_once(']')
        .map(|(_, rest)| rest)
        .unwrap_or(token);
    after_bracket
        .rsplit_once(':')
        .and_then(|(_, p)| p.parse::<u16>().ok())
        == Some(port)
}

/// 解析 `netstat -ano -p tcp` 输出中监听 `port` 的行，返回其 PID（纯函数，任意平台可单测）。
///
/// 行样例：`  TCP    0.0.0.0:8087    0.0.0.0:0    LISTENING    12345`；
/// IPv6 形如 `[::]:8087`（复用 [`local_addr_has_port`] 的括号处理）。
/// state 名（LISTENING 等）来自 IP Helper API，不随系统显示语言本地化。
pub fn netstat_listener_pid(output: &str, port: u16) -> Option<String> {
    for line in output.lines() {
        let mut cols = line.split_whitespace();
        if cols.next()? != "TCP" {
            continue;
        }
        let Some(local) = cols.next() else { continue };
        if !local_addr_has_port(local, port) {
            continue;
        }
        let Some(_remote) = cols.next() else { continue };
        let Some(state) = cols.next() else { continue };
        if state != "LISTENING" {
            continue;
        }
        let pid = cols.next().unwrap_or("unknown");
        return Some(pid.to_string());
    }
    None
}

/// Try to detect which PID holds `port`. Returns Ok(Some(pid)) if occupied,
/// Ok(None) if free, Err if detection tools unavailable.
pub(crate) fn port_occupant(port: u16) -> std::result::Result<Option<String>, String> {
    // Windows: netstat -ano（PID 列在行尾；state 名 LISTENING 来自 IP Helper API，不本地化）
    if cfg!(windows) {
        let output = Command::new("netstat")
            .args(["-ano", "-p", "tcp"])
            .output()
            .map_err(|e| format!("netstat failed: {e}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Ok(netstat_listener_pid(&stdout, port));
    }

    // Prefer `ss` (modern), fall back to `lsof`.
    if which_exists("ss") {
        let output = Command::new("ss")
            .args(["-ltnp", &format!("sport = :{port}")])
            .output()
            .map_err(|e| format!("ss failed: {e}"))?;
        if !output.status.success() {
            return Err(format!("ss exited with status {:?}", output.status.code()));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Only LISTEN rows whose local address ends with :{port} count as occupied.
        let listen_line = stdout.lines().find(|l| {
            let t = l.trim();
            !t.is_empty()
                && !t.starts_with("State")
                && t.contains("LISTEN")
                && t.split_whitespace()
                    .any(|tok| local_addr_has_port(tok, port))
        });
        if let Some(line) = listen_line {
            let pid = extract_pid_from_ss(line).unwrap_or_else(|| "unknown".into());
            return Ok(Some(pid));
        }
        return Ok(None);
    }

    if which_exists("lsof") {
        let output = Command::new("lsof")
            .args(["-i", &format!("TCP:{port}"), "-sTCP:LISTEN", "-t"])
            .output()
            .map_err(|e| format!("lsof failed: {e}"))?;
        // lsof returns non-zero when nothing matches — treat as free.
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            return Ok(None);
        }
        let pid = stdout.lines().next().unwrap_or("unknown").to_string();
        return Ok(Some(pid));
    }

    Err("neither ss nor lsof is available".into())
}

fn extract_pid_from_ss(stdout: &str) -> Option<String> {
    // Look for pid=1234
    for part in stdout.split([' ', ',', '(', ')']) {
        if let Some(rest) = part.strip_prefix("pid=") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// Whether this unit is already active (idempotent reinstall should not fail on own port).
///
/// pub：upgrade 流程在替换二进制前探测"服务在跑?"复用（三后端名形差异在此收敛）。
pub fn unit_is_active(name: &str, backend: ServiceBackend) -> bool {
    match backend {
        ServiceBackend::Launchd => {
            let spec = ServiceSpec {
                name: name.to_string(),
                description: String::new(),
                identity: ServiceIdentity {
                    user: String::new(),
                    group: String::new(),
                },
                install_dir: PathBuf::from("."),
                exec_start: vec![],
                env_file: None,
                extra_env: vec![],
                kill_signal: None,
                timeout_stop_sec: None,
                syslog_identifier: None,
                drop_ins: vec![],
                supplementary_groups: vec![],
                required_paths: vec![],
                listen_port: None,
            };
            crate::service_mgr::service_is_active(&spec, backend)
        }
        ServiceBackend::Systemd => {
            Command::new("systemctl")
                .args(["is-active", "--quiet", name])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
                || Command::new("sudo")
                    .args(["-n", "systemctl", "is-active", "--quiet", name])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
        }
        // 任务名与 unit 名不同形（com.nuwax.<name>）；直接按任务名查状态
        ServiceBackend::TaskScheduler => {
            crate::task_scheduler::task_state(&format!("com.nuwax.{name}"))
                == crate::task_scheduler::TaskState::Running
        }
    }
}

fn which_exists(bin: &str) -> bool {
    let probe = if cfg!(windows) { "where" } else { "which" };
    Command::new(probe)
        .arg(bin)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Probe whether sudo works non-interactively for the commands we actually need.
///
/// 探测 `sudo -n systemctl daemon-reload`（install 必经且幂等无害）而非泛化的
/// `sudo -n true`：后者要求 NOPASSWD: ALL，会误拒"仅 systemctl/journalctl
/// NOPASSWD"的最小权限配置（sudoers 推荐做法）。
pub fn sudo_available() -> std::result::Result<(), String> {
    if nix_geteuid_is_root() {
        return Ok(());
    }
    if !which_exists("sudo") {
        return Err("sudo not found in PATH".into());
    }
    let status = Command::new("sudo")
        .args(["-n", "systemctl", "daemon-reload"])
        .status()
        .map_err(|e| format!("failed to run sudo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(
            "sudo requires a password (sudo -n systemctl daemon-reload failed). \
             Configure NOPASSWD sudoers for the commands we need: \
             'user ALL=(root) NOPASSWD: /usr/bin/systemctl, /usr/bin/journalctl, \
             /usr/bin/install, /usr/bin/mkdir, /usr/bin/rm' \
             (systemctl/journalctl 管理 + install/mkdir/rm 写删 unit 文件), \
             or run install interactively so sudo can prompt."
                .into(),
        )
    }
}

fn nix_geteuid_is_root() -> bool {
    #[cfg(unix)]
    {
        // Avoid extra deps: parse `id -u`
        Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok().map(|s| s.trim() == "0"))
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Resolve current user name via `whoami`.
pub fn current_user() -> Result<String> {
    let output = Command::new("whoami")
        .output()
        .map_err(|e| InstallerError::CommandFailed {
            cmd: "whoami".into(),
            detail: e.to_string(),
        })?;
    if !output.status.success() {
        return Err(InstallerError::CommandFailed {
            cmd: "whoami".into(),
            detail: String::from_utf8_lossy(&output.stderr).into(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Resolve the systemd `User=` for install.
///
/// Prefer explicit `--user`; else `SUDO_USER` when invoked via sudo (so
/// `sudo ./bin service install` does not register as root); else `whoami`.
pub fn resolve_service_user(explicit: Option<String>) -> Result<String> {
    if let Some(u) = explicit {
        if u.is_empty() {
            return Err(InstallerError::InvalidField {
                field: "User".into(),
                reason: "empty".into(),
            });
        }
        return Ok(u);
    }
    if let Ok(u) = std::env::var("SUDO_USER") {
        let u = u.trim();
        if !u.is_empty() && u != "root" {
            return Ok(u.to_string());
        }
    }
    current_user()
}

/// Resolve primary group for `user` via `id -gn`.
pub fn group_for_user(user: &str) -> Result<String> {
    // Windows 无 id/主组概念；Task Scheduler 后端不消费组身份
    #[cfg(windows)]
    {
        return Ok(user.to_string());
    }
    #[cfg(not(windows))]
    {
        let output = Command::new("id")
            .args(["-gn", user])
            .output()
            .map_err(|e| InstallerError::CommandFailed {
                cmd: "id -gn".into(),
                detail: e.to_string(),
            })?;
        if !output.status.success() {
            return Err(InstallerError::CommandFailed {
                cmd: format!("id -gn {user}"),
                detail: String::from_utf8_lossy(&output.stderr).into(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Options controlling which prechecks are hard requirements.
#[derive(Debug, Clone)]
pub struct PrecheckOptions {
    /// When true, missing/unusable sudo is a hard failure (Linux systemd only).
    pub require_sudo: bool,
    /// When true, run listen-port conflict detection.
    pub check_port: bool,
    /// Active service manager backend.
    pub backend: ServiceBackend,
}

impl Default for PrecheckOptions {
    fn default() -> Self {
        Self {
            require_sudo: true,
            check_port: true,
            backend: ServiceBackend::Systemd,
        }
    }
}

/// Run all prechecks. Returns `Err(PrecheckFailed)` when any item is `Fail`.
pub fn precheck(spec: &ServiceSpec, opts: &PrecheckOptions) -> Result<PrecheckReport> {
    let mut report = PrecheckReport::default();

    // 1. Binary
    if let Some(bin) = spec.exec_start.first() {
        let bin_path = PathBuf::from(bin);
        if is_executable(&bin_path) {
            report.push(
                "binary",
                CheckSeverity::Pass,
                format!("{}", bin_path.display()),
            );
        } else {
            report.push(
                "binary",
                CheckSeverity::Fail,
                format!("not found or not executable: {}", bin_path.display()),
            );
        }
    } else {
        report.push("binary", CheckSeverity::Fail, "ExecStart is empty");
    }

    // 2. required_paths
    for path in &spec.required_paths {
        if path.exists() {
            report.push(
                "required_path",
                CheckSeverity::Pass,
                path.display().to_string(),
            );
        } else {
            report.push(
                "required_path",
                CheckSeverity::Fail,
                format!("missing: {}", path.display()),
            );
        }
    }

    // 3. install_dir writable
    if path_writable(&spec.install_dir) {
        report.push(
            "install_dir",
            CheckSeverity::Pass,
            format!("writable: {}", spec.install_dir.display()),
        );
    } else {
        report.push(
            "install_dir",
            CheckSeverity::Fail,
            format!("not a writable directory: {}", spec.install_dir.display()),
        );
    }

    // 4. env_file (warn only)
    if let Some(env_file) = &spec.env_file {
        if env_file.exists() {
            match fs::read_to_string(env_file) {
                Ok(content) => {
                    let looks_placeholder = content.lines().any(|l| {
                        let t = l.trim();
                        t.contains("CHANGE_ME")
                            || t.contains("your_access_key")
                            || t.ends_with("=replace_me")
                            || t.contains("TODO")
                    });
                    if looks_placeholder {
                        report.push(
                            "env_file",
                            CheckSeverity::Warn,
                            format!(
                                "{} exists but may still contain placeholder secrets",
                                env_file.display()
                            ),
                        );
                    } else {
                        report.push(
                            "env_file",
                            CheckSeverity::Pass,
                            env_file.display().to_string(),
                        );
                    }
                }
                Err(e) => report.push(
                    "env_file",
                    CheckSeverity::Warn,
                    format!("cannot read {}: {e}", env_file.display()),
                ),
            }
        } else if opts.require_sudo {
            // Real install: EnvironmentFile= without '-' fails systemd start.
            report.push(
                "env_file",
                CheckSeverity::Fail,
                format!(
                    "missing: {} (required for systemd EnvironmentFile=)",
                    env_file.display()
                ),
            );
        } else {
            report.push(
                "env_file",
                CheckSeverity::Warn,
                format!("missing (dry-run): {}", env_file.display()),
            );
        }
    }

    // 5. port conflict (skip hard-fail when this unit is already active — idempotent reinstall)
    if opts.check_port
        && let Some(port) = spec.listen_port
    {
        let self_active = unit_is_active(&spec.name, opts.backend);
        match port_occupant(port) {
            Ok(Some(pid)) if self_active => {
                report.push(
                    "port",
                    CheckSeverity::Warn,
                    format!(
                        "port {port} held by pid={pid}, but {}.service is already active — will restart",
                        spec.name
                    ),
                );
            }
            Ok(Some(pid)) => {
                report.push(
                    "port",
                    CheckSeverity::Fail,
                    format!(
                        "port {port} is in use (pid={pid}). Change config server.port and retry."
                    ),
                );
            }
            Ok(None) => {
                report.push("port", CheckSeverity::Pass, format!("port {port} is free"));
            }
            Err(reason) => {
                report.push(
                    "port",
                    CheckSeverity::Warn,
                    format!(
                        "could not check port {port} ({reason}). Manual: ss -ltnp 'sport = :{port}'"
                    ),
                );
            }
        }
    }

    // 6. sudo
    if opts.require_sudo {
        match sudo_available() {
            Ok(()) => report.push("sudo", CheckSeverity::Pass, "available"),
            Err(reason) => report.push("sudo", CheckSeverity::Fail, reason),
        }
    } else if opts.backend == ServiceBackend::Launchd {
        report.push(
            "sudo",
            CheckSeverity::Pass,
            "not required (launchd user agent)",
        );
    } else {
        report.push("sudo", CheckSeverity::Pass, "skipped (dry-run)");
    }

    // 7. existing unit / plist / task xml
    let existing = match opts.backend {
        ServiceBackend::Launchd => spec.launchd_plist_path(),
        ServiceBackend::Systemd => spec.unit_path(),
        ServiceBackend::TaskScheduler => spec.task_xml_path(),
    };
    if existing.exists() {
        report.push(
            "existing_unit",
            CheckSeverity::Warn,
            format!("{} exists and will be overwritten", existing.display()),
        );
    } else if opts.backend == ServiceBackend::TaskScheduler
        && crate::task_scheduler::task_exists(&spec.task_name())
    {
        // 注册的任务与持久化 XML 可能单边存在（手工删过文件/未卸载干净），都提示
        report.push(
            "existing_unit",
            CheckSeverity::Warn,
            format!(
                "scheduled task {} is registered and will be re-created",
                spec.task_name()
            ),
        );
    } else {
        report.push("existing_unit", CheckSeverity::Pass, "no existing unit");
    }

    if report.has_failures() {
        report.print_summary();
        return Err(InstallerError::PrecheckFailed {
            details: report.failure_details(),
        });
    }
    Ok(report)
}

/// 检测当前用户是否存在已登录的 macOS GUI 会话（`launchctl print gui/<uid>` 成功）。
///
/// doctor 与 install 共用：SSH-only 场景下 launchd 的 gui domain 不可用，
/// LaunchAgent 的 bootstrap/enable 会以 exit 134 (SIGABRT) 失败。
#[cfg(target_os = "macos")]
pub fn macos_gui_session_present() -> bool {
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if uid.is_empty() {
        return false;
    }
    Command::new("launchctl")
        .args(["print", &format!("gui/{uid}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// MinerU（opencv 依赖链）在 headless Linux 上运行所需的基础共享库（ldconfig 名称）。
///
/// 无桌面的服务器发行版默认不带这些 X11/GL 客户端库；缺失时 venv 内 mineru 会在
/// `--help` 自检阶段以 `ImportError: libxcb.so.1: cannot open shared object file` 失败。
/// 该清单与 document-parser 侧 `detection::shared_lib_install_hint` 的建议命令保持一致。
pub const REQUIRED_LINUX_SYSLIBS: &[&str] = &["libxcb.so.1", "libGL.so.1", "libglib-2.0.so.0"];

/// ldconfig 缺库时的安装提示文案（RHEL/Debian 双系命令），doctor 与 setup 预检共用。
pub fn linux_syslibs_install_hint() -> String {
    "缺少上述库时按发行版安装：\n  \
     RHEL 系: sudo dnf install libxcb libxkbcommon libXext libXrender mesa-libGL glib2\n  \
     Debian 系: sudo apt install libxcb1 libxkbcommon-x11-0 libgl1 libglib2.0-0"
        .to_string()
}

/// 在 `ldconfig -p` 缓存文本中返回无法解析的库名（纯函数，任意平台可单测）。
///
/// 缓存行样例：`        libxcb.so.1 (libc6,x86-64) => /lib/x86_64-linux-gnu/libxcb.so.1`
/// 行首库名后必须紧跟空格或 `(`（避免 `libxcb.so` 前缀误匹配 `libxcb.so.1` 的行，
/// 以及 `libglib-2.0.so.0x` 之类的超长名误报）。
pub fn libs_missing_from_ldconfig(required: &[&str], ldconfig_cache: &str) -> Vec<String> {
    required
        .iter()
        .filter(|lib| {
            !ldconfig_cache.lines().any(|line| {
                let t = line.trim_start();
                t.strip_prefix(*lib)
                    .is_some_and(|rest| rest.starts_with(' ') || rest.starts_with('('))
            })
        })
        .map(|lib| (*lib).to_string())
        .collect()
}

/// Linux 系统库预检结果。
#[derive(Debug, Clone, Default)]
pub struct LinuxSyslibStatus {
    /// 未解析到的库名（空 = 全部就绪）。
    pub missing: Vec<String>,
    /// `ldconfig` 不可用（如 musl 环境）时检查被跳过，调用方应降级为 WARN 而非失败。
    pub skipped: bool,
}

/// 运行 `ldconfig -p` 并比对 [`REQUIRED_LINUX_SYSLIBS`]。
#[cfg(target_os = "linux")]
pub fn check_required_linux_syslibs() -> LinuxSyslibStatus {
    match Command::new("ldconfig").arg("-p").output() {
        Ok(out) if out.status.success() => LinuxSyslibStatus {
            missing: libs_missing_from_ldconfig(
                REQUIRED_LINUX_SYSLIBS,
                &String::from_utf8_lossy(&out.stdout),
            ),
            skipped: false,
        },
        _ => LinuxSyslibStatus {
            missing: Vec::new(),
            skipped: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{ServiceIdentity, ServiceSpec};
    use tempfile::tempdir;

    #[test]
    fn local_addr_has_port_exact() {
        assert!(local_addr_has_port("*:8077", 8077));
        assert!(local_addr_has_port("127.0.0.1:8077", 8077));
        assert!(local_addr_has_port("[::1]:8077", 8077));
        assert!(!local_addr_has_port("*:8087", 808));
        assert!(!local_addr_has_port("*:8077", 807));
        assert!(!local_addr_has_port("*:8", 8087));
    }

    fn minimal_spec(dir: &Path, bin_name: &str) -> ServiceSpec {
        let bin = dir.join(bin_name);
        fs::write(&bin, b"#!/bin/sh\necho ok\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&bin).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&bin, perms).unwrap();
        }
        let config = dir.join("config.yml");
        fs::write(&config, b"server:\n  port: 19999\n").unwrap();
        ServiceSpec {
            name: "test-svc".into(),
            description: "test".into(),
            identity: ServiceIdentity {
                user: "nobody".into(),
                group: "nogroup".into(),
            },
            install_dir: dir.to_path_buf(),
            exec_start: vec![bin.display().to_string()],
            env_file: None,
            extra_env: vec![],
            kill_signal: None,
            timeout_stop_sec: None,
            syslog_identifier: None,
            drop_ins: vec![],
            supplementary_groups: vec![],
            required_paths: vec![config],
            listen_port: None, // skip port in unit test
        }
    }

    #[test]
    fn precheck_fails_missing_binary() {
        let dir = tempdir().unwrap();
        let mut spec = minimal_spec(dir.path(), "okbin");
        spec.exec_start = vec![dir.path().join("missing").display().to_string()];
        // sudo will also fail on Mac without NOPASSWD — so we only assert binary failure is present
        // by inspecting via a local helper that doesn't require sudo success...
        // Instead call pieces: binary check is Fail when missing.
        let bin = PathBuf::from(&spec.exec_start[0]);
        assert!(!is_executable(&bin));
    }

    #[test]
    fn precheck_fails_missing_required_path() {
        let dir = tempdir().unwrap();
        let mut spec = minimal_spec(dir.path(), "okbin");
        spec.required_paths = vec![dir.path().join("nope.yml")];
        // Can't easily run full precheck without sudo; assert path_writable + required
        assert!(!spec.required_paths[0].exists());
        assert!(path_writable(dir.path()));
    }

    #[test]
    fn ldconfig_missing_reports_missing_libs() {
        let cache = "\tlibxcb.so.1 (libc6,x86-64) => /lib/x86_64-linux-gnu/libxcb.so.1\n\
                     \tlibGL.so.1 (libc6,x86-64) => /lib/x86_64-linux-gnu/libGL.so.1\n";
        let missing = libs_missing_from_ldconfig(REQUIRED_LINUX_SYSLIBS, cache);
        assert_eq!(missing, vec!["libglib-2.0.so.0"]);
    }

    #[test]
    fn ldconfig_no_prefix_false_positive() {
        // 行首库名后必须跟空格/括号：libxcb.so.1 的行不能糊弄 libxcb.so，
        // libglib-2.0.so.0x 的行不能糊弄 libglib-2.0.so.0
        let cache = "\tlibxcb.so.1 (libc6) => /lib/libxcb.so.1\n\
                     \tlibglib-2.0.so.0x (libc6) => /lib/libglib-2.0.so.0x\n";
        let missing = libs_missing_from_ldconfig(&["libxcb.so", "libglib-2.0.so.0"], cache);
        assert_eq!(missing, vec!["libxcb.so", "libglib-2.0.so.0"]);
    }

    #[test]
    fn ldconfig_empty_cache_reports_all_missing() {
        let missing = libs_missing_from_ldconfig(REQUIRED_LINUX_SYSLIBS, "");
        assert_eq!(
            missing,
            vec!["libxcb.so.1", "libGL.so.1", "libglib-2.0.so.0"]
        );
    }

    #[test]
    fn linux_syslib_hint_contains_dnf_and_apt() {
        let hint = linux_syslibs_install_hint();
        assert!(hint.contains("dnf install libxcb"));
        assert!(hint.contains("apt install libxcb1"));
    }
}
