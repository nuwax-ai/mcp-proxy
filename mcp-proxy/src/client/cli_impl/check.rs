//! 检查和检测命令
//!
//! 实现服务状态检查和协议检测功能

use anyhow::Result;

use crate::client::support::{CheckArgs, DetectArgs};

/// 运行检查命令
pub async fn run_check_command(args: CheckArgs, _verbose: bool, quiet: bool) -> Result<()> {
    if !quiet {
        eprintln!("Checking service health: {}", &args.url);
    }

    match crate::client::protocol::detect_mcp_protocol(&args.url).await {
        Ok(protocol) => {
            if !quiet {
                eprintln!("Service is healthy (protocol: {protocol})");
            }
            Ok(())
        }
        Err(e) => {
            if !quiet {
                eprintln!("Service check failed: {e}");
            }
            Err(e)
        }
    }
}

/// 运行协议检测命令
pub async fn run_detect_command(args: DetectArgs, _verbose: bool, quiet: bool) -> Result<()> {
    let protocol = crate::client::protocol::detect_mcp_protocol(&args.url).await?;

    if quiet {
        println!("{}", protocol);
    } else {
        eprintln!("{}", protocol);
    }

    Ok(())
}
