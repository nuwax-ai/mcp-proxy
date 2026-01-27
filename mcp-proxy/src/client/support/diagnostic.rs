//! 诊断和错误处理
//!
//! 提供错误分类、诊断报告生成等功能

/// 错误分类
pub fn classify_error(e: &anyhow::Error) -> &'static str {
    let err_str = e.to_string().to_lowercase();

    // 特殊识别 30 秒超时（可能是服务器限制）
    if (err_str.contains("30") || err_str.contains("thirty"))
        && (err_str.contains("timeout") || err_str.contains("second") || err_str.contains("秒"))
    {
        "30秒超时（可能是服务器限制）"
    }
    // 识别 503 服务不可用
    else if err_str.contains("503") || err_str.contains("service unavailable") {
        "服务不可用(503)"
    }
    // 识别其他 HTTP 5xx 错误
    else if err_str.contains("500") || err_str.contains("internal server error") {
        "服务器内部错误(500)"
    } else if err_str.contains("502") || err_str.contains("bad gateway") {
        "网关错误(502)"
    } else if err_str.contains("504") || err_str.contains("gateway timeout") {
        "网关超时(504)"
    }
    // 识别 HTTP 4xx 错误
    else if err_str.contains("401") || err_str.contains("unauthorized") {
        "未授权(401)"
    } else if err_str.contains("403") || err_str.contains("forbidden") {
        "禁止访问(403)"
    } else if err_str.contains("404") || err_str.contains("not found") {
        "资源未找到(404)"
    } else if err_str.contains("408") || err_str.contains("request timeout") {
        "请求超时(408)"
    }
    // 通用超时
    else if err_str.contains("timeout") || err_str.contains("timed out") {
        "超时"
    }
    // 连接相关错误
    else if err_str.contains("connection refused") {
        "连接被拒绝"
    } else if err_str.contains("connection reset") {
        "连接被重置"
    } else if err_str.contains("eof") || err_str.contains("closed") || err_str.contains("shutdown")
    {
        "连接关闭"
    }
    // 网络相关错误
    else if err_str.contains("dns") || err_str.contains("resolve") {
        "DNS解析失败"
    } else if err_str.contains("certificate") || err_str.contains("ssl") || err_str.contains("tls")
    {
        "SSL/TLS错误"
    } else if err_str.contains("sending request") || err_str.contains("network") {
        "网络错误"
    }
    // 会话相关
    else if err_str.contains("session") {
        "会话错误"
    } else {
        "未知错误"
    }
}

/// 简化错误信息（用于单行日志）
pub fn summarize_error(e: &anyhow::Error) -> String {
    let full = e.to_string();
    // 截取第一行或前80个字符
    let first_line = full.lines().next().unwrap_or(&full);
    // 使用 chars() 安全处理 UTF-8 字符，避免在多字节字符中间截断
    if first_line.chars().count() > 80 {
        format!("{}...", first_line.chars().take(77).collect::<String>())
    } else {
        first_line.to_string()
    }
}

/// 生成诊断报告
pub fn print_diagnostic_report(
    protocol: &str,
    url: &str,
    alive_duration_secs: u64,
    disconnect_reason: &str,
    error_type: Option<&str>,
    diagnostic: bool,
) {
    if !diagnostic {
        return;
    }

    eprintln!("\n========== 诊断报告 ==========");
    eprintln!("连接协议: {}", protocol);

    // 隐藏 URL 中的敏感信息（如 token/ak 参数）
    let masked_url = if url.contains("?") {
        let parts: Vec<&str> = url.split('?').collect();
        if parts.len() == 2 {
            let base = parts[0];
            let params: Vec<&str> = parts[1].split('&').collect();
            let masked_params: Vec<String> = params
                .iter()
                .map(|p| {
                    if p.starts_with("ak=") || p.starts_with("token=") || p.starts_with("auth=") {
                        let key = p.split('=').next().unwrap_or("");
                        format!("{}=***", key)
                    } else {
                        p.to_string()
                    }
                })
                .collect();
            format!("{}?{}", base, masked_params.join("&"))
        } else {
            url.to_string()
        }
    } else {
        url.to_string()
    };

    eprintln!("服务 URL: {}", masked_url);
    eprintln!("连接存活时长: {} 秒", alive_duration_secs);
    eprintln!("断开原因: {}", disconnect_reason);

    if let Some(err_type) = error_type {
        eprintln!("错误类型: {}", err_type);
    }

    // 分析可能的原因
    eprintln!("\n可能原因分析:");
    if (28..=32).contains(&alive_duration_secs) {
        eprintln!("  ⚠️  连接在约 30 秒时断开，极有可能是:");
        eprintln!("     1. 服务器端设置了 30 秒超时限制");
        eprintln!("     2. 负载均衡器（如 Nginx/ALB）的默认超时");
        eprintln!("     3. 云服务商的网关超时限制");
    } else if alive_duration_secs < 10 {
        eprintln!("  ⚠️  连接很快断开（{}秒），可能是:", alive_duration_secs);
        eprintln!("     1. 认证失败或 token 无效");
        eprintln!("     2. 服务器拒绝连接");
        eprintln!("     3. 网络不稳定");
    } else if alive_duration_secs >= 60 {
        eprintln!(
            "  ✅ 连接保持了较长时间（{}秒），可能是:",
            alive_duration_secs
        );
        eprintln!("     1. 工具调用执行时间过长");
        eprintln!("     2. 网络波动导致断开");
    }

    if error_type == Some("30秒超时（可能是服务器限制）") || error_type == Some("服务不可用(503)")
    {
        eprintln!("\n建议:");
        eprintln!("  1. 联系服务提供商增加超时限制");
        eprintln!("  2. 使用 --request-timeout 参数设置客户端超时");
        eprintln!("  3. 考虑使用异步处理模式（webhook 回调）");
        eprintln!("  4. 尝试增加 ping 间隔: --ping-interval 120");
    } else if disconnect_reason.contains("Ping 检测超时") {
        eprintln!("\n建议:");
        eprintln!("  1. 增加 ping 超时时间: --ping-timeout 30");
        eprintln!("  2. 增加 ping 间隔: --ping-interval 60");
        eprintln!("  3. 或禁用 ping: --ping-interval 0");
    }

    eprintln!("==============================\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn test_classify_error() {
        assert_eq!(classify_error(&anyhow!("connection timeout")), "超时");
        assert_eq!(classify_error(&anyhow!("connection refused")), "连接被拒绝");
        assert_eq!(
            classify_error(&anyhow!("503 Service Unavailable")),
            "服务不可用(503)"
        );
        assert_eq!(classify_error(&anyhow!("401 Unauthorized")), "未授权(401)");
    }

    #[test]
    fn test_summarize_error() {
        let short_err = anyhow!("short error");
        assert_eq!(summarize_error(&short_err), "short error");

        let long_err = anyhow!(
            "this is a very long error message that exceeds eighty characters and should be truncated"
        );
        let summary = summarize_error(&long_err);
        assert!(summary.len() <= 80);
        assert!(summary.ends_with("..."));
    }
}
