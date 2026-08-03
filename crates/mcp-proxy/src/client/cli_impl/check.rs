//! 检查和检测命令
//!
//! 实现服务状态检查和协议检测功能

use std::collections::HashMap;

use anyhow::Result;

use crate::client::protocol::detect_mcp_protocol_with_headers;
use crate::client::support::{CheckArgs, DetectArgs, merge_headers};

/// 运行检查命令
pub async fn run_check_command(args: CheckArgs, _verbose: bool, quiet: bool) -> Result<()> {
    if !quiet {
        eprintln!("Checking service health: {}", args.url);
    }

    // 合并 --auth 与 -H 自定义 headers 用于协议探测。
    // 注意：空 map 传入 Some(&_) 与 None 行为等价（is_sse_with_headers 内部按 is_empty 判定），
    // 因此无需 if-empty 桥接。
    let headers = merge_headers(HashMap::new(), &args.header, args.auth.as_ref());
    match detect_mcp_protocol_with_headers(&args.url, Some(&headers)).await {
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
    let headers = merge_headers(HashMap::new(), &args.header, args.auth.as_ref());
    let protocol = detect_mcp_protocol_with_headers(&args.url, Some(&headers)).await?;

    if quiet {
        println!("{}", protocol);
    } else {
        eprintln!("Detected protocol: {}", protocol);
        println!("{}", protocol);
    }

    Ok(())
}
