//! Cross-platform service lifecycle via [`service-manager`](https://github.com/chipsenkbeil/service-manager-rs).
//!
//! macOS launchd uses user-level LaunchAgents. Linux systemd uses system-level units; when the
//! process is not root, file writes and `systemctl` run through the sudo helpers in [`crate::systemd`].

use crate::error::{InstallerError, Result};
use crate::platform::ServiceBackend;
use crate::render::{render_unit, validate_unit_name};
use crate::render_plist::{ensure_log_dir, ensure_run_server_script, render_launchd_plist};
use crate::spec::{DropIn, ServiceSpec};
use crate::systemd;
use service_manager::{
    RestartPolicy, ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager, ServiceStartCtx,
    ServiceStatus, ServiceStatusCtx, ServiceStopCtx, ServiceUninstallCtx,
};
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const LAUNCHD_PATH: &str = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/Library/Frameworks/Python.framework/Versions/Current/bin";

/// Service label for the active backend (`com.nuwax.*` on launchd, unit basename on systemd).
pub fn service_label(spec: &ServiceSpec, backend: ServiceBackend) -> Result<ServiceLabel> {
    let raw = match backend {
        ServiceBackend::Launchd => spec.launchd_label(),
        ServiceBackend::Systemd => spec.name.clone(),
    };
    raw.parse().map_err(|e: std::io::Error| {
        InstallerError::Other(format!("invalid service label `{raw}`: {e}"))
    })
}

fn map_io(e: std::io::Error) -> InstallerError {
    InstallerError::CommandFailed {
        cmd: "service-manager".into(),
        detail: e.to_string(),
    }
}

fn native_manager(backend: ServiceBackend) -> Result<Box<dyn ServiceManager>> {
    let mut mgr = <dyn ServiceManager>::native().map_err(map_io)?;
    let level = match backend {
        ServiceBackend::Launchd => ServiceLevel::User,
        ServiceBackend::Systemd => ServiceLevel::System,
    };
    mgr.set_level(level).map_err(map_io)?;
    Ok(mgr)
}

fn is_root() -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim() == "0")
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn build_install_ctx(
    spec: &ServiceSpec,
    backend: ServiceBackend,
    contents: Option<String>,
    autostart: bool,
    run_at_load: bool,
) -> Result<ServiceInstallCtx> {
    let label = service_label(spec, backend)?;
    match backend {
        ServiceBackend::Launchd => {
            ensure_run_server_script(spec, false)?;
            ensure_log_dir(spec)?;
            let plist = match contents {
                Some(c) => c,
                None => render_launchd_plist(spec, run_at_load)?,
            };
            Ok(ServiceInstallCtx {
                label,
                program: spec.run_server_script_path(),
                args: vec![],
                contents: Some(plist),
                username: None,
                working_directory: Some(spec.install_dir.clone()),
                environment: Some(vec![("PATH".into(), LAUNCHD_PATH.into())]),
                autostart,
                restart_policy: RestartPolicy::OnFailure { delay_secs: None },
            })
        }
        ServiceBackend::Systemd => {
            let bin = spec
                .exec_start
                .first()
                .ok_or(InstallerError::EmptyExecStart)?;
            let args = spec.exec_start[1..]
                .iter()
                .map(|s| OsString::from(s.as_str()))
                .collect();
            let env = if spec.extra_env.is_empty() {
                None
            } else {
                Some(spec.extra_env.clone())
            };
            let unit = match contents {
                Some(c) => c,
                None => render_unit(spec)?,
            };
            Ok(ServiceInstallCtx {
                label,
                program: PathBuf::from(bin),
                args,
                contents: Some(unit),
                username: Some(spec.identity.user.clone()),
                working_directory: Some(spec.install_dir.clone()),
                environment: env,
                autostart,
                restart_policy: RestartPolicy::OnFailure {
                    delay_secs: Some(5),
                },
            })
        }
    }
}

fn write_dropin_via_sudo(spec: &ServiceSpec, drop_in: &DropIn) -> Result<()> {
    validate_unit_name(&drop_in.name)?;
    let dir = spec.drop_in_dir();
    systemd::mkdir_p(&dir)?;
    let path = dir.join(format!("{}.conf", drop_in.name));
    let mut tmp = tempfile::NamedTempFile::new().map_err(InstallerError::Io)?;
    std::io::Write::write_all(&mut tmp, drop_in.content.as_bytes())?;
    tmp.flush()?;
    systemd::install_file(tmp.path(), &path)?;
    Ok(())
}

fn install_systemd_via_sudo(
    spec: &ServiceSpec,
    ctx: &ServiceInstallCtx,
    start: bool,
) -> Result<()> {
    let unit_text = ctx.contents.as_deref().ok_or_else(|| {
        InstallerError::Other("systemd install requires rendered unit contents".into())
    })?;
    let unit_path = spec.unit_path();
    let mut tmp = tempfile::NamedTempFile::new().map_err(InstallerError::Io)?;
    std::io::Write::write_all(&mut tmp, unit_text.as_bytes())?;
    tmp.flush()?;
    systemd::install_file(tmp.path(), &unit_path)?;
    for d in &spec.drop_ins {
        write_dropin_via_sudo(spec, d)?;
    }
    systemd::daemon_reload()?;
    if ctx.autostart {
        systemd::enable(&spec.name)?;
    }
    if start {
        systemd::restart(&spec.name)?;
    }
    println!("Installed {}.service → {}", spec.name, unit_path.display());
    if ctx.autostart {
        println!("  enabled (WantedBy=multi-user.target)");
    }
    if start {
        println!("  restarted");
    }
    Ok(())
}

/// Install or update a service using the native service manager.
pub fn install_service(spec: &ServiceSpec, dry_run: bool, enable: bool, start: bool) -> Result<()> {
    validate_unit_name(&spec.name)?;
    let backend = crate::platform::current_backend();

    if backend == ServiceBackend::Launchd {
        for d in &spec.drop_ins {
            if !d.content.is_empty() {
                return Err(InstallerError::Other(
                    "launchd backend does not support systemd drop-ins".into(),
                ));
            }
        }
    }

    let ctx = build_install_ctx(spec, backend, None, enable, start)?;

    if dry_run {
        let path = match backend {
            ServiceBackend::Launchd => spec.launchd_plist_path(),
            ServiceBackend::Systemd => spec.unit_path(),
        };
        let body = ctx.contents.as_deref().unwrap_or("");
        println!("--- {} ---\n{body}", path.display());
        for d in &spec.drop_ins {
            println!(
                "--- drop-in {}.service.d/{}.conf ---\n{}",
                spec.name, d.name, d.content
            );
        }
        println!("(dry-run: no files written, service manager not invoked)");
        return Ok(());
    }

    match backend {
        ServiceBackend::Launchd => {
            let mgr = native_manager(backend)?;
            mgr.install(ctx.clone()).map_err(map_io)?;
            if start {
                mgr.start(ServiceStartCtx {
                    label: ctx.label.clone(),
                })
                .map_err(map_io)?;
            }
            println!(
                "Installed LaunchAgent {} → {}",
                spec.launchd_label(),
                spec.launchd_plist_path().display()
            );
            if start {
                println!("  started");
            }
        }
        ServiceBackend::Systemd => {
            if is_root() {
                let mgr = native_manager(backend)?;
                mgr.install(ctx.clone()).map_err(map_io)?;
                if start {
                    mgr.start(ServiceStartCtx {
                        label: ctx.label.clone(),
                    })
                    .map_err(map_io)?;
                }
                println!(
                    "Installed {}.service → {}",
                    spec.name,
                    spec.unit_path().display()
                );
                if enable {
                    println!("  enabled (WantedBy=multi-user.target)");
                }
                if start {
                    println!("  restarted");
                }
            } else {
                install_systemd_via_sudo(spec, &ctx, start)?;
            }
        }
    }
    Ok(())
}

/// Stop, disable, and remove an installed service.
pub fn uninstall_service(spec: &ServiceSpec) -> Result<()> {
    validate_unit_name(&spec.name)?;
    let backend = crate::platform::current_backend();
    let label = service_label(spec, backend)?;

    match backend {
        ServiceBackend::Launchd => {
            let mgr = native_manager(backend)?;
            mgr.uninstall(ServiceUninstallCtx {
                label: label.clone(),
            })
            .map_err(map_io)?;
            println!("Uninstalled LaunchAgent {}", spec.launchd_label());
        }
        ServiceBackend::Systemd => {
            if is_root() {
                let mgr = native_manager(backend)?;
                mgr.uninstall(ServiceUninstallCtx { label })
                    .map_err(map_io)?;
            } else {
                let stop_err = systemd::stop(&spec.name).err();
                let disable_err = systemd::disable(&spec.name).err();
                systemd::remove_file(&spec.unit_path())?;
                let drop_in_dir = spec.drop_in_dir();
                if let Err(e) = systemd::remove_dir_all(&drop_in_dir) {
                    let detail = e.to_string();
                    if !detail.contains("No such file") && !detail.contains("cannot remove") {
                        let _ = systemd::daemon_reload();
                        return Err(e);
                    }
                }
                systemd::daemon_reload()?;
                if let Some(e) = stop_err {
                    println!("  note: stop: {e}");
                }
                if let Some(e) = disable_err {
                    println!("  note: disable: {e}");
                }
            }
            println!("Uninstalled {}.service", spec.name);
        }
    }
    Ok(())
}

/// Restart an installed service (stop then start when needed).
pub fn restart_service(spec: &ServiceSpec) -> Result<()> {
    validate_unit_name(&spec.name)?;
    let backend = crate::platform::current_backend();
    let label = service_label(spec, backend)?;

    match backend {
        ServiceBackend::Launchd => {
            let mgr = native_manager(backend)?;
            let _ = mgr.stop(ServiceStopCtx {
                label: label.clone(),
            });
            mgr.start(ServiceStartCtx { label }).map_err(map_io)?;
            println!("Restarted {}", spec.launchd_label());
        }
        ServiceBackend::Systemd => {
            if is_root() {
                let mgr = native_manager(backend)?;
                let _ = mgr.stop(ServiceStopCtx {
                    label: label.clone(),
                });
                mgr.start(ServiceStartCtx { label }).map_err(map_io)?;
            } else {
                systemd::restart(&spec.name)?;
            }
            println!("Restarted {}", spec.name);
        }
    }
    Ok(())
}

/// Print service state and recent logs.
pub fn status_service(spec: &ServiceSpec) -> Result<()> {
    validate_unit_name(&spec.name)?;
    let backend = crate::platform::current_backend();
    let label = service_label(spec, backend)?;

    println!("Service: {}", spec.name);
    println!(
        "  backend:    {}",
        match backend {
            ServiceBackend::Launchd => "launchd",
            ServiceBackend::Systemd => "systemd",
        }
    );

    match backend {
        ServiceBackend::Launchd => {
            println!("  label:      {}", spec.launchd_label());
            println!("  plist:      {}", spec.launchd_plist_path().display());
            let mgr = native_manager(backend)?;
            match mgr.status(ServiceStatusCtx { label }).map_err(map_io)? {
                ServiceStatus::NotInstalled => println!("  state:      not loaded"),
                ServiceStatus::Running => println!("  state:      running"),
                ServiceStatus::Stopped(reason) => {
                    if let Some(r) = reason {
                        println!("  state:      stopped ({r})");
                    } else {
                        println!("  state:      stopped");
                    }
                }
            }
            print_launchd_plist_and_logs(spec);
        }
        ServiceBackend::Systemd => {
            if is_root() {
                let mgr = native_manager(backend)?;
                match mgr.status(ServiceStatusCtx { label }).map_err(map_io)? {
                    ServiceStatus::NotInstalled => {
                        println!("  is-enabled: not installed");
                        println!("  is-active:  not installed");
                    }
                    ServiceStatus::Running => {
                        println!("  is-enabled: (see unit)");
                        println!("  is-active:  active");
                    }
                    ServiceStatus::Stopped(_) => {
                        println!("  is-enabled: (see unit)");
                        println!("  is-active:  inactive");
                    }
                }
            } else {
                let enabled =
                    systemd::is_enabled(&spec.name).unwrap_or_else(|e| format!("(error: {e})"));
                let active =
                    systemd::is_active(&spec.name).unwrap_or_else(|e| format!("(error: {e})"));
                println!("  is-enabled: {enabled}");
                println!("  is-active:  {active}");
            }
            println!();
            match systemd::cat_unit(&spec.name) {
                Ok(text) => {
                    println!("--- unit ---");
                    println!("{text}");
                }
                Err(e) => println!("(could not cat unit: {e})"),
            }
            println!("--- recent logs ---");
            match systemd::recent_logs(&spec.name, 30) {
                Ok(logs) => println!("{logs}"),
                Err(e) => println!("(could not read journal: {e})"),
            }
        }
    }
    Ok(())
}

fn print_launchd_plist_and_logs(spec: &ServiceSpec) {
    println!();
    if spec.launchd_plist_path().exists() {
        match fs::read_to_string(spec.launchd_plist_path()) {
            Ok(text) => {
                println!("--- plist ---");
                println!("{text}");
            }
            Err(e) => println!("(could not read plist: {e})"),
        }
    }
    let stdout_log = spec.install_dir.join("logs/launchd.stdout.log");
    let stderr_log = spec.install_dir.join("logs/launchd.stderr.log");
    println!("--- recent logs ---");
    for (name, path) in [("stdout", stdout_log), ("stderr", stderr_log)] {
        tail_log_file(name, &path);
    }
}

fn tail_log_file(name: &str, path: &Path) {
    if !path.exists() {
        return;
    }
    if let Ok(content) = fs::read_to_string(path) {
        let tail: String = content
            .lines()
            .rev()
            .take(15)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        println!("({name})\n{tail}");
    }
}

/// Whether the service is loaded/running (for precheck idempotency).
pub fn service_is_active(spec: &ServiceSpec, backend: ServiceBackend) -> bool {
    service_label(spec, backend)
        .ok()
        .and_then(|label| {
            native_manager(backend)
                .ok()?
                .status(ServiceStatusCtx { label })
                .ok()
        })
        .is_some_and(|s| matches!(s, ServiceStatus::Running))
}
