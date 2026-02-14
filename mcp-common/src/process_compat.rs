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

    match cmd_ext.as_deref() {
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

/// 确保应用内置运行时路径（NUWAX_APP_RUNTIME_PATH）在 PATH 最前面。
///
/// 当应用捆绑了 node/uv 等运行时时，通过 `NUWAX_APP_RUNTIME_PATH` 传递其路径。
/// 此函数将这些路径插入到给定 PATH 的最前面，确保优先使用应用内置版本，
/// 即使用户在 MCP 配置的 `env` 中指定了自定义 PATH。
///
/// **按段去重**：将 runtime_path 和现有 PATH 拆分为独立条目，
/// 先放 runtime 段，再追加 PATH 中不在 runtime 里的段，彻底避免重复。
///
/// 如果 `NUWAX_APP_RUNTIME_PATH` 未设置或为空，直接返回原始 PATH。
pub fn ensure_runtime_path(path: &str) -> String {
    if let Ok(runtime_path) = std::env::var("NUWAX_APP_RUNTIME_PATH") {
        let runtime_path = runtime_path.trim();
        if !runtime_path.is_empty() {
            let sep = if cfg!(windows) { ";" } else { ":" };

            // 将 runtime_path 拆成各段
            let runtime_segments: Vec<&str> =
                runtime_path.split(sep).filter(|s| !s.is_empty()).collect();

            // 将现有 PATH 拆成各段，去掉已在 runtime 中的
            let existing_segments: Vec<&str> = path
                .split(sep)
                .filter(|s| !s.is_empty() && !runtime_segments.contains(s))
                .collect();

            let merged: Vec<&str> = runtime_segments
                .iter()
                .copied()
                .chain(existing_segments)
                .collect();

            let result = merged.join(sep);
            if result != path {
                tracing::info!(
                    "[ProcessCompat] 前置应用内置运行时到 PATH: {}",
                    runtime_path
                );
            }
            return result;
        }
    }
    path.to_string()
}

/// 为 stdio 子进程准备最终的 PATH 和过滤后的环境变量。
///
/// 统一处理：
/// 1. 从 config env 或父进程确定基础 PATH
/// 2. Windows 上追加 npm 全局 bin 目录
/// 3. 通过 `ensure_runtime_path` 按段去重前置应用内置运行时
/// 4. 从 config env 中过滤掉 PATH（已单独处理）
///
/// 返回 `(Option<final_path>, filtered_env)`，调用方只需 apply 到 `cmd` 即可。
pub fn prepare_stdio_env(
    env: &Option<std::collections::HashMap<String, String>>,
) -> (Option<String>, Option<Vec<(String, String)>>) {
    // 1. 确定基础 PATH
    let base_path = if env.as_ref().map_or(true, |e| !e.contains_key("PATH")) {
        std::env::var("PATH").ok()
    } else {
        env.as_ref().and_then(|e| e.get("PATH").cloned())
    };

    // 2. Windows: 追加 npm 全局 bin + 3. ensure_runtime_path
    let final_path = base_path.map(|path| {
        #[cfg(target_os = "windows")]
        let path = {
            if let Ok(appdata) = std::env::var("APPDATA") {
                let npm_path = format!(r"{}\npm", appdata);
                if !path.contains(&npm_path) {
                    format!("{};{}", path, npm_path)
                } else {
                    path
                }
            } else {
                tracing::warn!("Windows: APPDATA not found, skipping npm global bin");
                path
            }
        };
        ensure_runtime_path(&path)
    });

    // 4. 过滤掉 PATH（已单独处理）
    let filtered_env = env.as_ref().map(|vars| {
        vars.iter()
            .filter(|(k, _)| k.as_str() != "PATH")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    });

    (final_path, filtered_env)
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

    #[test]
    fn test_ensure_runtime_path_no_env() {
        // NUWAX_APP_RUNTIME_PATH 未设置时，返回原始 PATH
        unsafe { std::env::remove_var("NUWAX_APP_RUNTIME_PATH") };
        let result = ensure_runtime_path("/usr/bin:/usr/local/bin");
        assert_eq!(result, "/usr/bin:/usr/local/bin");
    }

    #[test]
    fn test_ensure_runtime_path_prepend() {
        unsafe {
            std::env::set_var("NUWAX_APP_RUNTIME_PATH", "/app/node/bin:/app/uv/bin");
        }
        let result = ensure_runtime_path("/usr/bin:/usr/local/bin");
        assert_eq!(result, "/app/node/bin:/app/uv/bin:/usr/bin:/usr/local/bin");
        unsafe { std::env::remove_var("NUWAX_APP_RUNTIME_PATH") };
    }

    #[test]
    fn test_ensure_runtime_path_dedup() {
        // 模拟：PATH 中已有 runtime 的部分段 → 不应重复
        unsafe {
            std::env::set_var("NUWAX_APP_RUNTIME_PATH", "/app/node/bin:/app/uv/bin");
        }
        let result =
            ensure_runtime_path("/app/node/bin:/opt/homebrew/bin:/usr/bin");
        assert_eq!(
            result,
            "/app/node/bin:/app/uv/bin:/opt/homebrew/bin:/usr/bin"
        );
        unsafe { std::env::remove_var("NUWAX_APP_RUNTIME_PATH") };
    }

    #[test]
    fn test_ensure_runtime_path_all_present() {
        // PATH 已含全部 runtime 段 → 仅调整顺序确保 runtime 在前
        unsafe {
            std::env::set_var("NUWAX_APP_RUNTIME_PATH", "/app/node/bin:/app/uv/bin");
        }
        let result =
            ensure_runtime_path("/app/uv/bin:/usr/bin:/app/node/bin");
        assert_eq!(result, "/app/node/bin:/app/uv/bin:/usr/bin");
        unsafe { std::env::remove_var("NUWAX_APP_RUNTIME_PATH") };
    }

    #[test]
    fn test_ensure_runtime_path_double_node() {
        // 模拟日志中的问题：node/bin 出现两次
        unsafe {
            std::env::set_var(
                "NUWAX_APP_RUNTIME_PATH",
                "/app/node/bin:/app/uv/bin:/app/debug",
            );
        }
        let result = ensure_runtime_path(
            "/app/node/bin:/app/node/bin:/app/uv/bin:/app/debug:/opt/homebrew/bin",
        );
        assert_eq!(
            result,
            "/app/node/bin:/app/uv/bin:/app/debug:/opt/homebrew/bin"
        );
        unsafe { std::env::remove_var("NUWAX_APP_RUNTIME_PATH") };
    }
}
