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
/// → 注册 → 启动（终态协议验证）。
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
        // 已注册则先结束运行实例，让新定义下次启动即生效（等待窗口同
        // restart——旧实例优雅关闭期间仍持有监听 socket，见 restart_task）
        if let Err(e) = task_scheduler::end(&name) {
            println!("  note: end previous task instance: {e}");
        }
        wait_port_released(spec, std::time::Duration::from_secs(45));
    }
    task_scheduler::create_from_xml(&name, &xml_path)?;
    if start {
        run_until_healthy(spec, &name, std::time::Duration::from_secs(40))?;
        println!("  started");
    }
    println!("Installed scheduled task {name} → {}", xml_path.display());
    println!("  trigger: at logon (S4U, runs without desktop login)");
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

/// 重启：结束 → 等端口释放 → **run + curl 健康验证循环（≤3 次）**。
///
/// 确定性协议（53 多轮实测迭代出的结论）：/end 后旧实例处于 CTRL 优雅
/// 关闭期，监听 socket 的释放与内核回收之间存在窗口，单一 `/run` 可能
/// bind 失败（日志 "Server listening" 打印在 bind 之前，失败无痕）；
/// netstat 的 LISTENING 也不等于服务可用。只认 `curl /health` 真正响应，
/// 未通自动再 `/run`，全败明确报错。
pub fn restart_task(spec: &ServiceSpec) -> Result<()> {
    let name = spec.task_name();
    if let Err(e) = task_scheduler::end(&name) {
        println!("  note: end task: {e}");
    }
    wait_port_released(spec, std::time::Duration::from_secs(45));
    if spec.listen_port.is_none() {
        // 无端口信息：退回旧行为（run 一次即认为完成）
        task_scheduler::run(&name)?;
        println!("Restarted {name}");
        return Ok(());
    }
    run_until_healthy(spec, &name, std::time::Duration::from_secs(40))?;
    println!("Restarted {name}");
    Ok(())
}

/// 终态启动协议：`/run` → curl /health 轮询（per_attempt）→ 未通自动再
/// `/run`（≤3 次）→ 全败报错。schtasks 的"已在运行"等命令错误一律忽略，
/// 健康探测是唯一判定。每轮 40s：document-parser 冷启动（MinerU 环境检查）
/// 可达 2 分钟，3×40s 覆盖；voice-cli 常规 10-20s 出头首轮即过。
fn run_until_healthy(
    spec: &ServiceSpec,
    name: &str,
    per_attempt: std::time::Duration,
) -> Result<()> {
    let Some(port) = spec.listen_port else {
        task_scheduler::run(name)?;
        return Ok(());
    };
    let secs = per_attempt.as_secs();
    for attempt in 1..=3 {
        // "已在运行"等命令错误忽略——以健康探测为准
        let _ = task_scheduler::run(name);
        if crate::cli::common::wait_for_health(port, "/health", secs) {
            if attempt > 1 {
                println!("  (service healthy after {attempt} start attempts)");
            }
            return Ok(());
        }
        println!("  note: health not up after attempt {attempt} — retrying /run");
    }
    Err(InstallerError::Other(format!(
        "task {name} started but /health never responded on port {port}"
    )))
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
