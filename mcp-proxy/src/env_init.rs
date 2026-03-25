//! 进程环境初始化
//!
//! 在 mcp-proxy 启动早期调用，设置进程级环境变量：
//! - 镜像源（npm_config_registry / UV_INDEX_URL）

use crate::config::{AppConfig, MirrorYamlConfig};
use mcp_common::t;

/// 初始化进程环境（镜像源）
///
/// 在 main() 启动早期、日志初始化前调用。
/// 设置的环境变量会被所有子进程（npx/uvx 等）自动继承。
pub fn init(app_config: &AppConfig) {
    // 1. 镜像源配置
    init_mirror(&app_config.mirror);

    // 2. 汇总诊断日志
    mcp_common::diagnostic::eprint_env_summary();
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
        eprintln!("  - {}", t!("cli.mirror.not_configured"));
        return;
    }

    if let Some(ref npm) = config.npm_registry {
        eprintln!("  - {}", t!("cli.mirror.npm", url = npm));
    }
    if let Some(ref pypi) = config.pypi_index_url {
        eprintln!("  - {}", t!("cli.mirror.pypi", url = pypi));
    }
    config.apply_to_process_env();
}
