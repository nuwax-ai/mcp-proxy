//! `check` / `install` 子命令：环境检查与依赖安装。

use anyhow::Result;
use document_parser::{AppError, utils::environment_manager::EnvironmentManager};
use log::{error, info, warn};

/// 处理环境检查命令
pub async fn handle_check_command(environment_manager: &EnvironmentManager) -> Result<()> {
    info!("Check Python environment status...");

    // 首先进行路径诊断
    println!("🔍 Diagnose virtual environment path...");
    let path_issues = environment_manager.diagnose_venv_path_issues().await;
    if !path_issues.is_empty() {
        println!("⚠️ Found path related issues:");
        for issue in &path_issues {
            println!("   • {issue}");
        }
        println!();

        println!("💡 Suggestions for solving path problems:");
        let suggestions = environment_manager.get_venv_recovery_suggestions().await;
        for suggestion in suggestions {
            println!("   {suggestion}");
        }
        println!();
    } else {
        println!("✅Virtual environment path check passed");
        println!();
    }

    match environment_manager.get_detailed_status_report().await {
        Ok(detailed_report) => {
            // 输出详细的诊断报告
            println!("{detailed_report}");

            // 输出增强的依赖验证报告
            println!("🔬 Perform enhanced dependency verification...");
            match environment_manager.get_enhanced_dependency_report().await {
                Ok(enhanced_report) => {
                    println!("{enhanced_report}");
                }
                Err(e) => {
                    println!("⚠️ Enhanced dependency verification failed: {e}");
                }
            }

            // 检查环境状态以确定退出码
            match environment_manager.check_environment().await {
                Ok(status) => {
                    if status.is_ready() {
                        println!(
                            "✅ Environmental inspection passed! All dependencies are in place."
                        );
                        Ok(())
                    } else {
                        let critical_issues = status.get_critical_issues();
                        if !critical_issues.is_empty() {
                            println!(
                                "❌ Found {} key issues that need to be resolved",
                                critical_issues.len()
                            );
                            for issue in critical_issues {
                                println!("  • {}: {}", issue.component, issue.message);
                                println!("Suggestion: {}", issue.suggestion);
                            }
                        }

                        let auto_fixable = status.get_auto_fixable_issues();
                        if !auto_fixable.is_empty() {
                            println!(
                                "💡 {} problems can be fixed automatically, run 'document-parser uv-init' to fix them",
                                auto_fixable.len()
                            );
                        }

                        // 如果有路径问题，提供额外的建议
                        if !path_issues.is_empty() {
                            println!();
                            println!("🔧 Path problem fix:");
                            println!(
                                "• Running 'document-parser uv-init' will try to fix path issues automatically"
                            );
                            println!(
                                "• Or manually solve the path problem by following the suggestions above"
                            );
                        }

                        Err(anyhow::anyhow!(
                            "环境未就绪，健康评分: {}/100",
                            status.health_score()
                        ))
                    }
                }
                Err(e) => {
                    println!("❌ Environment status check failed: {e}");

                    // 如果是路径相关错误，提供详细建议
                    match &e {
                        AppError::VirtualEnvironmentPath(_)
                        | AppError::Permission(_)
                        | AppError::Path(_) => {
                            println!();
                            println!("💡 Suggestions for solving path errors:");
                            for suggestion in e.get_path_recovery_suggestions() {
                                println!("   • {suggestion}");
                            }
                        }
                        _ => {}
                    }

                    Err(anyhow::anyhow!("环境状态检查失败: {}", e))
                }
            }
        }
        Err(e) => {
            println!("❌ Environment check failed: {e}");

            // 如果是路径相关错误，提供详细建议
            match &e {
                AppError::VirtualEnvironmentPath(_)
                | AppError::Permission(_)
                | AppError::Path(_) => {
                    println!();
                    println!("💡 Suggestions for solving path errors:");
                    for suggestion in e.get_path_recovery_suggestions() {
                        println!("   • {suggestion}");
                    }
                }
                _ => {}
            }

            Err(anyhow::anyhow!("环境检查失败: {}", e))
        }
    }
}

/// 处理依赖安装命令
pub async fn handle_install_command(environment_manager: &EnvironmentManager) -> Result<()> {
    info!("Start installing Python dependencies...");

    match environment_manager.setup_python_environment().await {
        Ok(_) => {
            info!("Python dependency installation is complete!");

            // 验证安装结果
            match environment_manager.check_environment().await {
                Ok(status) => {
                    if status.mineru_available && status.markitdown_available {
                        info!(
                            "The installation verification was successful and all dependencies are in place!"
                        );
                    } else {
                        warn!(
                            "The installation is completed but verification fails. Some dependencies may not be installed correctly."
                        );
                    }
                }
                Err(e) => {
                    warn!("Installation completed but verification failed: {e}");
                }
            }
        }
        Err(e) => {
            error!("Python dependency installation failed: {e}");
            return Err(anyhow::anyhow!("Python依赖安装失败: {}", e));
        }
    }

    Ok(())
}
