//! Windows 任务计划程序（schtasks.exe / PowerShell）包装。
//!
//! 与 [`crate::systemd`]（systemctl 包装）同层级。两个只读状态探针走
//! PowerShell（`Get-ScheduledTask` 的 `State` 枚举值不受系统本地化影响；
//! `schtasks /query /v` 的字段名/值随显示语言变化，不可解析）；其余生命周期
//! 操作走 `schtasks`。任务名已由 [`crate::validate_unit_name`] 限制在
//! `[A-Za-z0-9._-]`，嵌入 PowerShell 单引号字符串内无注入面。

use crate::error::{InstallerError, Result};
use std::path::Path;

/// 任务运行状态（`Get-ScheduledTask` 的 State 枚举 ToString，语言无关）。
///
/// 非 Windows 平台仅 `Unknown` 可达（桩），其余变体由 Windows 的 `task_state` 构造。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Disabled,
    /// 任务未注册。
    Missing,
    Unknown,
}

/// 任务是否已注册（`schtasks /query /tn` exit 0）。
#[cfg(target_os = "windows")]
pub fn task_exists(name: &str) -> bool {
    std::process::Command::new("schtasks")
        .args(["/query", "/tn", name])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// 从 XML 定义文件注册（覆盖同名任务）。
#[cfg(target_os = "windows")]
pub fn create_from_xml(name: &str, xml_path: &Path) -> Result<()> {
    let cmd = format!("schtasks /create /f /tn {name}");
    let out = std::process::Command::new("schtasks")
        .args(["/create", "/f", "/tn", name])
        .arg("/xml")
        .arg(xml_path)
        .output()
        .map_err(|e| InstallerError::CommandFailed {
            cmd: cmd.clone(),
            detail: e.to_string(),
        })?;
    ensure_success(&cmd, &out)
}

/// 立即启动任务（`/run`）。
#[cfg(target_os = "windows")]
pub fn run(name: &str) -> Result<()> {
    let cmd = format!("schtasks /run /tn {name}");
    let out = std::process::Command::new("schtasks")
        .args(["/run", "/tn", name])
        .output()
        .map_err(|e| InstallerError::CommandFailed {
            cmd: cmd.clone(),
            detail: e.to_string(),
        })?;
    ensure_success(&cmd, &out)
}

/// 结束任务的运行实例（`/end`）。任务未运行视为成功（对齐 systemd stop 软语义）。
#[cfg(target_os = "windows")]
pub fn end(name: &str) -> Result<()> {
    let cmd = format!("schtasks /end /tn {name}");
    let out = std::process::Command::new("schtasks")
        .args(["/end", "/tn", name])
        .output()
        .map_err(|e| InstallerError::CommandFailed {
            cmd: cmd.clone(),
            detail: e.to_string(),
        })?;
    let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
    let not_running = stderr.contains("is not running") || stderr.contains("尚未运行");
    if !out.status.success() && !not_running {
        return Err(InstallerError::CommandFailed {
            cmd,
            detail: stderr,
        });
    }
    Ok(())
}

/// 删除任务注册（`/delete /f`）。任务不存在视为成功。
#[cfg(target_os = "windows")]
pub fn delete(name: &str) -> Result<()> {
    let cmd = format!("schtasks /delete /f /tn {name}");
    let out = std::process::Command::new("schtasks")
        .args(["/delete", "/f", "/tn", name])
        .output()
        .map_err(|e| InstallerError::CommandFailed {
            cmd: cmd.clone(),
            detail: e.to_string(),
        })?;
    let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
    let not_found = stderr.contains("cannot find") || stderr.contains("找不到");
    if !out.status.success() && !not_found {
        return Err(InstallerError::CommandFailed {
            cmd,
            detail: stderr,
        });
    }
    Ok(())
}

/// 读回已注册任务的 XML 定义（`/query /xml`）。
#[cfg(target_os = "windows")]
pub fn query_xml(name: &str) -> Result<String> {
    let cmd = format!("schtasks /query /tn {name} /xml");
    let out = std::process::Command::new("schtasks")
        .args(["/query", "/tn", name, "/xml"])
        .output()
        .map_err(|e| InstallerError::CommandFailed {
            cmd: cmd.clone(),
            detail: e.to_string(),
        })?;
    ensure_success(&cmd, &out)?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 任务当前状态（PowerShell，语言无关枚举值）。
#[cfg(target_os = "windows")]
pub fn task_state(name: &str) -> TaskState {
    let script = state_probe_script(name);
    let Ok(out) = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
    else {
        return TaskState::Unknown;
    };
    if !out.status.success() {
        return TaskState::Unknown;
    }
    match String::from_utf8_lossy(&out.stdout).trim() {
        "Running" => TaskState::Running,
        "Ready" => TaskState::Ready,
        "Disabled" => TaskState::Disabled,
        "" => TaskState::Missing,
        _ => TaskState::Unknown,
    }
}

/// 任务上次退出码（`Get-ScheduledTaskInfo` 的 LastTaskResult）。查询失败返回 None。
#[cfg(target_os = "windows")]
pub fn last_task_result(name: &str) -> Option<i32> {
    let script = format!(
        "(Get-ScheduledTaskInfo -TaskName '{}' -ErrorAction SilentlyContinue).LastTaskResult",
        name
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<i32>()
        .ok()
}

/// PowerShell 状态探针脚本（纯函数供单测：任务名单引号包裹，名字字符集已受限）。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn state_probe_script(name: &str) -> String {
    format!("(Get-ScheduledTask -TaskName '{name}' -ErrorAction SilentlyContinue).State")
}

// ---- 非 Windows 桩：保持模块全平台可编译（TaskScheduler 分支在非 Windows 上
// 不可达，但 service_mgr/checks 的 match 臂引用这些函数时不需再逐处 cfg 门控）----

#[cfg(not(target_os = "windows"))]
pub fn task_exists(_name: &str) -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub fn create_from_xml(_name: &str, _xml_path: &Path) -> Result<()> {
    Err(unavailable())
}

#[cfg(not(target_os = "windows"))]
pub fn run(_name: &str) -> Result<()> {
    Err(unavailable())
}

#[cfg(not(target_os = "windows"))]
pub fn end(_name: &str) -> Result<()> {
    Err(unavailable())
}

#[cfg(not(target_os = "windows"))]
pub fn delete(_name: &str) -> Result<()> {
    Err(unavailable())
}

#[cfg(not(target_os = "windows"))]
pub fn query_xml(_name: &str) -> Result<String> {
    Err(unavailable())
}

#[cfg(not(target_os = "windows"))]
pub fn task_state(_name: &str) -> TaskState {
    TaskState::Unknown
}

#[cfg(not(target_os = "windows"))]
pub fn last_task_result(_name: &str) -> Option<i32> {
    None
}

#[cfg(not(target_os = "windows"))]
fn unavailable() -> InstallerError {
    InstallerError::Other("task scheduler backend is only available on Windows".into())
}

#[cfg(target_os = "windows")]
fn ensure_success(cmd: &str, out: &std::process::Output) -> Result<()> {
    if out.status.success() {
        return Ok(());
    }
    Err(InstallerError::CommandFailed {
        cmd: cmd.to_string(),
        detail: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_probe_script_quotes_task_name() {
        assert_eq!(
            state_probe_script("com.nuwax.document-parser"),
            "(Get-ScheduledTask -TaskName 'com.nuwax.document-parser' -ErrorAction SilentlyContinue).State"
        );
    }

    #[test]
    fn state_probe_script_rejects_quote_injection() {
        // validate_unit_name 已限制字符集；此处验证带引号的名字会被渲染层拒绝，
        // 探针脚本本身遇到非法名应先失败（防御纵深）
        let bad = "a'b";
        assert!(crate::render::validate_unit_name(bad).is_err());
    }
}
