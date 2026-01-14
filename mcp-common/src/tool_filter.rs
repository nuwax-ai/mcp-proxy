//! 工具过滤器
//!
//! 提供白名单和黑名单两种过滤模式

use std::collections::HashSet;

/// 工具过滤配置
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ToolFilter {
    /// 白名单（只允许这些工具）
    pub allow_tools: Option<HashSet<String>>,
    /// 黑名单（排除这些工具）
    pub deny_tools: Option<HashSet<String>>,
}

impl ToolFilter {
    /// 创建白名单过滤器
    pub fn allow(tools: Vec<String>) -> Self {
        Self {
            allow_tools: Some(tools.into_iter().collect()),
            deny_tools: None,
        }
    }

    /// 创建黑名单过滤器
    pub fn deny(tools: Vec<String>) -> Self {
        Self {
            allow_tools: None,
            deny_tools: Some(tools.into_iter().collect()),
        }
    }

    /// 检查工具是否被允许
    pub fn is_allowed(&self, tool_name: &str) -> bool {
        // 白名单模式：只有在白名单中的工具才被允许
        if let Some(ref allow_list) = self.allow_tools {
            return allow_list.contains(tool_name);
        }
        // 黑名单模式：不在黑名单中的工具都被允许
        if let Some(ref deny_list) = self.deny_tools {
            return !deny_list.contains(tool_name);
        }
        // 无过滤：全部允许
        true
    }

    /// 检查是否启用了过滤
    pub fn is_enabled(&self) -> bool {
        self.allow_tools.is_some() || self.deny_tools.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allow_filter() {
        let filter = ToolFilter::allow(vec!["tool1".to_string(), "tool2".to_string()]);
        assert!(filter.is_allowed("tool1"));
        assert!(filter.is_allowed("tool2"));
        assert!(!filter.is_allowed("tool3"));
    }

    #[test]
    fn test_deny_filter() {
        let filter = ToolFilter::deny(vec!["tool1".to_string()]);
        assert!(!filter.is_allowed("tool1"));
        assert!(filter.is_allowed("tool2"));
        assert!(filter.is_allowed("tool3"));
    }

    #[test]
    fn test_no_filter() {
        let filter = ToolFilter::default();
        assert!(filter.is_allowed("any_tool"));
        assert!(!filter.is_enabled());
    }
}
