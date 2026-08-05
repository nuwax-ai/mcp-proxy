//! `uv-init` 子命令：在当前目录初始化 uv 虚拟环境并安装依赖。

use anyhow::{Context as _, Result};
use document_parser::{
    AppError,
    utils::environment_manager::{
        CleanupRisk, DirectoryValidationResult, EnvironmentManager, EnvironmentStatus, InstallStage,
    },
};
use std::env;

/// 处理uv环境初始化命令
pub async fn handle_uv_init_command(_environment_manager: &EnvironmentManager) -> Result<()> {
    println!(
        "🚀 Start initializing the uv virtual environment and dependencies in the current directory..."
    );
    println!();

    // 检查当前目录
    let current_dir = env::current_dir().context("无法获取当前目录")?;
    println!("📁 Current working directory: {}", current_dir.display());
    println!("📁 The virtual environment will be created in: ./venv/");
    println!();

    // 创建基于当前目录的环境管理器
    let local_env_manager =
        EnvironmentManager::for_current_directory().context("无法创建环境管理器")?;

    // 1. 验证当前目录设置（任务12的核心功能）
    println!("🔍 Verify current directory settings...");
    let _validation_result = match local_env_manager.check_current_directory_readiness().await {
        Ok(result) => {
            if result.is_valid {
                println!("✅ Directory verification passed");
                if !result.warnings.is_empty() {
                    println!("⚠️ Found {} warnings", result.warnings.len());
                    for warning in &result.warnings {
                        println!("      • {}", warning.message);
                    }
                }
            } else {
                println!(
                    "❌ Directory verification failed, {} problems found",
                    result.issues.len()
                );
                for issue in &result.issues {
                    println!(
                        "      • [{}] {}",
                        format!("{:?}", issue.severity).to_uppercase(),
                        issue.message
                    );
                }

                // 尝试自动修复可修复的问题
                let auto_fixable: Vec<_> = result
                    .issues
                    .iter()
                    .filter(|issue| issue.auto_fixable)
                    .collect();

                if !auto_fixable.is_empty() {
                    println!(
                        "🔧 Try to automatically fix {} problems...",
                        auto_fixable.len()
                    );

                    for cleanup_option in &result.cleanup_options {
                        if cleanup_option.risk_level == CleanupRisk::Low
                            || cleanup_option.risk_level == CleanupRisk::Medium
                        {
                            match local_env_manager
                                .execute_cleanup_option(cleanup_option.option_type.clone())
                                .await
                            {
                                Ok(message) => println!("      ✅ {message}"),
                                Err(e) => println!("❌ Cleanup failed: {e}"),
                            }
                        }
                    }
                } else {
                    println!("💡 Please solve the following problems manually:");
                    for recommendation in &result.recommendations {
                        println!("      • {recommendation}");
                    }
                    println!();
                    return Err(anyhow::anyhow!("目录验证失败，请解决上述问题后重试"));
                }
            }
            result
        }
        Err(e) => {
            println!("⚠️ Directory verification failed: {e}");
            println!("Proceed with the installation, but you may encounter problems...");
            // 创建一个默认的验证结果以继续执行
            DirectoryValidationResult {
                is_valid: false,
                current_directory: current_dir.clone(),
                venv_path: current_dir.join("venv"),
                issues: Vec::new(),
                warnings: Vec::new(),
                cleanup_options: Vec::new(),
                recommendations: Vec::new(),
            }
        }
    };
    println!();

    // 1. 检查当前环境状态
    println!("🔍 Check current environment status...");
    let env_status = match local_env_manager.check_environment().await {
        Ok(status) => {
            println!("Environmental check completed:");
            println!(
                "   Python:     {}",
                if status.python_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            println!(
                "uv tool: {}",
                if status.uv_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            println!(
                "Virtual environment: {}",
                if status.virtual_env_active {
                    "✅ Active"
                } else {
                    "❌ Inactive"
                }
            );
            println!(
                "   MinerU:     {}",
                if status.mineru_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            println!(
                "   MarkItDown: {}",
                if status.markitdown_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            println!();
            status
        }
        Err(e) => {
            println!("⚠️ Environment check failed: {e}");
            println!("Proceed with the installation...");
            println!();
            EnvironmentStatus::default()
        }
    };

    // 2. 检查是否需要安装
    let needs_setup = !env_status.uv_available
        || !env_status.virtual_env_active
        || !env_status.mineru_available
        || !env_status.markitdown_available;

    if !needs_setup {
        println!("✨ All dependencies are ready, no installation required!");
        print_success_message(&current_dir);
        return Ok(());
    }

    // 3. 显示安装计划
    println!("📋 Installation plan:");
    if !env_status.uv_available {
        println!("• Install uv tools");
    }
    if !env_status.virtual_env_active {
        println!("• Create a virtual environment (./venv/)");
    }
    if !env_status.mineru_available {
        println!("• Install MinerU dependencies");
    }
    if !env_status.markitdown_available {
        println!("• Install MarkItDown dependencies");
    }
    println!();

    // 4. 执行环境设置
    println!("⚙️ Start setting up the Python environment and dependencies...");
    println!("This may take a few minutes, please be patient...");
    println!();

    // 创建进度监控
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let env_manager_with_progress = local_env_manager.clone().with_progress_sender(progress_tx);

    // 启动进度显示任务
    let progress_task = tokio::spawn(async move {
        let mut last_package = String::new();
        let mut last_progress = 0.0;

        while let Some(progress) = progress_rx.recv().await {
            // 只在包或进度有显著变化时显示
            if progress.package != last_package || (progress.progress - last_progress).abs() > 10.0
            {
                let stage_icon = match progress.stage {
                    InstallStage::Preparing => "🔧",
                    InstallStage::Downloading => "⬇️",
                    InstallStage::Installing => "📦",
                    InstallStage::Configuring => "⚙️",
                    InstallStage::Verifying => "✅",
                    InstallStage::Completed => "🎉",
                    InstallStage::Failed(_) => "❌",
                    InstallStage::Retrying {
                        attempt,
                        max_attempts,
                    } => {
                        println!(
                            "🔄 Try again {}/{}: {}",
                            attempt, max_attempts, progress.message
                        );
                        continue;
                    }
                };

                let progress_bar = create_progress_bar(progress.progress);
                println!(
                    "   {} {} [{}] {:.0}% - {}",
                    stage_icon, progress.package, progress_bar, progress.progress, progress.message
                );

                last_package = progress.package.clone();
                last_progress = progress.progress;
            }
        }
    });

    // 预检查：诊断潜在的路径问题
    let path_issues = env_manager_with_progress.diagnose_venv_path_issues().await;
    if !path_issues.is_empty() {
        println!("⚠️ Potential routing issue detected:");
        for issue in &path_issues {
            println!("   • {issue}");
        }
        println!();

        // 尝试自动修复
        println!("🔧 Try to fix the problem automatically...");
        match env_manager_with_progress.auto_fix_venv_path_issues().await {
            Ok(fixed) => {
                if !fixed.is_empty() {
                    println!("✅ The following issues have been fixed:");
                    for fix in &fixed {
                        println!("   • {fix}");
                    }
                    println!();
                } else {
                    println!(
                        "Unable to be repaired automatically, please solve the above problem manually"
                    );
                    println!();

                    // 显示详细的恢复建议
                    let suggestions = env_manager_with_progress
                        .get_venv_recovery_suggestions()
                        .await;
                    for suggestion in suggestions {
                        println!("   {suggestion}");
                    }
                    println!();

                    return Err(anyhow::anyhow!("存在无法自动修复的路径问题"));
                }
            }
            Err(e) => {
                println!("❌ Automatic repair failed: {e}");
                println!();

                // 显示详细的恢复建议
                println!("💡 Manual repair suggestions:");
                for suggestion in e.get_path_recovery_suggestions() {
                    println!("   • {suggestion}");
                }
                println!();

                return Err(anyhow::anyhow!("路径问题修复失败: {}", e));
            }
        }
    }

    // 执行安装
    let install_result = env_manager_with_progress.setup_python_environment().await;

    // 停止进度显示
    drop(env_manager_with_progress);
    let _ = progress_task.await;

    match install_result {
        Ok(_) => {
            println!();
            println!("✅ Python environment setup completed!");
        }
        Err(e) => {
            println!();
            println!("❌ Python environment setting failed: {e}");
            println!();

            // 提供基于错误类型的详细建议
            println!("💡 Detailed troubleshooting suggestions:");
            match &e {
                AppError::VirtualEnvironmentPath(_)
                | AppError::Permission(_)
                | AppError::Path(_) => {
                    for suggestion in e.get_path_recovery_suggestions() {
                        println!("   • {suggestion}");
                    }
                }
                AppError::Environment(msg) if msg.contains("超时") => {
                    println!(
                        "• The network connection may be slow, please check the network status"
                    );
                    println!("• Try to use domestic mirror sources");
                    println!("• Increase the timeout and try again");
                }
                AppError::Environment(msg) if msg.contains("权限") => {
                    println!("• Run the command with administrator privileges");
                    println!("• Check directory permission settings");
                    if cfg!(unix) {
                        println!("• Run: chmod 755 .");
                        println!("• Run: chown $USER .");
                    }
                }
                _ => {
                    println!("• Check network connection");
                    println!("• Make sure there is enough disk space (at least 500MB)");
                    println!("• Check firewall settings");
                    println!("• Try rerunning the command");
                    println!("• Check if antivirus software is blocking the operation");
                }
            }

            // 提供诊断命令
            println!();
            println!("🔍 Diagnostic commands:");
            println!("• Check environment status: document-parser check");
            println!("• View detailed logs: Check the logs/ directory");

            return Err(anyhow::anyhow!("Python环境设置失败: {}", e));
        }
    }

    // 5. 验证安装结果
    println!();
    println!("🔍 Verify installation results...");
    match local_env_manager.check_environment().await {
        Ok(status) => {
            println!("Verification completed:");
            println!(
                "   Python:     {}",
                if status.python_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            if let Some(ref version) = status.python_version {
                println!("Version: {version}");
            }
            println!(
                "uv tool: {}",
                if status.uv_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            if let Some(ref version) = status.uv_version {
                println!("Version: {version}");
            }
            println!(
                "Virtual environment: {}",
                if status.virtual_env_active {
                    "✅ Active"
                } else {
                    "❌ Inactive"
                }
            );
            println!(
                "   MinerU:     {}",
                if status.mineru_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            if let Some(ref version) = status.mineru_version {
                println!("Version: {version}");
            }
            println!(
                "   MarkItDown: {}",
                if status.markitdown_available {
                    "✅ Available"
                } else {
                    "❌ Unavailable"
                }
            );
            if let Some(ref version) = status.markitdown_version {
                println!("Version: {version}");
            }
            println!();

            if status.is_ready() {
                print_success_message(&current_dir);
            } else {
                println!("⚠️ There may be problems with the installation of some dependencies");
                println!();
                let critical_issues = status.get_critical_issues();
                if !critical_issues.is_empty() {
                    println!("🔧 Problems that need to be solved:");
                    for issue in critical_issues {
                        println!("   • {}: {}", issue.component, issue.message);
                        println!("Suggestion: {}", issue.suggestion);
                    }
                }
                return Err(anyhow::anyhow!("环境初始化未完全成功"));
            }
        }
        Err(e) => {
            println!("❌ Verification failed: {e}");
            return Err(anyhow::anyhow!("环境验证失败: {}", e));
        }
    }

    Ok(())
}

/// 创建进度条字符串
fn create_progress_bar(progress: f32) -> String {
    let width = 20;
    let filled = ((progress / 100.0) * width as f32) as usize;
    let empty = width - filled;

    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

/// 打印成功消息和下一步指引
fn print_success_message(_current_dir: &std::path::Path) {
    println!("🎉 The uv environment initialization is completed!");
    println!();
    println!("✨ All dependencies are in place, now you can start the server");
    println!();

    // 提供激活虚拟环境的指令
    println!("📋 Virtual environment activation instructions:");

    // 检测当前shell类型并提供相应的激活命令
    if let Ok(shell) = std::env::var("SHELL") {
        if shell.contains("fish") {
            println!("   source ./venv/bin/activate.fish");
        } else {
            println!("   source ./venv/bin/activate");
        }
    } else if cfg!(windows) {
        println!("   .\\venv\\Scripts\\activate");
    } else {
        println!("   source ./venv/bin/activate");
    }

    println!();
    println!("🚀 Start the server:");
    println!("   document-parser server");
    println!();
    println!("🔧 Or use uv to run the command directly:");
    println!("   uv run mineru -h");
    println!("   uv run python -m markitdown --help");
    println!();
    println!("📚 More help:");
    println!("   document-parser --help");
    println!("document-parser check # Check environment status");
    println!("document-parser troubleshoot # Troubleshooting guide");
    println!();
    println!("💡 Tips:");
    println!("• Virtual environment location: ./venv/");
    println!(
        "• Python executable file: ./venv/bin/python (Linux/macOS) or .\\\\venv\\\\Scripts\\\\python.exe (Windows)"
    );
    println!(
        "• If you encounter problems, run 'document-parser troubleshoot' for detailed guidance"
    );
}
