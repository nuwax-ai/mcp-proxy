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
    let base_path = if env.as_ref().is_none_or(|e| !e.contains_key("PATH")) {
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
/// - Windows: `CreationFlags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)` + `JobObject`
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
    ($cmd:expr) => {{
        use process_wrap::tokio::ProcessGroup;
        $cmd.wrap(ProcessGroup::leader());
    }};
}

#[cfg(windows)]
#[macro_export]
macro_rules! wrap_process_v8 {
    ($cmd:expr) => {{
        use process_wrap::tokio::{CreationFlags, JobObject};
        use windows::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};
        $cmd.wrap(CreationFlags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP));
        $cmd.wrap(JobObject);
    }};
}

/// 为 process-wrap 9.x 的 CommandWrap 应用平台特定的包装
///
/// 此宏会根据目标平台自动应用正确的进程包装：
/// - Windows: `CreationFlags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)` + `JobObject`
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
    ($cmd:expr) => {{
        use process_wrap::tokio::ProcessGroup;
        $cmd.wrap(ProcessGroup::leader());
    }};
}

#[cfg(windows)]
#[macro_export]
macro_rules! wrap_process_v9 {
    ($cmd:expr) => {{
        use process_wrap::tokio::{CreationFlags, JobObject};
        use windows::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};
        $cmd.wrap(CreationFlags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP));
        $cmd.wrap(JobObject);
    }};
}

/// 启动 stderr 日志读取任务
///
/// 创建一个异步任务来读取子进程的 stderr 输出并记录到日志。
/// 这个函数封装了通用的 stderr 读取逻辑。
///
/// # Arguments
///
/// * `stderr` - stderr 管道（实现 AsyncRead + Unpin + Send）
/// * `service_name` - MCP 服务名称（用于日志标识）
///
/// # Returns
///
/// 返回 `JoinHandle<()>`，任务会在 stderr 关闭时自动结束
///
/// # Example
///
/// ```ignore
/// use mcp_common::process_compat::spawn_stderr_reader;
///
/// let (tokio_process, child_stderr) = TokioChildProcess::builder(wrapped_cmd)
///     .stderr(Stdio::piped())
///     .spawn()?;
///
/// if let Some(stderr) = child_stderr {
///     spawn_stderr_reader(stderr, "my-mcp-service".to_string());
/// }
/// ```
pub fn spawn_stderr_reader<T>(stderr: T, service_name: String) -> tokio::task::JoinHandle<()>
where
    T: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    // EOF - stderr 已关闭
                    tracing::debug!("[子进程 stderr][{}] 读取结束 (EOF)", service_name);
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        tracing::warn!("[子进程 stderr][{}] {}", service_name, trimmed);
                    }
                }
                Err(e) => {
                    tracing::debug!("[子进程 stderr][{}] 读取错误: {}", service_name, e);
                    break;
                }
            }
        }
    })
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

/// Windows 上预处理 npx 命令，避免 .cmd 文件导致窗口闪烁
///
/// 将 `npx -y package@version` 转换为直接的 `node` 命令。
///
/// # Arguments
///
/// * `command` - 原始命令
/// * `args` - 原始参数
///
/// # Returns
///
/// 返回 `(new_command, new_args)` 元组。如果无法转换，返回原始值。
///
/// # Example
///
/// ```ignore
/// use mcp_common::process_compat::preprocess_npx_command_windows;
///
/// let (cmd, args) = preprocess_npx_command_windows(
///     "npx",
///     Some(vec!["-y".to_string(), "chrome-devtools-mcp@latest".to_string()])
/// );
/// // cmd 可能是 "node.exe"，args 可能是 ["path/to/chrome-devtools-mcp/bin/mcp.js"]
/// ```
#[cfg(target_os = "windows")]
pub fn preprocess_npx_command_windows(
    command: &str,
    args: Option<&[String]>,
) -> (String, Option<Vec<String>>) {
    use tracing::info;

    // 检测 npx 命令
    let is_npx = command == "npx"
        || command == "npx.cmd"
        || command.ends_with("/npx")
        || command.ends_with("\\npx")
        || command.ends_with("/npx.cmd")
        || command.ends_with("\\npx.cmd");

    if !is_npx {
        return (command.to_string(), args.map(|a| a.to_vec()));
    }

    let args = match args {
        Some(a) => a,
        None => return (command.to_string(), None),
    };

    // 提取包名（跳过 -y 标志等）
    // 支持: chrome-devtools-mcp@latest, @scope/package@1.0.0
    let package_spec = args
        .iter()
        .find(|s| !s.starts_with('-') && (s.contains('@') || s.starts_with('@')));

    let Some(pkg) = package_spec else {
        return (command.to_string(), Some(args.to_vec()));
    };

    // 解析包名（去掉版本号，处理 scoped packages）
    let package_name = if pkg.starts_with('@') {
        // Scoped package: @scope/name@version
        let parts: Vec<&str> = pkg.splitn(3, '@').collect();
        if parts.len() >= 3 {
            // @scope/name@version -> @scope/name
            format!("@{}", parts[1])
        } else if parts.len() == 2 && parts[1].contains('/') {
            // @scope/name (no version)
            pkg.to_string()
        } else {
            pkg.to_string()
        }
    } else {
        // Regular package: name@version
        pkg.split('@').next().unwrap_or(pkg).to_string()
    };

    // 尝试找到已安装的包
    if let Some((node_exe, js_entry)) = find_npx_package_entry_windows(&package_name) {
        info!(
            "[MCP] Windows npx 转换: npx {} -> node {}",
            pkg,
            js_entry.display()
        );

        // 构建新参数
        let mut new_args = vec![js_entry.to_string_lossy().to_string()];
        for arg in args {
            // 跳过 -y 和包名
            if arg != "-y" && arg != pkg {
                new_args.push(arg.clone());
            }
        }

        return (node_exe.to_string_lossy().to_string(), Some(new_args));
    }

    // 未找到已安装的包，保持原样
    info!("[MCP] Windows npx 未找到已安装的包: {}，保持原命令", pkg);
    (command.to_string(), Some(args.to_vec()))
}

/// 非 Windows 平台的空实现
#[cfg(not(target_os = "windows"))]
pub fn preprocess_npx_command_windows(
    command: &str,
    args: Option<&[String]>,
) -> (String, Option<Vec<String>>) {
    (command.to_string(), args.map(|a| a.to_vec()))
}

/// 查找 npx 包的 node 可执行文件和 JS 入口（Windows）
#[cfg(target_os = "windows")]
fn find_npx_package_entry_windows(
    package_name: &str,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    use std::path::PathBuf;
    use tracing::info;

    // 查找 node.exe
    let node_exe = find_node_exe_windows()?;

    // 在多个可能的位置查找已安装的包
    let base_search_paths = get_npx_cache_paths_windows();

    for base_path in base_search_paths {
        // 收集所有可能的 node_modules 目录
        let mut node_modules_dirs = Vec::new();

        if base_path.ends_with("_npx") {
            // npx 缓存目录结构: _npx/<hash>/node_modules/<package>
            // 需要遍历 hash 目录
            if let Ok(entries) = std::fs::read_dir(&base_path) {
                for entry in entries.flatten() {
                    let hash_dir = entry.path();
                    let node_modules = hash_dir.join("node_modules");
                    if node_modules.exists() {
                        node_modules_dirs.push(node_modules);
                    }
                }
            }
        } else {
            // 直接是 node_modules 目录或包含 node_modules 的目录
            if base_path.ends_with("node_modules") {
                node_modules_dirs.push(base_path.clone());
            } else if base_path.join("node_modules").exists() {
                node_modules_dirs.push(base_path.join("node_modules"));
            }
        }

        // 在每个 node_modules 目录中查找包
        for node_modules_dir in node_modules_dirs {
            let package_dir = node_modules_dir.join(package_name);
            if !package_dir.exists() {
                continue;
            }

            // 读取 package.json 查找入口
            let package_json_path = package_dir.join("package.json");
            if let Ok(content) = std::fs::read_to_string(&package_json_path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    // 查找 bin 字段
                    let bin_entry = json.get("bin").and_then(|b| {
                        if let Some(s) = b.as_str() {
                            Some(s.to_string())
                        } else if let Some(obj) = b.as_object() {
                            // 对于 bin: { "pkg-name": "./bin/mcp.js" } 的情况
                            // 尝试匹配包名或取第一个
                            obj.get(package_name)
                                .or_else(|| obj.values().next())
                                .and_then(|v| v.as_str())
                                .map(str::to_string)
                        } else {
                            None
                        }
                    });

                    if let Some(bin_entry) = bin_entry {
                        let js_entry = package_dir.join(bin_entry);
                        if js_entry.exists() {
                            info!(
                                "[MCP] Windows 找到包入口: {} -> {}",
                                package_name,
                                js_entry.display()
                            );
                            return Some((node_exe.clone(), js_entry));
                        }
                    }
                }
            }
        }
    }

    None
}

/// 查找 node.exe 路径（Windows）
#[cfg(target_os = "windows")]
fn find_node_exe_windows() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    // 1. 检查环境变量
    if let Ok(node_from_env) = std::env::var("NUWAX_NODE_EXE") {
        let path = PathBuf::from(node_from_env);
        if path.exists() {
            return Some(path);
        }
    }

    // 2. 检查应用资源目录
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let resource_paths = [
                exe_dir
                    .join("resources")
                    .join("node")
                    .join("bin")
                    .join("node.exe"),
                exe_dir
                    .parent()
                    .unwrap_or(exe_dir)
                    .join("resources")
                    .join("node")
                    .join("bin")
                    .join("node.exe"),
            ];

            for path in resource_paths {
                if path.exists() {
                    return Some(path);
                }
            }
        }
    }

    // 3. 在 PATH 中查找
    which::which("node.exe").ok()
}

/// 获取 npx 缓存搜索路径（Windows）
#[cfg(target_os = "windows")]
fn get_npx_cache_paths_windows() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;

    let mut paths = Vec::new();

    // npx 缓存目录（npm 8.16+）- 优先检查 LOCALAPPDATA
    // 这是 npx 实际使用的缓存位置
    if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
        let local_appdata_path = PathBuf::from(&local_appdata);
        // npx 缓存目录格式: LOCALAPPDATA\npm-cache\_npx\<hash>\node_modules
        paths.push(local_appdata_path.join("npm-cache").join("_npx"));
    }

    // npm 全局 node_modules - APPDATA
    if let Ok(appdata) = std::env::var("APPDATA") {
        let appdata_path = PathBuf::from(&appdata);

        // npm 全局目录
        paths.push(appdata_path.join("npm").join("node_modules"));

        // 应用私有目录
        paths.push(
            appdata_path
                .join("com.nuwax.agent-tauri-client")
                .join("node_modules"),
        );

        // 旧版 npm 缓存位置（备用）
        paths.push(appdata_path.join("npm-cache").join("_npx"));
    }

    // 应用资源目录
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let resource_paths = [
                exe_dir.join("resources").join("node").join("node_modules"),
                exe_dir
                    .parent()
                    .unwrap_or(exe_dir)
                    .join("resources")
                    .join("node")
                    .join("node_modules"),
            ];

            for path in resource_paths {
                if path.exists() {
                    paths.push(path);
                }
            }
        }
    }

    paths
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
        let result = ensure_runtime_path("/app/node/bin:/opt/homebrew/bin:/usr/bin");
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
        let result = ensure_runtime_path("/app/uv/bin:/usr/bin:/app/node/bin");
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
        assert_eq!(result, "C:\\Program Files\\nodejs;D:\\tools\\bin;E:\\dev");

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
