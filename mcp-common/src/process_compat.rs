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

/// Windows 上解析命令路径，自动添加扩展名
///
/// 在 Windows 上，命令如 `npx` 实际上是 `npx.cmd` 批处理文件。
/// `std::process::Command` 不会自动查找 `.cmd` 扩展名，需要手动指定。
/// 此函数尝试在 PATH 中查找命令，并返回带扩展名的完整路径或原始命令。
///
/// # Arguments
///
/// * `command` - 要解析的命令字符串
///
/// # Returns
///
/// 如果找到，返回带扩展名的命令；否则返回原始命令
///
/// # Example
///
/// ```ignore
/// use mcp_common::process_compat::resolve_windows_command;
///
/// let resolved = resolve_windows_command("npx");
/// // 返回 "npx.cmd" 或 "C:\Program Files\nodejs\npx.cmd"
/// ```
#[cfg(target_os = "windows")]
pub fn resolve_windows_command(command: &str) -> String {
    use std::path::Path;

    // 如果已经有扩展名，直接返回
    if Path::new(command).extension().is_some() {
        return command.to_string();
    }

    // 如果是绝对路径，直接返回
    if Path::new(command).is_absolute() {
        return command.to_string();
    }

    // 获取 PATH 环境变量
    let path_env = match std::env::var("PATH") {
        Ok(p) => p,
        Err(_) => return command.to_string(),
    };

    // Windows 可执行文件扩展名（按优先级）
    let extensions = [".cmd", ".exe", ".bat", ".ps1"];

    // 遍历 PATH 中的每个目录
    for dir in path_env.split(';') {
        let dir = dir.trim();
        if dir.is_empty() {
            continue;
        }

        // 尝试每个扩展名
        for ext in &extensions {
            let full_path = Path::new(dir).join(format!("{}{}", command, ext));
            if full_path.exists() {
                tracing::debug!(
                    "[MCP] Windows 命令解析: {} -> {}",
                    command,
                    full_path.display()
                );
                // 返回带扩展名的命令（不是完整路径，保持简洁）
                return format!("{}{}", command, ext);
            }
        }
    }

    // 未找到，返回原始命令
    command.to_string()
}

/// 非 Windows 平台的空实现
#[cfg(not(target_os = "windows"))]
pub fn resolve_windows_command(command: &str) -> String {
    command.to_string()
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

/// Windows 上将 Unix 风格路径转换为 Windows 风格
///
/// 转换规则:
/// - `/c/Program Files/...` -> `C:\Program Files\...`
/// - `/cygdrive/c/...` -> `C:\...`
/// - 已经是 Windows 格式的路径保持不变
///
/// # Arguments
///
/// * `path` - 要转换的路径字符串
///
/// # Example
///
/// ```ignore
/// use mcp_common::process_compat::convert_unix_path_to_windows;
///
/// assert_eq!(convert_unix_path_to_windows("/c/Program Files/nodejs"), "C:\\Program Files\\nodejs");
/// assert_eq!(convert_unix_path_to_windows("C:\\Windows"), "C:\\Windows");
/// ```
#[cfg(target_os = "windows")]
pub fn convert_unix_path_to_windows(path: &str) -> String {
    let path = path.trim();

    // 检查是否已经是 Windows 格式 (如 C:\...)
    if path.len() >= 2 && path.chars().nth(1) == Some(':') {
        return path.to_string();
    }

    // 处理 /c/ 格式（Git Bash, MSYS2）
    if path.starts_with('/') && path.len() > 2 {
        let chars: Vec<char> = path.chars().collect();
        if chars[2] == '/' {
            let drive = chars[1].to_ascii_uppercase();
            let rest = &path[3..];
            return format!("{}:\\{}", drive, rest.replace('/', "\\"));
        }
    }

    // 处理 /cygdrive/c/ 格式
    if path.starts_with("/cygdrive/") && path.len() > 11 {
        let rest = &path[10..];
        let chars: Vec<char> = rest.chars().collect();
        if chars.len() >= 2 && chars[1] == '/' {
            let drive = chars[0].to_ascii_uppercase();
            let rest_path = &rest[2..];
            return format!("{}:\\{}", drive, rest_path.replace('/', "\\"));
        }
    }

    path.to_string()
}

/// 非 Windows 平台的空实现
#[cfg(not(target_os = "windows"))]
pub fn convert_unix_path_to_windows(path: &str) -> String {
    path.to_string()
}

/// Windows 上将整个 PATH 环境变量转换为 Windows 格式
///
/// 遍历 PATH 中的每个路径段，将 Unix 风格路径（如 Git Bash/MSYS2 格式）
/// 转换为 Windows 格式。
///
/// # Arguments
///
/// * `path_env` - PATH 环境变量字符串
///
/// # Example
///
/// ```ignore
/// use mcp_common::process_compat::convert_path_to_windows_format;
///
/// let path = "/c/Program Files/nodejs;C:\\Windows\\System32;/d/tools";
/// let result = convert_path_to_windows_format(path);
/// assert_eq!(result, "C:\\Program Files\\nodejs;C:\\Windows\\System32;D:\\tools");
/// ```
#[cfg(target_os = "windows")]
pub fn convert_path_to_windows_format(path_env: &str) -> String {
    path_env
        .split(';')
        .map(convert_unix_path_to_windows)
        .collect::<Vec<_>>()
        .join(";")
}

/// 非 Windows 平台的空实现
#[cfg(not(target_os = "windows"))]
pub fn convert_path_to_windows_format(path_env: &str) -> String {
    path_env.to_string()
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

    #[test]
    #[cfg(target_os = "windows")]
    fn test_convert_unix_path_to_windows() {
        // Git Bash 格式
        assert_eq!(
            convert_unix_path_to_windows("/c/Program Files/nodejs"),
            "C:\\Program Files\\nodejs"
        );
        // MSYS2/Cygwin 格式
        assert_eq!(
            convert_unix_path_to_windows("/cygdrive/c/Windows"),
            "C:\\Windows"
        );
        // 已经是 Windows 格式
        assert_eq!(
            convert_unix_path_to_windows("C:\\Windows\\System32"),
            "C:\\Windows\\System32"
        );
        // 小写驱动器号
        assert_eq!(
            convert_unix_path_to_windows("/d/tools/bin"),
            "D:\\tools\\bin"
        );
        // 根路径
        assert_eq!(convert_unix_path_to_windows("/c/"), "C:\\");
        // 空字符串
        assert_eq!(convert_unix_path_to_windows(""), "");
        // 空白字符串
        assert_eq!(convert_unix_path_to_windows("  "), "");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_convert_path_to_windows_format() {
        // 混合格式 PATH
        let path = "/c/Program Files/nodejs;C:\\Windows\\System32;/d/tools";
        let result = convert_path_to_windows_format(path);
        assert_eq!(
            result,
            "C:\\Program Files\\nodejs;C:\\Windows\\System32;D:\\tools"
        );

        // 纯 Unix 风格 PATH
        let unix_path = "/c/Program Files/nodejs;/d/tools/bin;/e/dev";
        let result = convert_path_to_windows_format(unix_path);
        assert_eq!(
            result,
            "C:\\Program Files\\nodejs;D:\\tools\\bin;E:\\dev"
        );

        // 纯 Windows 风格 PATH（保持不变）
        let win_path = "C:\\Windows\\System32;D:\\tools\\bin";
        let result = convert_path_to_windows_format(win_path);
        assert_eq!(result, win_path);

        // 空字符串
        assert_eq!(convert_path_to_windows_format(""), "");

        // 单个路径
        assert_eq!(
            convert_path_to_windows_format("/c/Program Files/nodejs"),
            "C:\\Program Files\\nodejs"
        );
    }
}
