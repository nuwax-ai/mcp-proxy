//! 诊断和错误处理
//!
//! 提供错误分类、诊断报告生成等功能

use mcp_common::t;

/// 错误分类
pub fn classify_error(e: &anyhow::Error) -> String {
    let err_str = e.to_string().to_lowercase();

    // 特殊识别 30 秒超时（可能是服务器限制）
    if (err_str.contains("30") || err_str.contains("thirty"))
        && (err_str.contains("timeout") || err_str.contains("second") || err_str.contains("秒"))
    {
        t!("error_classify.timeout_30s").to_string()
    }
    // 识别 503 服务不可用
    else if err_str.contains("503") || err_str.contains("service unavailable") {
        t!("error_classify.service_unavailable_503").to_string()
    }
    // 识别其他 HTTP 5xx 错误
    else if err_str.contains("500") || err_str.contains("internal server error") {
        t!("error_classify.internal_server_error_500").to_string()
    } else if err_str.contains("502") || err_str.contains("bad gateway") {
        t!("error_classify.bad_gateway_502").to_string()
    } else if err_str.contains("504") || err_str.contains("gateway timeout") {
        t!("error_classify.gateway_timeout_504").to_string()
    }
    // 识别 HTTP 4xx 错误
    else if err_str.contains("401") || err_str.contains("unauthorized") {
        t!("error_classify.unauthorized_401").to_string()
    } else if err_str.contains("403") || err_str.contains("forbidden") {
        t!("error_classify.forbidden_403").to_string()
    } else if err_str.contains("404") || err_str.contains("not found") {
        t!("error_classify.not_found_404").to_string()
    } else if err_str.contains("408") || err_str.contains("request timeout") {
        t!("error_classify.request_timeout_408").to_string()
    }
    // 通用超时
    else if err_str.contains("timeout") || err_str.contains("timed out") {
        t!("error_classify.timeout").to_string()
    }
    // 连接相关错误
    else if err_str.contains("connection refused") {
        t!("error_classify.connection_refused").to_string()
    } else if err_str.contains("connection reset") {
        t!("error_classify.connection_reset").to_string()
    } else if err_str.contains("eof") || err_str.contains("closed") || err_str.contains("shutdown")
    {
        t!("error_classify.connection_closed").to_string()
    }
    // 网络相关错误
    else if err_str.contains("dns") || err_str.contains("resolve") {
        t!("error_classify.dns_failed").to_string()
    } else if err_str.contains("certificate") || err_str.contains("ssl") || err_str.contains("tls")
    {
        t!("error_classify.ssl_tls_error").to_string()
    } else if err_str.contains("sending request") || err_str.contains("network") {
        t!("error_classify.network_error").to_string()
    }
    // 会话相关
    else if err_str.contains("session") {
        t!("error_classify.session_error").to_string()
    } else {
        t!("error_classify.unknown_error").to_string()
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

    eprintln!("\n{}", t!("diagnostic.report_header"));
    eprintln!("{}", t!("diagnostic.connection_protocol", protocol = protocol));

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

    eprintln!("{}", t!("diagnostic.service_url", url = masked_url));
    eprintln!("{}", t!("diagnostic.connection_duration", seconds = alive_duration_secs));
    eprintln!("{}", t!("diagnostic.disconnect_reason", reason = disconnect_reason));

    if let Some(err_type) = error_type {
        eprintln!("{}", t!("diagnostic.error_type", error_type = err_type));
    }

    // 分析可能的原因
    eprintln!("\n{}", t!("diagnostic.possible_causes"));
    if (28..=32).contains(&alive_duration_secs) {
        eprintln!("  {}", t!("diagnostic.analysis.timeout_30s"));
        eprintln!("     1. {}", t!("diagnostic.analysis.timeout_30s_cause1"));
        eprintln!("     2. {}", t!("diagnostic.analysis.timeout_30s_cause2"));
        eprintln!("     3. {}", t!("diagnostic.analysis.timeout_30s_cause3"));
    } else if alive_duration_secs < 10 {
        eprintln!("  {}", t!("diagnostic.analysis.quick_disconnect", seconds = alive_duration_secs));
        eprintln!("     1. {}", t!("diagnostic.analysis.quick_disconnect_cause1"));
        eprintln!("     2. {}", t!("diagnostic.analysis.quick_disconnect_cause2"));
        eprintln!("     3. {}", t!("diagnostic.analysis.quick_disconnect_cause3"));
    } else if alive_duration_secs >= 60 {
        eprintln!("  {}", t!("diagnostic.analysis.long_connection", seconds = alive_duration_secs));
        eprintln!("     1. {}", t!("diagnostic.analysis.long_connection_cause1"));
        eprintln!("     2. {}", t!("diagnostic.analysis.long_connection_cause2"));
    }

    // 获取翻译后的错误类型用于比较
    let timeout_30s = t!("error_classify.timeout_30s").to_string();
    let service_unavailable = t!("error_classify.service_unavailable_503").to_string();

    if error_type == Some(&timeout_30s) || error_type == Some(&service_unavailable) {
        eprintln!("\n{}", t!("diagnostic.suggestions"));
        eprintln!("  1. {}", t!("diagnostic.suggestion.timeout_30s"));
        eprintln!("  2. {}", t!("diagnostic.suggestion.timeout_client"));
        eprintln!("  3. {}", t!("diagnostic.suggestion.async_mode"));
        eprintln!("  4. {}", t!("diagnostic.suggestion.ping_interval", seconds = 120));
    } else if disconnect_reason.contains("Ping") || disconnect_reason.contains("ping") {
        eprintln!("\n{}", t!("diagnostic.suggestions"));
        eprintln!("  1. {}", t!("diagnostic.suggestion.ping_timeout", seconds = 30));
        eprintln!("  2. {}", t!("diagnostic.suggestion.ping_interval", seconds = 60));
        eprintln!("  3. {}", t!("diagnostic.suggestion.ping_disable"));
    }

    eprintln!("==============================\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn test_classify_error() {
        // 初始化语言设置为英文进行测试
        mcp_common::set_locale("en");

        assert_eq!(classify_error(&anyhow!("connection timeout")), t!("error_classify.timeout").to_string());
        assert_eq!(classify_error(&anyhow!("connection refused")), t!("error_classify.connection_refused").to_string());
        assert_eq!(
            classify_error(&anyhow!("503 Service Unavailable")),
            t!("error_classify.service_unavailable_503").to_string()
        );
        assert_eq!(classify_error(&anyhow!("401 Unauthorized")), t!("error_classify.unauthorized_401").to_string());
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
