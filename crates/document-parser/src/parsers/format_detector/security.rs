//! `FormatDetector` 的安全检查族（从 format_detector.rs 拆出）：文件安全校验
//! （大小/路径检查）与按格式的安全状态评估，及 `SecurityConfig` 的构建。
//! 纯代码搬移，无行为变化。

use crate::config::{FileSizePurpose, get_file_size_limit};
use crate::models::DocumentFormat;
use anyhow::{Result, bail};
use std::path::Path;

use super::{SecurityConfig, SecurityStatus};

impl super::FormatDetector {
    /// 验证文件安全性
    pub(super) fn validate_file_security(&self, file_path: &str) -> Result<()> {
        if !self.security_config.enable_size_check
            && !self.security_config.enable_filename_validation
        {
            return Ok(());
        }

        let path = Path::new(file_path);

        // 文件名验证
        if self.security_config.enable_filename_validation {
            if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
                let ext_lower = extension.to_lowercase();
                if self
                    .security_config
                    .dangerous_extensions
                    .contains(&ext_lower)
                {
                    bail!("危险文件扩展名: {}", extension);
                }
            }

            // 检查文件名中的危险字符
            if let Some(filename) = path.file_name().and_then(|name| name.to_str())
                && (filename.contains("..") || filename.contains("/") || filename.contains("\\"))
            {
                bail!("文件名包含危险字符: {}", filename);
            }
        }

        // 文件大小检查
        if self.security_config.enable_size_check
            && let Ok(metadata) = std::fs::metadata(file_path)
        {
            let file_size = metadata.len();
            if file_size > self.security_config.max_allowed_size {
                bail!(
                    "文件大小超过限制: {} bytes (最大: {} bytes)",
                    file_size,
                    self.security_config.max_allowed_size
                );
            }
        }

        Ok(())
    }

    /// 评估安全状态
    pub(super) fn assess_security_status(
        &self,
        format: &DocumentFormat,
        file_path: &str,
    ) -> SecurityStatus {
        let path = Path::new(file_path);

        // 检查文件扩展名
        if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
            let ext_lower = extension.to_lowercase();
            if self
                .security_config
                .dangerous_extensions
                .contains(&ext_lower)
            {
                return SecurityStatus::Dangerous(format!("危险文件扩展名: {extension}"));
            }
        }

        // 根据格式评估安全性
        match format {
            DocumentFormat::PDF
            | DocumentFormat::Word
            | DocumentFormat::Excel
            | DocumentFormat::PowerPoint => SecurityStatus::Safe,
            DocumentFormat::Image | DocumentFormat::Audio => SecurityStatus::Safe,
            DocumentFormat::HTML => SecurityStatus::Suspicious("HTML文件可能包含脚本".to_string()),
            DocumentFormat::Text | DocumentFormat::Txt | DocumentFormat::Md => SecurityStatus::Safe,
            DocumentFormat::Other(_) => SecurityStatus::Unknown,
        }
    }
}

impl SecurityConfig {
    /// 使用全局配置创建安全配置
    pub fn with_global_config() -> Self {
        // 安全地获取文件大小限制，如果全局配置未初始化则使用默认值
        let max_allowed_size = std::panic::catch_unwind(|| {
            get_file_size_limit(&FileSizePurpose::FormatDetector).bytes()
        })
        .unwrap_or(100 * 1024 * 1024);

        Self {
            enable_size_check: true,
            enable_malware_detection: false,
            enable_filename_validation: true,
            max_allowed_size,
            dangerous_extensions: vec![
                "exe".to_string(),
                "bat".to_string(),
                "cmd".to_string(),
                "scr".to_string(),
                "com".to_string(),
                "pif".to_string(),
                "vbs".to_string(),
                "js".to_string(),
                "jar".to_string(),
            ],
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self::with_global_config()
    }
}
