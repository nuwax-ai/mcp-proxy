//! `troubleshoot` 子命令：显示详细故障排除指南。

use anyhow::{Context as _, Result};
use document_parser::utils::environment_manager::EnvironmentManager;
use std::env;

/// 处理故障排除命令
pub async fn handle_troubleshoot_command(environment_manager: &EnvironmentManager) -> Result<()> {
    println!("🔧 Document Parser Troubleshooting Guide");
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    // 显示当前环境概览
    println!("📊 Current environment overview:");
    let current_dir = env::current_dir().context("无法获取当前目录")?;
    println!("Working directory: {}", current_dir.display());
    println!("Virtual environment: ./venv/");
    println!(
        "Operating system: {}",
        if cfg!(windows) {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "Linux"
        }
    );
    println!();

    // 1. 虚拟环境问题
    println!("🏠 1. Virtual environment issues");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    println!("❓ Problem: Virtual environment creation failed");
    println!("🔍 Diagnosis steps:");
    println!("1. Check the current directory permissions: ls -la (Linux/macOS) or dir (Windows)");
    println!("2. Check disk space: df -h (Linux/macOS) or dir (Windows)");
    println!("3. Check whether a file with the same name exists: ls -la venv");
    println!();
    println!("💡 Solution:");
    println!("• Make sure the current directory has write permissions");
    if cfg!(unix) {
        println!("• Modify permissions: chmod 755.");
        println!("• Change owner: chown $USER .");
    } else if cfg!(windows) {
        println!("• Run command prompt as administrator");
        println!("• Check User Account Control (UAC) settings");
    }
    println!(
        "• Delete existing venv files: rm -rf ./venv (Linux/macOS) or rmdir /s .\\\\venv (Windows)"
    );
    println!("• Make sure there is at least 500MB of free disk space");
    println!();

    println!("❓ Problem: Virtual environment activation failed");
    println!("🔍 Diagnosis steps:");
    println!(
        "1. Check whether the virtual environment exists: ls ./venv/bin/ (Linux/macOS) or dir .\\\\venv\\\\Scripts\\\\ (Windows)"
    );
    println!("2. Check activation script permissions");
    println!();
    println!("💡 Solution:");
    if cfg!(windows) {
        println!("   • Windows: .\\venv\\Scripts\\activate");
        println!("   • PowerShell: .\\venv\\Scripts\\Activate.ps1");
        println!(
            "• If PowerShell enforcement policy restrictions, run: Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser"
        );
    } else {
        println!("   • Bash/Zsh: source ./venv/bin/activate");
        println!("   • Fish: source ./venv/bin/activate.fish");
        println!("• Check script permissions: chmod +x ./venv/bin/activate");
    }
    println!();

    // 2. 依赖安装问题
    println!("📦 2. Dependency installation issues");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    println!("❓ Problem: UV tool is not installed or unavailable");
    println!("💡 Solution:");
    println!(
        "• Use the official installation script: curl -LsSf https://astral.sh/uv/install.sh | sh"
    );
    println!("• Or install using pip: pip install uv");
    println!("• Or use a package manager:");
    if cfg!(target_os = "macos") {
        println!("     - macOS: brew install uv");
    } else if cfg!(unix) {
        println!("- Ubuntu/Debian: See https://docs.astral.sh/uv/getting-started/installation/");
    } else if cfg!(windows) {
        println!("     - Windows: winget install astral-sh.uv");
    }
    println!("• Restart the terminal and try again");
    println!();

    println!("❓ Problem: MinerU or MarkItDown installation failed");
    println!("🔍 Diagnosis steps:");
    println!("1. Check network connection: ping pypi.org");
    println!("2. Check Python version: python --version (requires 3.8+)");
    println!("3. Check pip in the virtual environment: ./venv/bin/pip --version");
    println!();
    println!("💡 Solution:");
    println!("• Use domestic mirror sources:");
    println!("     uv pip install -i https://pypi.tuna.tsinghua.edu.cn/simple/ mineru[core]");
    println!("• Increase timeout: uv pip install --timeout 300 mineru[core]");
    println!("• Step-by-step installation:");
    println!("     1. uv pip install --upgrade pip");
    println!("     2. uv pip install mineru[core]");
    println!("     3. uv pip install markitdown");
    println!("• Clean the cache and try again: uv cache clean");
    println!();

    // 3. 网络和下载问题
    println!("🌐 3. Network and download issues");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    println!("❓ Problem: Network connection timed out or download failed");
    println!("💡 Solution:");
    println!("• Check network connections and firewall settings");
    println!("• Using a proxy (if required):");
    println!("     export HTTP_PROXY=http://proxy:port");
    println!("     export HTTPS_PROXY=http://proxy:port");
    println!("• Use domestic mirror sources:");
    println!("- Tsinghua source: https://pypi.tuna.tsinghua.edu.cn/simple/");
    println!("- Ali source: https://mirrors.aliyun.com/pypi/simple/");
    println!("• Retry installation: document-parser uv-init");
    println!();

    // 4. 系统环境问题
    println!("⚙️ 4. System environment issues");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    println!("❓ Problem: Python version is incompatible");
    println!("🔍 Check command: python --version or python3 --version");
    println!("💡 Solution:");
    println!("• Requires Python 3.8 or higher");
    if cfg!(target_os = "macos") {
        println!("• macOS installation: brew install python@3.11");
    } else if cfg!(unix) {
        println!("   • Ubuntu/Debian: sudo apt update && sudo apt install python3.11");
        println!("   • CentOS/RHEL: sudo yum install python311");
    } else if cfg!(windows) {
        println!("• Windows: Download and install from https://python.org");
    }
    println!();

    println!("❓ Question: CUDA environment configuration (optional, for GPU acceleration)");
    println!("🔍 Check command: nvidia-smi");
    println!("💡 Solution:");
    println!("• Install NVIDIA driver");
    println!("• Install CUDA Toolkit (11.8 or 12.x recommended)");
    println!("• Verify installation: nvidia-smi and nvcc --version");
    println!("• Note: CPU mode also works normally, GPU is only used for acceleration");
    println!();

    // 5. 常用诊断命令
    println!("🔍 5. Common diagnostic commands");
    println!("───────────────────────────────────────────────────────────────");
    println!();
    println!("Environmental inspection:");
    println!("document-parser check # Complete environment check");
    println!("document-parser uv-init # Reinitialize the environment");
    println!();
    println!("Manual verification:");
    println!("uv --version # Check UV version");
    println!("./venv/bin/python --version # Check virtual environment Python (Linux/macOS)");
    println!(
        ".\\\\venv\\\\Scripts\\\\python --version # Check the virtual environment Python (Windows)"
    );
    println!("./venv/bin/mineru --help # Check MinerU (Linux/macOS)");
    println!(".\\\\venv\\\\Scripts\\\\mineru --help # Check MinerU (Windows)");
    println!();
    println!("Log view:");
    println!("tail -f logs/log.$(date +%Y-%m-%d) # View today’s logs (Linux/macOS)");
    println!("type logs\\\\log.%date:~0,10% # View today’s log (Windows)");
    println!();

    // 6. 获取帮助
    println!("🆘 6. Get more help");
    println!("───────────────────────────────────────────────────────────────");
    println!();
    println!("If none of the above resolves the issue, please:");
    println!("1. Run detailed diagnostics: document-parser check");
    println!("2. Collect error information:");
    println!("• Complete error message");
    println!("• Operating system version");
    println!("• Python version");
    println!("• Current working directory");
    println!("3. View log files: logs/ directory");
    println!("4. Try reinitializing in a new directory");
    println!();

    // 执行实时诊断
    println!("🔬 Real-time environment diagnosis");
    println!("───────────────────────────────────────────────────────────────");
    match environment_manager.check_environment().await {
        Ok(status) => {
            if status.is_ready() {
                println!(
                    "✅ The environment is in good condition and all dependencies are in place"
                );
            } else {
                println!("⚠️ Found the following issues:");
                let issues = status.get_critical_issues();
                for issue in issues {
                    println!("   • {}: {}", issue.component, issue.message);
                    println!("Suggestion: {}", issue.suggestion);
                }
            }
        }
        Err(e) => {
            println!("❌ Environment check failed: {e}");
            println!("Please follow the above guide to troubleshoot");
        }
    }

    println!();
    println!("═══════════════════════════════════════════════════════════════");
    println!("💡 Tip: Most problems can be solved by re-running 'document-parser uv-init'");

    Ok(())
}
