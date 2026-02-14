//! 跨平台进程管理兼容层
//!
//! 提供统一的进程管理抽象，减少平台特定代码的侵入性。
//!
//! # 使用方法
//!
//! ## 命令检测
//!
//! ```ignore
//! use mcp_common::process_compat::check_windows_command;
//!
//! check_windows_command(&config.command);
//! ```
//!
//! ## 进程包装宏
//!
//! process-wrap 8.x (TokioCommandWrap):
//! ```ignore
//! use mcp_common::process_compat::wrap_process_v8;
//!
//! let mut wrapped_cmd = TokioCommandWrap::with_new(...);
//! wrap_process_v8!(wrapped_cmd);
//! wrapped_cmd.wrap(KillOnDrop);
//! ```
//!
//! process-wrap 9.x (CommandWrap):
//! ```ignore
//! use mcp_common::process_compat::wrap_process_v9;
//!
//! let mut wrapped_cmd = CommandWrap::with_new(...);
//! wrap_process_v9!(wrapped_cmd);
//! wrapped_cmd.wrap(KillOnDrop);
//! ```

#[cfg(windows)]
use tracing::{info, warn};

/// 检测 Windows 平台上可能导致弹窗的命令格式
///
/// 在 Windows 上，运行 `.cmd`、`.bat` 文件或 `npx` 命令可能会弹出 CMD 窗口。
/// 此函数会检测这些情况并输出警告，建议用户使用替代方案。
///
/// # Arguments
///
/// * `command` - 要执行的命令字符串
///
/// # Example
///
/// ```ignore
/// use mcp_common::process_compat::check_windows_command;
///
/// check_windows_command("npx some-server");
/// check_windows_command("mcp-server.cmd");
/// ```
#[cfg(windows)]
pub fn check_windows_command(command: &str) {
    use std::path::Path;

    let cmd_ext = Path::new(command)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());

    match cmd_ext {
        Some("cmd" | "bat") => {
            warn!(
                "[MCP] Windows 检测到 .cmd/.bat 命令: {} - 可能会弹 CMD 窗口！",
                command
            );
            warn!("[MCP] 建议改用 node.exe 直接运行 JS 文件，或在配置中使用完整路径");
        }
        None => {
            // 无扩展名，检查是否是 npx 命令
            if command.contains("npx") {
                warn!(
                    "[MCP] Windows 检测到 npx 命令: {} - 可能会弹 CMD 窗口！",
                    command
                );
                warn!("[MCP] 建议改用 node.exe 直接运行 JS 文件");
            }
        }
        _ => {
            info!("[MCP] Windows 检测到命令格式: {}", command);
        }
    }
}

/// Unix/macOS 平台的空实现
#[cfg(not(windows))]
pub fn check_windows_command(_command: &str) {
    // 非 Windows 平台无需检测
}

/// 为 process-wrap 8.x 的 TokioCommandWrap 应用平台特定的包装
///
/// 此宏会根据目标平台自动应用正确的进程包装：
/// - Windows: `CreationFlags(CREATE_NO_WINDOW)` + `JobObject`
/// - Unix: `ProcessGroup::leader()`
///
/// # Arguments
///
/// * `$cmd` - 可变的 TokioCommandWrap 实例
///
/// # Example
///
/// ```ignore
/// use process_wrap::tokio::{TokioCommandWrap, KillOnDrop};
/// use mcp_common::process_compat::wrap_process_v8;
///
/// let mut wrapped_cmd = TokioCommandWrap::with_new("node", |cmd| {
///     cmd.arg("server.js");
/// });
/// wrap_process_v8!(wrapped_cmd);
/// wrapped_cmd.wrap(KillOnDrop);
/// ```
#[cfg(unix)]
#[macro_export]
macro_rules! wrap_process_v8 {
    ($cmd:expr) => {
        {
            use process_wrap::tokio::ProcessGroup;
            $cmd.wrap(ProcessGroup::leader());
        }
    };
}

#[cfg(windows)]
#[macro_export]
macro_rules! wrap_process_v8 {
    ($cmd:expr) => {
        {
            use process_wrap::tokio::{CreationFlags, JobObject};
            use windows::Win32::System::Threading::CREATE_NO_WINDOW;
            $cmd.wrap(CreationFlags(CREATE_NO_WINDOW));
            $cmd.wrap(JobObject);
        }
    };
}

/// 为 process-wrap 9.x 的 CommandWrap 应用平台特定的包装
///
/// 此宏会根据目标平台自动应用正确的进程包装：
/// - Windows: `CreationFlags(CREATE_NO_WINDOW)` + `JobObject`
/// - Unix: `ProcessGroup::leader()`
///
/// # Arguments
///
/// * `$cmd` - 可变的 CommandWrap 实例
///
/// # Example
///
/// ```ignore
/// use process_wrap::tokio::{CommandWrap, KillOnDrop};
/// use mcp_common::process_compat::wrap_process_v9;
///
/// let mut wrapped_cmd = CommandWrap::with_new("node", |cmd| {
///     cmd.arg("server.js");
/// });
/// wrap_process_v9!(wrapped_cmd);
/// wrapped_cmd.wrap(KillOnDrop);
/// ```
#[cfg(unix)]
#[macro_export]
macro_rules! wrap_process_v9 {
    ($cmd:expr) => {
        {
            use process_wrap::tokio::ProcessGroup;
            $cmd.wrap(ProcessGroup::leader());
        }
    };
}

#[cfg(windows)]
#[macro_export]
macro_rules! wrap_process_v9 {
    ($cmd:expr) => {
        {
            use process_wrap::tokio::{CreationFlags, JobObject};
            use windows::Win32::System::Threading::CREATE_NO_WINDOW;
            $cmd.wrap(CreationFlags(CREATE_NO_WINDOW));
            $cmd.wrap(JobObject);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_windows_command_non_windows() {
        // 在非 Windows 平台上，此函数应该不执行任何操作
        check_windows_command("npx some-server");
        check_windows_command("test.cmd");
    }
}
