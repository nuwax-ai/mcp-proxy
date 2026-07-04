use crate::model::CommandExecutor;
use anyhow::Result;
use log::{info, warn};
use std::path::Path;
use tokio::process::Command;

//定义国内python加速地址: https://mirrors.aliyun.com/pypi/simple
const PYTHON_ACCELERATION_ADDRESS: &str = "https://mirrors.aliyun.com/pypi/simple";

//使用 uv安装 python 3.13，比如： uv python install 3.13
async fn install_python_3_13() -> Result<()> {
    let mut cmd = Command::new("uv");
    cmd.arg("python")
        .arg("install")
        .arg("3.13")
        .kill_on_drop(true);
    match CommandExecutor::with_timeout(cmd.status(), 600).await {
        Ok(Ok(status)) => {
            if !status.success() {
                warn!("安装Python 3.13失败");
                return Err(anyhow::anyhow!("安装Python 3.13失败"));
            }
            info!("安装Python 3.13成功");
            Ok(())
        }
        Ok(Err(e)) => {
            warn!("命令执行失败: {e}");
            Err(anyhow::anyhow!("命令执行失败: {}", e))
        }
        Err(e) => {
            warn!("执行超时或系统错误: {e}");
            Err(anyhow::anyhow!("执行超时或系统错误: {}", e))
        }
    }
}

// 检查 uv 虚拟环境是否存在
async fn check_and_create_uv_venv() -> Result<()> {
    info!("检查 uv 虚拟环境...");

    // 检查 .venv 目录是否存在
    if Path::new(".venv").exists() {
        info!("uv 虚拟环境已存在");
        return Ok(());
    }

    info!("未检测到 uv 虚拟环境，开始创建...");
    let mut cmd = Command::new("uv");
    cmd.arg("venv").kill_on_drop(true);

    match CommandExecutor::with_timeout(cmd.status(), 600).await {
        Ok(Ok(status)) => {
            if !status.success() {
                warn!("创建 uv 虚拟环境失败");
                return Err(anyhow::anyhow!("创建 uv 虚拟环境失败"));
            }
            info!("创建 uv 虚拟环境成功");
            Ok(())
        }
        Ok(Err(e)) => {
            warn!("命令执行失败: {e}");
            Err(anyhow::anyhow!("命令执行失败: {}", e))
        }
        Err(e) => {
            warn!("执行超时或系统错误: {e}");
            Err(anyhow::anyhow!("执行超时或系统错误: {}", e))
        }
    }
}

/// 预热Python环境，安装常用依赖
async fn warm_up_python_env(custom_deps: Option<Vec<String>>) -> Result<()> {
    info!("开始预热Python环境...");

    // 默认Python依赖列表
    let default_deps = [
        "requests",
        "pandas",
        "numpy",
        "matplotlib",
        "scikit-learn",
        "pytest",
        "pydantic",
        "fastapi",
        "uvicorn",
        "sqlalchemy",
        "opencv-python",
        "python-docx",
    ];

    // 使用自定义依赖或默认依赖
    let deps_to_install = if let Some(deps) = custom_deps {
        info!("使用自定义Python依赖列表");
        deps
    } else {
        info!("使用默认Python依赖列表");
        default_deps.iter().map(|&s| s.to_string()).collect()
    };

    let total_deps = deps_to_install.len();
    info!("总共需要预热 {total_deps} 个Python依赖");

    // 使用uv安装依赖
    for (index, dep) in deps_to_install.iter().enumerate() {
        let progress = ((index + 1) as f32 / total_deps as f32 * 100.0) as u32;
        info!("预热进度: {progress}% - 正在安装Python依赖: {dep}");

        let mut cmd = Command::new("uv");
        cmd.arg("pip")
            .arg("install")
            .arg("--default-index")
            .arg(PYTHON_ACCELERATION_ADDRESS)
            .arg(dep)
            .kill_on_drop(true);

        match CommandExecutor::with_timeout(cmd.status(), 600).await {
            Ok(Ok(status)) => {
                if !status.success() {
                    warn!("安装依赖 {dep} 失败");
                }
            }
            Ok(Err(e)) => {
                warn!("命令执行失败: {e} - 依赖: {dep}");
                continue;
            }
            Err(e) => {
                warn!("执行超时或系统错误: {e} - 依赖: {dep}");
                continue;
            }
        }
    }

    info!("Python环境预热完成 (100%)");
    Ok(())
}

/// 预热JavaScript/TypeScript环境，缓存常用模块
async fn warm_up_js_env(
    custom_npm_packages: Option<Vec<String>>,
    custom_jsr_packages: Option<Vec<String>>,
    custom_node_modules: Option<Vec<String>>,
) -> Result<()> {
    info!("开始预热JavaScript/TypeScript环境...");

    // 默认npm包列表
    let default_npm_packages = [
        "lodash",
        "axios",
        "moment",
        "uuid",
        "express",
        "react",
        "react-dom",
        "typescript",
        "jest",
        "webpack",
    ];

    // 默认JSR包列表
    let default_jsr_packages = [
        "@std/testing",
        "@std/http",
        "@std/path",
        "@std/fs",
        "@std/encoding/json",
    ];

    // 默认Node.js内置模块列表
    let default_node_modules = [
        "crypto", "buffer", "fs", "path", "http", "https", "url", "util", "stream", "events",
    ];

    // 使用自定义npm包或默认包
    let npm_packages = if let Some(packages) = custom_npm_packages {
        info!("使用自定义npm包列表");
        packages
    } else {
        info!("使用默认npm包列表");
        default_npm_packages
            .iter()
            .map(|&s| s.to_string())
            .collect()
    };

    // 使用自定义JSR包或默认包
    let jsr_packages = if let Some(packages) = custom_jsr_packages {
        info!("使用自定义JSR包列表");
        packages
    } else {
        info!("使用默认JSR包列表");
        default_jsr_packages
            .iter()
            .map(|&s| s.to_string())
            .collect()
    };

    // 使用自定义Node.js模块或默认模块
    let node_modules = if let Some(modules) = custom_node_modules {
        info!("使用自定义Node.js模块列表");
        modules
    } else {
        info!("使用默认Node.js模块列表");
        default_node_modules
            .iter()
            .map(|&s| s.to_string())
            .collect()
    };

    // 计算总任务数
    let total_tasks = npm_packages.len() + jsr_packages.len() + node_modules.len();
    info!("总共需要预热 {total_tasks} 个JavaScript/TypeScript模块");

    let mut completed_tasks = 0;

    // 预热npm包
    for pkg in npm_packages.iter() {
        completed_tasks += 1;
        let progress = (completed_tasks as f32 / total_tasks as f32 * 100.0) as u32;
        info!("预热进度: {progress}% - 正在缓存npm包: {pkg}");

        let mut cmd = Command::new("deno");
        cmd.args(["cache", "--reload", &format!("npm:{pkg}")])
            .kill_on_drop(true);

        match CommandExecutor::with_timeout(cmd.status(), 600).await {
            Ok(Ok(status)) => {
                if !status.success() {
                    warn!("缓存npm包 {pkg} 失败");
                }
            }
            Ok(Err(e)) => {
                warn!("命令执行失败: {e} - 包: {pkg}");
                continue;
            }
            Err(e) => {
                warn!("执行超时或系统错误: {e} - 包: {pkg}");
                continue;
            }
        }
    }

    // 预热JSR包
    for pkg in jsr_packages.iter() {
        completed_tasks += 1;
        let progress = (completed_tasks as f32 / total_tasks as f32 * 100.0) as u32;
        info!("预热进度: {progress}% - 正在缓存JSR包: {pkg}");

        let mut cmd = Command::new("deno");
        cmd.args(["cache", "--reload", &format!("jsr:{pkg}")])
            .kill_on_drop(true);

        match CommandExecutor::with_timeout(cmd.status(), 600).await {
            Ok(Ok(status)) => {
                if !status.success() {
                    warn!("缓存JSR包 {pkg} 失败");
                }
            }
            Ok(Err(e)) => {
                warn!("命令执行失败: {e} - 包: {pkg}");
                continue;
            }
            Err(e) => {
                warn!("执行超时或系统错误: {e} - 包: {pkg}");
                continue;
            }
        }
    }

    // 预热Node.js内置模块
    for module in node_modules.iter() {
        completed_tasks += 1;
        let progress = (completed_tasks as f32 / total_tasks as f32 * 100.0) as u32;
        info!("预热进度: {progress}% - 正在缓存Node.js模块: {module}");

        let mut cmd = Command::new("deno");
        cmd.args(["cache", "--reload", &format!("node:{module}")])
            .kill_on_drop(true);

        match CommandExecutor::with_timeout(cmd.status(), 600).await {
            Ok(Ok(status)) => {
                if !status.success() {
                    warn!("缓存Node.js模块 {module} 失败");
                }
            }
            Ok(Err(e)) => {
                warn!("命令执行失败: {e} - 模块: {module}");
                continue;
            }
            Err(e) => {
                warn!("执行超时或系统错误: {e} - 包: {module}");
                continue;
            }
        }
    }

    info!("JavaScript/TypeScript环境预热完成 (100%)");
    Ok(())
}

/// 预热所有脚本执行环境
pub async fn warm_up_all_envs(
    custom_python_deps: Option<Vec<String>>,
    custom_npm_packages: Option<Vec<String>>,
    custom_jsr_packages: Option<Vec<String>>,
    custom_node_modules: Option<Vec<String>>,
) -> Result<()> {
    info!("开始预热所有脚本执行环境...");

    // 检查并创建 uv 虚拟环境
    if let Err(e) = check_and_create_uv_venv().await {
        warn!("检查或创建 uv 虚拟环境失败: {e}");
    }

    // 安装Python 3.13
    if let Err(e) = install_python_3_13().await {
        warn!("安装Python 3.13失败: {e}");
    }

    // 预热Python环境
    if let Err(e) = warm_up_python_env(custom_python_deps).await {
        warn!("预热Python环境失败: {e}");
    }

    // 预热JavaScript/TypeScript环境
    if let Err(e) = warm_up_js_env(
        custom_npm_packages,
        custom_jsr_packages,
        custom_node_modules,
    )
    .await
    {
        warn!("预热JavaScript/TypeScript环境失败: {e}");
    }

    info!("所有脚本执行环境预热完成");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_warm_up_python_env() {
        // 测试默认依赖
        warm_up_python_env(None).await.unwrap();

        // 测试自定义依赖
        let custom_deps = vec!["requests".to_string(), "pandas".to_string()];
        warm_up_python_env(Some(custom_deps)).await.unwrap();
    }

    #[tokio::test]
    async fn test_warm_up_js_env() {
        // 测试默认依赖
        warm_up_js_env(None, None, None).await.unwrap();

        // 测试自定义依赖
        let custom_npm = vec!["lodash".to_string(), "axios".to_string()];
        let custom_jsr = vec!["@std/testing".to_string()];
        let custom_node = vec!["crypto".to_string(), "buffer".to_string()];
        warm_up_js_env(Some(custom_npm), Some(custom_jsr), Some(custom_node))
            .await
            .unwrap();
    }
}
