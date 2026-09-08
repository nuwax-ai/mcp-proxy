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
use crate::render_task::render_task_xml;
use crate::spec::{DropIn, ServiceSpec};
use crate::systemd;
use crate::task_scheduler;
use service_manager::{
    RestartPolicy, ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager, ServiceStartCtx,
    ServiceStatus, ServiceStatusCtx, ServiceStopCtx, ServiceUninstallCtx,
};
use std::fs;
use std::io::Write;
use std::path::Path;

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
        return install_task(spec, dry_run, enable, start);
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

/// 任务计划程序安装链路：渲染 XML → 持久化到 install_dir → 停旧实例（尽力）
/// → 注册 → 启动。
///
/// S4U 主体（无需提权注册、无桌面登录也运行）；注册失败时由上层决定是否用
/// 无 Principal 的降级 XML 重试（Interactive，仅登录运行）。
fn install_task(spec: &ServiceSpec, dry_run: bool, _enable: bool, start: bool) -> Result<()> {
    let xml = render_task_xml(spec, true)?;
    let xml_path = spec.task_xml_path();
    if dry_run {
        println!("--- {} ---\n{xml}", xml_path.display());
        println!("(dry-run: no files written, service manager not invoked)");
        return Ok(());
    }
    crate::installer::write_user_file(&xml_path, &xml, None)?;

    let name = spec.task_name();
    if task_scheduler::task_exists(&name) {
        // 已注册则先结束运行实例，让新定义下次启动即生效
        if let Err(e) = task_scheduler::end(&name) {
            println!("  note: end previous task instance: {e}");
        }
    }
    task_scheduler::create_from_xml(&name, &xml_path)?;
    if start {
        task_scheduler::run(&name)?;
    }
    println!("Installed scheduled task {name} → {}", xml_path.display());
    println!("  trigger: at logon (S4U, runs without desktop login)");
    if start {
        println!("  started");
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
        ServiceBackend::TaskScheduler => {
            let name = spec.task_name();
            // 先结束运行实例再删除注册（与 systemd 分支同理：防孤儿进程占端口）
            if let Err(e) = task_scheduler::end(&name) {
                println!("  note: end task: {e}");
            }
            task_scheduler::delete(&name)?;
            let xml_path = spec.task_xml_path();
            if xml_path.exists() {
                std::fs::remove_file(&xml_path).map_err(InstallerError::Io)?;
            }
            println!("Uninstalled scheduled task {name}");
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
        ServiceBackend::TaskScheduler => {
            let name = spec.task_name();
            if let Err(e) = task_scheduler::end(&name) {
                println!("  note: end task: {e}");
            }
            task_scheduler::run(&name)?;
            println!("Restarted {name}");
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
        ServiceBackend::TaskScheduler => {
            let name = spec.task_name();
            println!("  task:       {name}");
            println!("  task xml:   {}", spec.task_xml_path().display());
            match task_scheduler::task_state(&name) {
                task_scheduler::TaskState::Running => println!("  state:      running"),
                task_scheduler::TaskState::Ready => println!("  state:      ready"),
                task_scheduler::TaskState::Disabled => println!("  state:      disabled"),
                task_scheduler::TaskState::Missing => println!("  state:      not installed"),
                task_scheduler::TaskState::Unknown => println!("  state:      unknown"),
            }
            match task_scheduler::last_task_result(&name) {
                Some(code) => println!("  last result: {code:#010x}"),
                None => println!("  last result: (unavailable)"),
            }
            println!();
            println!("--- task xml ---");
            match task_scheduler::query_xml(&name) {
                Ok(text) => println!("{text}"),
                Err(_) => match fs::read_to_string(spec.task_xml_path()) {
                    Ok(text) => println!("{text}"),
                    Err(_) => println!("(task not registered and no persisted xml)"),
                },
            }
            println!("--- recent logs ---");
            tail_latest_log_in(&spec.install_dir.join("logs"));
        }
    }
    Ok(())
}

/// tail install_dir/logs 下最新修改的日志文件（任务计划后端：服务自写文件日志，
/// 文件名不固定，按 mtime 取最新）。
fn tail_latest_log_in(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let newest = entries
        .flatten()
        .filter(|e| e.path().is_file())
        .max_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0)
        });
    if let Some(entry) = newest {
        tail_log_file(&entry.file_name().to_string_lossy(), &entry.path());
    }
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
    if backend == ServiceBackend::TaskScheduler {
        return task_scheduler::task_state(&spec.task_name()) == task_scheduler::TaskState::Running;
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
