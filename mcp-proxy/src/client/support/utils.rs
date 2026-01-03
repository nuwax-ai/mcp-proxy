//! 通用工具函数

/// 获取协议名称
pub fn protocol_name(protocol: &crate::client::protocol::McpProtocol) -> &'static str {
    match protocol {
        crate::client::protocol::McpProtocol::Sse => "SSE",
        crate::client::protocol::McpProtocol::Stream => "Streamable HTTP",
        crate::client::protocol::McpProtocol::Stdio => "Stdio",
    }
}

/// 截断字符串（UTF-8 安全）
pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        format!("{}...", s.chars().take(max_len - 3).collect::<String>())
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::protocol::McpProtocol;

    #[test]
    fn test_protocol_name() {
        assert_eq!(protocol_name(&McpProtocol::Sse), "SSE");
        assert_eq!(protocol_name(&McpProtocol::Stream), "Streamable HTTP");
        assert_eq!(protocol_name(&McpProtocol::Stdio), "Stdio");
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 8), "hello...");
        assert_eq!(truncate_str("你好世界", 3), "...");
        assert_eq!(truncate_str("你好世界", 6), "你好世界");
    }
}
