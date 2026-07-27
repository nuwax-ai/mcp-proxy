//! 子进程启动诊断工具
//!
//! 提供 stdio 子进程启动时的环境诊断日志，供 mcp-proxy / mcp-sse-proxy /
//! mcp-streamable-proxy 共用，避免诊断代码散落在业务逻辑中。

use std::collections::HashMap;

/// PATH 分隔符
const PATH_SEP: char = if cfg!(windows) { ';' } else { ':' };

/// 诊断时检查的镜像相关环境变量
const MIRROR_ENV_KEYS: &[&str] = &[
    "npm_config_registry",
    "UV_INDEX_URL",
    "UV_EXTRA_INDEX_URL",
    "UV_INSECURE_HOST",
    "PIP_INDEX_URL",
];

// ─── 纯数据格式化（不依赖日志框架） ───

/// 返回 PATH 摘要字符串，只展示前 `max_segments` 段
///
/// 示例: `/usr/bin:/usr/local/bin ... (12 entries total)`
pub fn format_path_summary(max_segments: usize) -> String {
    match std::env::var("PATH") {
        Ok(path) => {
            let segments: Vec<&str> = path.split(PATH_SEP).collect();
            let preview: String = segments
                .iter()
                .take(max_segments)
                .copied()
                .collect::<Vec<_>>()
                .join(&PATH_SEP.to_string());
            if segments.len() > max_segments {
                format!("{} ... ({} entries total)", preview, segments.len())
            } else {
                preview
            }
        }
        Err(_) => "(unset)".to_string(),
    }
}

/// 收集当前进程中已设置的镜像相关环境变量
///
/// 返回 `Vec<(key, value)>`，仅包含已设置的条目。
pub fn collect_mirror_env_vars() -> Vec<(&'static str, String)> {
    MIRROR_ENV_KEYS
        .iter()
        .filter_map(|&key| std::env::var(key).ok().map(|val| (key, val)))
        .collect()
}

/// 构造 spawn 失败时的完整错误信息
pub fn format_spawn_error(
    mcp_id: &str,
    command: &str,
    args: &Option<Vec<String>>,
    inner: impl std::fmt::Display,
) -> String {
    let path_val = std::env::var("PATH").unwrap_or_else(|_| "(unset)".to_string());
    format!(
        "Failed to spawn child process - MCP ID: {}, command: {}, \
         args: {:?}, PATH: {}, error: {}",
        mcp_id,
        command,
        args.as_ref().unwrap_or(&Vec::new()),
        path_val,
        inner
    )
}

// ─── 带 tracing 的便捷函数 ───

/// 输出 stdio 子进程启动前的诊断日志（debug 级别）
///
/// 包含：PATH 摘要、镜像变量、config env keys 列表。
/// 业务代码只需在 spawn 前调用此函数。
pub fn log_stdio_spawn_context(tag: &str, mcp_id: &str, env: &Option<HashMap<String, String>>) {
    tracing::debug!(
        "[{}] MCP ID: {}, PATH: {}",
        tag,
        mcp_id,
        format_path_summary(3),
    );

    for (key, val) in collect_mirror_env_vars() {
        tracing::debug!("[{}] MCP ID: {}, {}={}", tag, mcp_id, key, val);
    }

    if let Some(env_vars) = env {
        let keys: Vec<&String> = env_vars.keys().collect();
        tracing::debug!("[{}] MCP ID: {}, config env keys: {:?}", tag, mcp_id, keys);
    }
}

/// 启动阶段环境变量汇总（eprintln 输出，日志框架尚未初始化时使用）
pub fn eprint_env_summary() {
    eprintln!("  - PATH: {}", format_path_summary(3));

    for (key, val) in collect_mirror_env_vars() {
        eprintln!("  - {}={}", key, val);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_path_summary_not_empty() {
        let summary = format_path_summary(3);
        assert!(!summary.is_empty());
    }

    #[test]
    fn test_collect_mirror_env_vars() {
        // 不应 panic
        let _ = collect_mirror_env_vars();
    }

    #[test]
    fn test_format_spawn_error() {
        let msg = format_spawn_error(
            "test-id",
            "npx",
            &Some(vec!["-y".into(), "server".into()]),
            "file not found",
        );
        assert!(msg.contains("test-id"));
        assert!(msg.contains("npx"));
        assert!(msg.contains("file not found"));
    }

    #[test]
    fn test_log_stdio_spawn_context_no_panic() {
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        log_stdio_spawn_context("Test", "test-id", &Some(env));
        log_stdio_spawn_context("Test", "test-id", &None);
    }
}
