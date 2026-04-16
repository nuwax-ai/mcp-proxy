//! 诊断和错误处理
//!
//! 提供错误分类、诊断报告生成等功能

/// 错误分类
pub fn classify_error(e: &anyhow::Error) -> String {
    let err_str = e.to_string().to_lowercase();

    // 特殊识别 30 秒超时（可能是服务器限制）
    if (err_str.contains("30") || err_str.contains("thirty"))
        && (err_str.contains("timeout") || err_str.contains("second") || err_str.contains("秒"))
    {
        "30-second timeout".to_string()
    }
    // 识别 503 服务不可用
    else if err_str.contains("503") || err_str.contains("service unavailable") {
        "503 Service Unavailable".to_string()
    }
    // 识别其他 HTTP 5xx 错误
    else if err_str.contains("500") || err_str.contains("internal server error") {
        "500 Internal Server Error".to_string()
    } else if err_str.contains("502") || err_str.contains("bad gateway") {
        "502 Bad Gateway".to_string()
    } else if err_str.contains("504") || err_str.contains("gateway timeout") {
        "504 Gateway Timeout".to_string()
    }
    // 识别 HTTP 4xx 错误
    else if err_str.contains("401") || err_str.contains("unauthorized") {
        "401 Unauthorized".to_string()
    } else if err_str.contains("403") || err_str.contains("forbidden") {
        "403 Forbidden".to_string()
    } else if err_str.contains("404") || err_str.contains("not found") {
        "404 Not Found".to_string()
    } else if err_str.contains("408") || err_str.contains("request timeout") {
        "408 Request Timeout".to_string()
    }
    // 通用超时
    else if err_str.contains("timeout") || err_str.contains("timed out") {
        "Timeout".to_string()
    }
    // 连接相关错误
    else if err_str.contains("connection refused") {
        "Connection Refused".to_string()
    } else if err_str.contains("connection reset") {
        "Connection Reset".to_string()
    } else if err_str.contains("eof") || err_str.contains("closed") || err_str.contains("shutdown")
    {
        "Connection Closed".to_string()
    }
    // 网络相关错误
    else if err_str.contains("dns") || err_str.contains("resolve") {
        "DNS Resolution Failed".to_string()
    } else if err_str.contains("certificate") || err_str.contains("ssl") || err_str.contains("tls")
    {
        "SSL/TLS Error".to_string()
    } else if err_str.contains("sending request") || err_str.contains("network") {
        "Network Error".to_string()
    }
    // 会话相关
    else if err_str.contains("session") {
        "Session Error".to_string()
    } else {
        "Unknown Error".to_string()
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

    eprintln!("\n=== Connection Diagnostic Report ===");
    eprintln!("Protocol: {protocol}");

    // 隐藏 URL 中的敏感信息（如 token/ak/key/secret 参数）
    let masked_url = if url.contains("?") {
        let parts: Vec<&str> = url.split('?').collect();
        if parts.len() == 2 {
            let base = parts[0];
            let params: Vec<&str> = parts[1].split('&').collect();
            let masked_params: Vec<String> = params
                .iter()
                .map(|p| {
                    let lower = p.to_lowercase();
                    let key_part = lower.split('=').next().unwrap_or("");
                    if key_part.contains("key")
                        || key_part.contains("token")
                        || key_part.contains("secret")
                        || key_part.contains("auth")
                        || key_part.contains("password")
                        || key_part.contains("passwd")
                        || key_part.contains("credential")
                        || key_part == "ak"
                        || key_part == "sk"
                    {
                        let original_key = p.split('=').next().unwrap_or("");
                        format!("{}=***", original_key)
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

    eprintln!("Service URL: {masked_url}");
    eprintln!("Connection duration: {}s", alive_duration_secs);
    eprintln!("Disconnect reason: {disconnect_reason}");

    if let Some(err_type) = error_type {
        eprintln!("Error type: {err_type}");
    }

    // 分析可能的原因
    eprintln!("\nPossible causes:");
    if (28..=32).contains(&alive_duration_secs) {
        eprintln!("  Connection dropped around 30 seconds:");
        eprintln!("     1. The backend may enforce a fixed session timeout");
        eprintln!("     2. A load balancer or gateway may be closing idle connections");
        eprintln!("     3. Keepalive/ping settings may be too weak for this environment");
    } else if alive_duration_secs < 10 {
        eprintln!("  Quick disconnect ({}s):", alive_duration_secs);
        eprintln!("     1. Service may be unavailable or misconfigured");
        eprintln!("     2. Authentication/headers may be invalid");
        eprintln!("     3. URL/path may point to a non-MCP endpoint");
    } else if alive_duration_secs >= 60 {
        eprintln!("  Long-lived connection ({}s):", alive_duration_secs);
        eprintln!("     1. Disconnect may be caused by transient network instability");
        eprintln!("     2. Backend restarts or rolling deployments may interrupt sessions");
    }

    let timeout_30s = "30-second timeout";
    let service_unavailable = "503 Service Unavailable";

    if error_type == Some(timeout_30s) || error_type == Some(service_unavailable) {
        eprintln!("\nSuggestions:");
        eprintln!("  1. Increase backend/read timeout settings");
        eprintln!("  2. Increase client ping timeout if the backend is slow");
        eprintln!("  3. Consider async/task-based invocation patterns");
        eprintln!("  4. Increase ping interval to {}s to reduce pressure", 120);
    } else if disconnect_reason.contains("Ping") || disconnect_reason.contains("ping") {
        eprintln!("\nSuggestions:");
        eprintln!("  1. Increase ping timeout to {}s", 30);
        eprintln!("  2. Increase ping interval to {}s", 60);
        eprintln!("  3. Disable ping if the backend does not support stable probes");
    }

    eprintln!("==============================\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn test_classify_error() {
        assert_eq!(classify_error(&anyhow!("connection timeout")), "Timeout");
        assert_eq!(
            classify_error(&anyhow!("connection refused")),
            "Connection Refused"
        );
        assert_eq!(
            classify_error(&anyhow!("503 Service Unavailable")),
            "503 Service Unavailable"
        );
        assert_eq!(
            classify_error(&anyhow!("401 Unauthorized")),
            "401 Unauthorized"
        );
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
