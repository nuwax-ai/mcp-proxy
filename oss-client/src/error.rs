//! 错误处理模块

use thiserror::Error;

/// OSS操作错误类型
#[derive(Error, Debug)]
pub enum OssError {
    #[error("{0}")]
    Config(String),

    #[error("{0}")]
    Network(String),

    #[error("{0}")]
    FileNotFound(String),

    #[error("{0}")]
    Permission(String),

    #[error("{0}")]
    Io(String),

    #[error("{0}")]
    Sdk(String),

    #[error("{0}")]
    FileSizeExceeded(String),

    #[error("{0}")]
    UnsupportedFileType(String),

    #[error("{0}")]
    Timeout(String),

    #[error("{0}")]
    InvalidParameter(String),
}

impl OssError {
    /// 创建配置错误
    pub fn config<T: Into<String>>(msg: T) -> Self {
        Self::Config(t!("errors.oss.config", detail = msg.into()).to_string())
    }

    /// 创建网络错误
    pub fn network<T: Into<String>>(msg: T) -> Self {
        Self::Network(t!("errors.oss.network", detail = msg.into()).to_string())
    }

    /// 创建文件不存在错误
    pub fn file_not_found<T: Into<String>>(msg: T) -> Self {
        Self::FileNotFound(t!("errors.oss.file_not_found", path = msg.into()).to_string())
    }

    /// 创建权限错误
    pub fn permission<T: Into<String>>(msg: T) -> Self {
        Self::Permission(t!("errors.oss.permission", detail = msg.into()).to_string())
    }

    /// 创建SDK错误
    pub fn sdk<T: Into<String>>(msg: T) -> Self {
        Self::Sdk(t!("errors.oss.sdk", detail = msg.into()).to_string())
    }

    /// 创建文件大小超出限制错误
    pub fn file_size_exceeded<T: Into<String>>(msg: T) -> Self {
        Self::FileSizeExceeded(t!("errors.oss.file_size_exceeded", detail = msg.into()).to_string())
    }

    /// 创建不支持的文件类型错误
    pub fn unsupported_file_type<T: Into<String>>(msg: T) -> Self {
        Self::UnsupportedFileType(
            t!("errors.oss.unsupported_file_type", detail = msg.into()).to_string(),
        )
    }

    /// 创建超时错误
    pub fn timeout<T: Into<String>>(msg: T) -> Self {
        Self::Timeout(t!("errors.oss.timeout", detail = msg.into()).to_string())
    }

    /// 创建无效参数错误
    pub fn invalid_parameter<T: Into<String>>(msg: T) -> Self {
        Self::InvalidParameter(t!("errors.oss.invalid_parameter", detail = msg.into()).to_string())
    }

    /// 创建IO错误
    pub fn io_error<T: Into<String>>(msg: T) -> Self {
        Self::Io(t!("errors.oss.io", detail = msg.into()).to_string())
    }

    /// 判断是否为配置错误
    pub fn is_config_error(&self) -> bool {
        matches!(self, Self::Config(_))
    }

    /// 判断是否为网络错误
    pub fn is_network_error(&self) -> bool {
        matches!(self, Self::Network(_))
    }

    /// 判断是否为文件不存在错误
    pub fn is_file_not_found(&self) -> bool {
        matches!(self, Self::FileNotFound(_))
    }

    /// 判断是否为权限错误
    pub fn is_permission_error(&self) -> bool {
        matches!(self, Self::Permission(_))
    }
}

/// 从标准库错误转换
impl From<std::io::Error> for OssError {
    fn from(err: std::io::Error) -> Self {
        Self::io_error(err.to_string())
    }
}

/// Result类型别名
pub type Result<T> = std::result::Result<T, OssError>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_error_creation() {
        let config_err = OssError::config("test config error");
        assert!(config_err.is_config_error());
        assert_eq!(config_err.to_string(), "配置错误: test config error");

        let network_err = OssError::network("test network error");
        assert!(network_err.is_network_error());
        assert_eq!(network_err.to_string(), "网络错误: test network error");

        let file_err = OssError::file_not_found("test.txt");
        assert!(file_err.is_file_not_found());
        assert_eq!(file_err.to_string(), "文件不存在: test.txt");

        let permission_err = OssError::permission("access denied");
        assert!(permission_err.is_permission_error());
        assert_eq!(permission_err.to_string(), "权限不足: access denied");
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let oss_err: OssError = io_err.into();

        match oss_err {
            OssError::Io(_) => {} // 正确
            _ => panic!("IO错误转换失败"),
        }
    }

    #[test]
    fn test_error_display() {
        let err = OssError::FileSizeExceeded("文件大小超过100MB".to_string());
        assert_eq!(err.to_string(), "文件大小超出限制: 文件大小超过100MB");

        let err = OssError::UnsupportedFileType("不支持.xyz格式".to_string());
        assert_eq!(err.to_string(), "不支持的文件类型: 不支持.xyz格式");

        let err = OssError::Timeout("操作超时30秒".to_string());
        assert_eq!(err.to_string(), "操作超时: 操作超时30秒");

        let err = OssError::InvalidParameter("object_key不能为空".to_string());
        assert_eq!(err.to_string(), "无效的参数: object_key不能为空");
    }

    #[test]
    fn test_error_type_checking() {
        let config_err = OssError::Config("test".to_string());
        assert!(config_err.is_config_error());
        assert!(!config_err.is_network_error());
        assert!(!config_err.is_file_not_found());
        assert!(!config_err.is_permission_error());

        let network_err = OssError::Network("test".to_string());
        assert!(!network_err.is_config_error());
        assert!(network_err.is_network_error());
        assert!(!network_err.is_file_not_found());
        assert!(!network_err.is_permission_error());
    }
}
