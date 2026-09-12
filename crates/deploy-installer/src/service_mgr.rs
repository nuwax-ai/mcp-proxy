//! Cross-platform service lifecycle via [`service-manager`](https://github.com/chipsenkbeil/service-manager-rs).
//!
//! macOS launchd uses user-level LaunchAgents. Linux systemd uses system-level units; when the
//! process is not root, file writes and `systemctl` run through the sudo helpers in [`crate::systemd`].
//! Windows Task Scheduler goes through the schtasks wrapper in [`crate::task_scheduler`]
//! (per-user S4U logon-triggered task; no service-manager crate involvement).

use crate::error::{InstallerError, Result};
use crate::platform::ServiceBackend;
use crate::render::{render_unit, validate_unit_name};
use crate::render_plist::{ensure_log_dir, render_launchd_plist};
use crate::service_task;
use crate::service_task::tail_log_file;
use crate::spec::{DropIn, ServiceSpec};
use crate::systemd;
use service_manager::{
    RestartPolicy, ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager, ServiceStartCtx,
    ServiceStatus, ServiceStatusCtx, ServiceStopCtx, ServiceUninstallCtx,
};
use std::fs;
use std::io::Write;

const LAUNCHD_PATH: &str = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/Library/Frameworks/Python.framework/Versions/Current/bin";

/// Service label for the active backend (`com.nuwax.*` on launchd, unit basename on systemd).
pub fn service_label(spec: &ServiceSpec, backend: ServiceBackend) -> Result<ServiceLabel> {
    let raw = match backend {
        ServiceBackend::Launchd => spec.launchd_label(),
        ServiceBackend::Systemd => spec.name.clone(),
        // 任务计划程序与 launchd 同名形状；不走 service-manager，此值仅用于展示
        ServiceBackend::TaskScheduler => spec.task_name(),
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
        // 任务计划程序的生命周期经 task_scheduler 模块，不经 service-manager
        ServiceBackend::TaskScheduler => {
            return Err(InstallerError::Other(
                "task scheduler backend does not use service-manager".into(),
            ));
        }
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
            ensure_log_dir(spec)?;
            let plist = match contents {
                Some(c) => c,
                None => render_launchd_plist(spec, run_at_load)?,
            };
            let (program, args) = crate::exec_argv::program_and_args(spec);
            let mut environment = vec![
                ("PATH".into(), LAUNCHD_PATH.into()),
                (
                    "HOME".into(),
                    std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()),
                ),
                (
                    "TMPDIR".into(),
                    std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into()),
                ),
            ];
            for (k, v) in &spec.extra_env {
                if environment.iter().any(|(ek, _)| ek == k) {
                    continue;
                }
                environment.push((k.clone(), v.clone()));
            }
            Ok(ServiceInstallCtx {
                label,
                program,
                args,
                contents: Some(plist),
                username: None,
                working_directory: Some(spec.install_dir.clone()),
                environment: Some(environment),
                autostart,
                restart_policy: RestartPolicy::OnFailure {
                    delay_secs: None,
                    max_retries: None,
                    reset_after_secs: None,
                },
            })
        }
        ServiceBackend::Systemd => {
            let (program, args) = crate::exec_argv::program_and_args(spec);
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
                program,
                args,
                contents: Some(unit),
                username: Some(spec.identity.user.clone()),
                working_directory: Some(spec.install_dir.clone()),
                environment: env,
                autostart,
                restart_policy: RestartPolicy::OnFailure {
                    delay_secs: Some(5),
                    max_retries: None,
                    reset_after_secs: None,
                },
            })
        }
        // 任务计划程序不构造 service-manager ctx——生命周期在 install_service 的
        // TaskScheduler 分支直接经 task_scheduler 模块完成，此臂不可达
        ServiceBackend::TaskScheduler => Err(InstallerError::Other(
            "task scheduler backend does not use ServiceInstallCtx".into(),
        )),
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

    if backend != ServiceBackend::Systemd {
        for d in &spec.drop_ins {
            if !d.content.is_empty() {
                return Err(InstallerError::Other(
                    "launchd/task-scheduler backends do not support systemd drop-ins".into(),
                ));
            }
        }
    }

    // 任务计划程序的安装链路独立于 service-manager（无 ServiceInstallCtx）
    if backend == ServiceBackend::TaskScheduler {
        return service_task::install_task(spec, dry_run, enable, start);
    }

    let ctx = build_install_ctx(spec, backend, None, enable, start)?;

    if dry_run {
        let path = match backend {
            ServiceBackend::Launchd => spec.launchd_plist_path(),
            ServiceBackend::Systemd => spec.unit_path(),
            ServiceBackend::TaskScheduler => spec.task_xml_path(),
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
        // 已在函数开头分流处理；此处防御性兜底
        ServiceBackend::TaskScheduler => {
            return Err(InstallerError::Other(
                "task scheduler branch must be handled before ctx construction".into(),
            ));
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
                // service-manager 的 uninstall 只 disable + 删 unit 文件，不 stop——
                // 先显式停止，否则运行中的进程变孤儿并继续占用端口（Linux 重装场景
                // 实测：旧进程带着已删除的 CWD 存活，顶替新服务的端口）。
                // systemctl stop 幂等：unit 未加载/未运行时同样返回成功。
                if let Err(e) = systemd::stop(&spec.name) {
                    println!("  note: stop before uninstall: {e}");
                }
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
        ServiceBackend::TaskScheduler => service_task::uninstall_task(spec)?,
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
            // stop 后等旧实例真正退出（端口释放）——否则 bootstrap 的新实例
            // bind 失败而死，且随后的健康验证会被垂死旧实例的响应骗过
            //（本地实测：restart 打印 Restarted 但实际新旧交替全死再被兜底）
            if let Some(port) = spec.listen_port {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
                while matches!(crate::checks::port_occupant(port), Ok(Some(_)))
                    && std::time::Instant::now() < deadline
                {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
            mgr.start(ServiceStartCtx {
                label: label.clone(),
            })
            .map_err(map_io)?;
            // 终态验证（93 实测三次）：bootstrap 可能静默未生效
            //（服务不拉起，须手动 kickstart 才活）——只认 curl /health 通；
            // 未通自动 kickstart 重试一次，仍失败如实报错而非打印假成功
            if let Some(port) = spec.listen_port
                && !crate::cli::common::wait_for_health(port, "/health", 30)
            {
                println!("  note: health not up after start — kickstart retry");
                let uid = std::process::Command::new("id")
                    .arg("-u")
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                let _ = std::process::Command::new("launchctl")
                    .args(["kickstart", "-k", &format!("gui/{uid}/{label}")])
                    .status();
                if !crate::cli::common::wait_for_health(port, "/health", 30) {
                    return Err(InstallerError::Other(format!(
                        "launchd job {} restarted but /health never responded on port {port}",
                        spec.launchd_label()
                    )));
                }
            }
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
        ServiceBackend::TaskScheduler => service_task::restart_task(spec)?,
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
            ServiceBackend::TaskScheduler => "task scheduler",
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
        ServiceBackend::TaskScheduler => service_task::status_task(spec)?,
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

/// Whether the service is loaded/running (for precheck idempotency).
pub fn service_is_active(spec: &ServiceSpec, backend: ServiceBackend) -> bool {
    if backend == ServiceBackend::TaskScheduler {
        return crate::task_scheduler::task_state(&spec.task_name())
            == crate::task_scheduler::TaskState::Running;
    }
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
