//! 子进程环境初始化
//!
//! 在 mcp-proxy 启动早期调用，设置进程级环境变量：
//! - 镜像源（npm_config_registry / UV_INDEX_URL）
//! - 应用内置运行时 PATH 优先

use crate::config::{AppConfig, MirrorYamlConfig};

/// 初始化子进程环境（镜像源 + 内置运行时 PATH）
///
/// 在 main() 启动早期、日志初始化前调用。
/// 设置的环境变量会被所有子进程（npx/uvx 等）自动继承。
pub fn init(app_config: &AppConfig) {
    // 1. 镜像源配置
    init_mirror(&app_config.mirror);

    // 2. 应用内置运行时 PATH 优先
    init_runtime_path();
}

/// 从 config.yml + 环境变量合并镜像配置，设为进程级环境变量
fn init_mirror(yml: &MirrorYamlConfig) {
    let mut config = mcp_common::mirror::MirrorConfig::from_env();

    // config.yml 非空值作为默认（环境变量优先级更高）
    if config.npm_registry.is_none() && !yml.npm_registry.is_empty() {
        config.npm_registry = Some(yml.npm_registry.clone());
    }
    if config.pypi_index_url.is_none() && !yml.pypi_index_url.is_empty() {
        config.pypi_index_url = Some(yml.pypi_index_url.clone());
    }

    if config.is_empty() {
        return;
    }

    if let Some(ref npm) = config.npm_registry {
        eprintln!("  - npm 镜像: {}", npm);
    }
    if let Some(ref pypi) = config.pypi_index_url {
        eprintln!("  - PyPI 镜像: {}", pypi);
    }
    config.apply_to_process_env();
}

/// 确保 NUWAX_APP_RUNTIME_PATH 在进程 PATH 最前面
fn init_runtime_path() {
    if let Ok(current_path) = std::env::var("PATH") {
        let merged = mcp_common::ensure_runtime_path(&current_path);
        if merged != current_path {
            eprintln!("  - 内置运行时 PATH 已前置");
            // SAFETY: main() 启动早期、单线程阶段
            unsafe { std::env::set_var("PATH", &merged) };
        }
    }
}
