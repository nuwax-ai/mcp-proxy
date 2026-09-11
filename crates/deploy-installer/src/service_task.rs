//! 任务计划程序（TaskScheduler）后端的服务生命周期实现。
//!
//! 由 [`crate::service_mgr`] 的 install/uninstall/restart/status 在 Windows
//! 平台分发调用；直接驱动 [`crate::task_scheduler`]（schtasks 包装）。

use crate::error::{InstallerError, Result};
use crate::render_task::render_task_xml;
use crate::spec::ServiceSpec;
use crate::task_scheduler;
use std::fs;
use std::path::Path;

/// 任务计划程序安装链路：渲染 XML → 持久化到 install_dir → 停旧实例（尽力）
/// → 注册 → 启动。
///
/// S4U 主体（无需提权注册、无桌面登录也运行）；注册失败时由上层决定是否用
/// 无 Principal 的降级 XML 重试（Interactive，仅登录运行）。
pub fn install_task(spec: &ServiceSpec, dry_run: bool, _enable: bool, start: bool) -> Result<()> {
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
        wait_port_released(spec, std::time::Duration::from_secs(10));
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

/// 注销任务：先结束运行实例（防孤儿进程占端口，与 systemd 分支同理）→
/// 删除注册 → 清理持久化的任务 XML。
pub fn uninstall_task(spec: &ServiceSpec) -> Result<()> {
    let name = spec.task_name();
    if let Err(e) = task_scheduler::end(&name) {
        println!("  note: end task: {e}");
    }
    task_scheduler::delete(&name)?;
    let xml_path = spec.task_xml_path();
    if xml_path.exists() {
        fs::remove_file(&xml_path).map_err(InstallerError::Io)?;
    }
    println!("Uninstalled scheduled task {name}");
    Ok(())
}

/// 重启：结束运行实例 → 等端口释放 → 启动 → **验证端口真正 LISTENING**。
///
/// 双重竞态兜底（53 实测）：`/end` 后 (a) 监听 socket 释放有延迟；
/// (b) 任务 XML 的 RestartOnFailure 会把强杀视为失败自动重拉，与
/// `/run` 双起竞争——输掉的实例退出后任务可能停在失败态、无进程存活。
/// 启动后等端口就绪，未就绪自动再 `/run` 一次（把"二次 restart 即愈"
/// 自动化为一条命令），仍失败明确报错而非静默假成功。
pub fn restart_task(spec: &ServiceSpec) -> Result<()> {
    let name = spec.task_name();
    if let Err(e) = task_scheduler::end(&name) {
        println!("  note: end task: {e}");
    }
    wait_port_released(spec, std::time::Duration::from_secs(10));
    task_scheduler::run(&name)?;
    if !wait_port_listening(spec, std::time::Duration::from_secs(25)) {
        println!("  note: port not listening after start — retrying /run once");
        // 对已 Running 的任务 /run 会报"已在运行"——以端口判定为准，忽略命令错误
        let _ = task_scheduler::run(&name);
        if !wait_port_listening(spec, std::time::Duration::from_secs(15)) {
            return Err(InstallerError::Other(format!(
                "task {name} restarted but port {:?} never came up",
                spec.listen_port
            )));
        }
    }
    println!("Restarted {name}");
    Ok(())
}

/// 等待端口开始监听（启动成功的判定；探测工具不可用时乐观返回 true）
fn wait_port_listening(spec: &ServiceSpec, timeout: std::time::Duration) -> bool {
    let Some(port) = spec.listen_port else {
        return true;
    };
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match crate::checks::port_occupant(port) {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(_) => return true,
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// 等待端口停止监听：`schtasks /end` 返回时进程只是**开始**终止，监听 socket
/// 尚未关闭——立即 `/run` 的新实例会 bind 失败（"Address already in use"，
/// 53 实测，二次 restart 才能恢复）。轮询 netstat 的 LISTENING 行直到消失；
/// 超时不阻断（打提示后照常启动，保持既有语义），探测工具不可用直接返回。
fn wait_port_released(spec: &ServiceSpec, timeout: std::time::Duration) {
    let Some(port) = spec.listen_port else {
        return;
    };
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match crate::checks::port_occupant(port) {
            Ok(None) => return,
            Ok(Some(_)) => {}
            Err(_) => return,
        }
        if std::time::Instant::now() >= deadline {
            println!("  note: port {port} still listening after {timeout:?} — starting anyway");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// 打印任务状态、上次结果、任务定义体与最新日志 tail。
pub fn status_task(spec: &ServiceSpec) -> Result<()> {
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
    Ok(())
}

pub(crate) fn tail_log_file(name: &str, path: &Path) {
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
