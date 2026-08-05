//! CLI 命令定义与各子命令处理模块。
//!
//! `main.rs` 只负责进程入口与 server 模式，各子命令的实现分布在子模块中：
//! - [`check`]：环境检查 / 依赖安装
//! - [`locale`]：按环境变量初始化 locale
//! - [`parse`]：单文件解析
//! - [`service`]：systemd 服务注册
//! - [`troubleshoot`]：故障排除指南
//! - [`uv_init`]：uv 虚拟环境初始化

pub mod check;
pub mod locale;
pub mod parse;
pub mod service;
pub mod troubleshoot;
pub mod uv_init;

use clap::{Parser, Subcommand};
use document_parser::APP_VERSION;
use std::path::PathBuf;

/// Document Parser - 文档解析服务
///
/// 使用当前目录虚拟环境 (./venv/) 进行Python依赖管理
///
/// 快速开始:
///   1. document-parser uv-init    # 初始化虚拟环境和依赖
///   2. document-parser server     # 启动服务器
///
/// 虚拟环境激活:
///   source ./venv/bin/activate    # Linux/macOS
///   .\venv\Scripts\activate       # Windows
#[derive(Parser)]
#[command(name = "document-parser")]
#[command(about = "A document parsing service with CLI support")]
#[command(version = APP_VERSION)]
#[command(long_about = "
Document Parser 是一个多格式文档解析服务，支持PDF、Word、Excel、PowerPoint等格式。

环境管理:
  本服务使用当前目录下的虚拟环境 (./venv/) 来管理Python依赖。
  首次使用请运行 'document-parser uv-init' 来自动设置环境。

支持的格式:
  • PDF (通过 MinerU 引擎)
  • Word, Excel, PowerPoint (通过 MarkItDown 引擎)
  • Markdown, HTML, Text 等

故障排除:
  • 运行 'document-parser check' 检查环境状态
  • 运行 'document-parser troubleshoot' 获取详细故障排除指南
  • 查看日志文件: logs/ 目录
")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// 配置文件路径
    #[arg(short, long)]
    pub config: Option<PathBuf>,

    /// 服务器端口
    #[arg(short, long)]
    pub port: Option<u16>,

    /// 服务器主机地址
    #[arg(long, default_value = "0.0.0.0")]
    pub host: String,
}

#[derive(Subcommand)]
pub enum Commands {
    /// 启动服务器模式
    Server {
        /// 后台运行
        #[arg(short, long)]
        daemon: bool,
    },
    /// 解析单个文件
    Parse {
        /// 输入文件路径
        #[arg(short, long)]
        input: PathBuf,
        /// 输出文件路径
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// 解析器类型 (mineru, markitdown)
        #[arg(short, long, default_value = "mineru")]
        parser: String,
    },
    /// 检查环境状态和虚拟环境配置
    #[command(long_about = "
检查Python环境、虚拟环境状态和依赖安装情况。

检查内容:
  • Python版本和可用性
  • UV工具安装状态
  • 虚拟环境 (./venv/) 状态
  • MinerU和MarkItDown依赖
  • CUDA支持情况
  • 路径和权限问题诊断

输出详细的诊断报告和修复建议。")]
    Check,
    /// 安装依赖环境 (已弃用，请使用 uv-init)
    #[command(hide = true)]
    Install,
    /// 初始化当前目录的uv虚拟环境和依赖
    #[command(name = "uv-init")]
    #[command(about = "在当前目录初始化uv虚拟环境，自动安装mineru和markitdown依赖")]
    #[command(long_about = "
在当前工作目录创建虚拟环境 (./venv/) 并安装所需的Python依赖。

执行步骤:
  1. 检查并安装UV工具 (如果缺失)
  2. 在当前目录创建虚拟环境: ./venv/
  3. 安装MinerU依赖: uv pip install -U \"mineru[core]\"
  4. 安装MarkItDown依赖: uv pip install markitdown
  5. 验证安装结果

完成后可以使用以下命令激活虚拟环境:
  Linux/macOS: source ./venv/bin/activate
  Windows:     .\\venv\\Scripts\\activate

然后启动服务器: document-parser server")]
    UvInit,
    /// 显示详细的故障排除指南
    #[command(about = "显示虚拟环境和依赖问题的详细故障排除指南")]
    #[command(long_about = "
显示常见问题的详细故障排除指南，包括:

虚拟环境问题:
  • 虚拟环境创建失败
  • 路径和权限问题
  • 依赖安装失败
  • 跨平台兼容性问题

网络和下载问题:
  • 网络连接超时
  • 包下载失败
  • 镜像源配置

系统环境问题:
  • Python版本兼容性
  • UV工具安装
  • CUDA环境配置

每个问题都包含详细的诊断步骤和解决方案。")]
    Troubleshoot,
    /// systemd service registration (Linux)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
pub enum ServiceAction {
    /// Generate unit, install to /etc/systemd/system, enable + start
    Install {
        /// Install root (WorkingDirectory); default: current directory
        #[arg(long, default_value = ".")]
        install_dir: PathBuf,

        /// systemd User= (default: current user)
        #[arg(long)]
        user: Option<String>,

        /// Register + enable but do not start/restart
        #[arg(long)]
        no_start: bool,

        /// Only print rendered unit; do not write or call systemctl
        #[arg(long)]
        dry_run: bool,
    },
    /// Stop, disable, and remove the unit
    Uninstall,
    /// Show enable/active state, unit, and recent logs
    Status,
    /// Restart the service
    Restart,
}
