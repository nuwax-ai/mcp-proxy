use crate::checks::{self, PrecheckOptions, PrecheckReport};
use crate::error::{InstallerError, Result};
use crate::platform::{ServiceBackend, current_backend};
use crate::render::validate_unit_name;
use crate::service_mgr;
use crate::spec::{ServiceIdentity, ServiceSpec};
use std::fs;
use std::path::PathBuf;

/// Options for [`install`].
#[derive(Debug, Clone)]
pub struct InstallOptions {
    /// Register for boot/login autostart (systemd enable / launchd RunAtLoad).
    pub enable: bool,
    /// Start or restart after install.
    pub start: bool,
    /// Only render and print unit/plist text; skip writes and service manager calls.
    pub dry_run: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            enable: true,
            start: true,
            dry_run: false,
        }
    }
}

fn precheck_options(opts: &InstallOptions) -> PrecheckOptions {
    let backend = current_backend();
    PrecheckOptions {
        require_sudo: backend == ServiceBackend::Systemd && !opts.dry_run,
        check_port: !opts.dry_run,
        backend,
    }
}

/// Install (or update) a service unit from `spec`.
pub fn install(spec: &ServiceSpec, opts: &InstallOptions) -> Result<PrecheckReport> {
    validate_unit_name(&spec.name)?;

    if opts.start && !opts.enable {
        return Err(InstallerError::Other(
            "invalid InstallOptions: start=true requires enable=true".into(),
        ));
    }

    let report = checks::precheck(spec, &precheck_options(opts))?;
    report.print_summary();

    service_mgr::install_service(spec, opts.dry_run, opts.enable, opts.start)?;
    Ok(report)
}

/// Stop, disable, remove unit/plist.
pub fn uninstall(name: &str) -> Result<()> {
    uninstall_in_dir(name, None)
}

/// Stop, disable, remove unit/plist using `install_dir` for launchd paths.
pub fn uninstall_in_dir(name: &str, install_dir: Option<PathBuf>) -> Result<()> {
    validate_unit_name(name)?;
    let spec = spec_for_name(name, install_dir)?;
    service_mgr::uninstall_service(&spec)
}

/// Print service state, unit/plist, and recent logs.
pub fn status(name: &str) -> Result<()> {
    status_in_dir(name, None)
}

/// Print service state using `install_dir` for launchd log paths.
pub fn status_in_dir(name: &str, install_dir: Option<PathBuf>) -> Result<()> {
    validate_unit_name(name)?;
    let spec = spec_for_name(name, install_dir)?;
    service_mgr::status_service(&spec)
}

/// Restart an already-installed service.
pub fn restart(name: &str) -> Result<()> {
    restart_in_dir(name, None)
}

/// Restart using `install_dir` for launchd.
pub fn restart_in_dir(name: &str, install_dir: Option<PathBuf>) -> Result<()> {
    validate_unit_name(name)?;
    let spec = spec_for_name(name, install_dir)?;
    service_mgr::restart_service(&spec)
}

fn spec_for_name(name: &str, install_dir: Option<PathBuf>) -> Result<ServiceSpec> {
    let install_dir = install_dir
        .or_else(|| std::env::var("NUWAX_INSTALL_DIR").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(ServiceSpec {
        name: name.to_string(),
        description: name.to_string(),
        identity: crate::checks::resolve_service_user(None).map(|user| {
            let group = crate::checks::group_for_user(&user).unwrap_or_else(|_| user.clone());
            ServiceIdentity { user, group }
        })?,
        install_dir,
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
    })
}

/// Helper used by callers that only need to know if a path exists.
pub fn path_exists(path: &std::path::Path) -> bool {
    path.exists()
}

/// Write UTF-8 content to a path (no sudo; for config / .env bootstrap in install_dir).
pub fn write_user_file(path: &std::path::Path, content: &str, mode: Option<u32>) -> Result<()> {
    // Windows 无 POSIX mode；参数保留以维持跨平台调用签名一致
    #[cfg(not(unix))]
    let _ = mode;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    #[cfg(unix)]
    if let Some(m) = mode {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(m);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}
