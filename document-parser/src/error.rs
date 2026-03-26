use thiserror::Error;

/// 应用错误类型
#[derive(Error, Debug, Clone)]
pub enum AppError {
    /// 配置错误
    #[error("{0}")]
    Config(String),

    /// 文件操作错误
    #[error("{0}")]
    File(String),

    /// 格式不支持错误
    #[error("{0}")]
    UnsupportedFormat(String),

    /// 解析错误
    #[error("{0}")]
    Parse(String),

    /// MinerU错误
    #[error("{0}")]
    MinerU(String),

    /// MarkItDown错误
    #[error("{0}")]
    MarkItDown(String),

    /// OSS操作错误
    #[error("{0}")]
    Oss(String),

    /// 数据库错误
    #[error("{0}")]
    Database(String),

    /// 网络错误
    #[error("{0}")]
    Network(String),

    /// 任务错误
    #[error("{0}")]
    Task(String),

    /// 内部错误
    #[error("{0}")]
    Internal(String),

    /// 超时错误
    #[error("{0}")]
    Timeout(String),

    /// 验证错误
    #[error("{0}")]
    Validation(String),

    /// 环境错误
    #[error("{0}")]
    Environment(String),

    /// 虚拟环境路径错误
    #[error("{0}")]
    VirtualEnvironmentPath(String),

    /// 权限错误
    #[error("{0}")]
    Permission(String),

    /// 路径错误
    #[error("{0}")]
    Path(String),

    /// 队列错误
    #[error("{0}")]
    Queue(String),

    /// 处理错误
    #[error("{0}")]
    Processing(String),
}

impl AppError {
    // ===========================================
    // 工厂方法 - 创建带国际化消息的错误
    // ===========================================

    /// 创建配置错误
    pub fn config_error(detail: impl Into<String>) -> Self {
        Self::Config(t!("errors.document_parser.config", detail = detail.into()).to_string())
    }

    /// 创建文件操作错误
    pub fn file_error(detail: impl Into<String>) -> Self {
        Self::File(t!("errors.document_parser.file", detail = detail.into()).to_string())
    }

    /// 创建格式不支持错误
    pub fn unsupported_format(format: impl Into<String>) -> Self {
        Self::UnsupportedFormat(t!("errors.document_parser.unsupported_format", format = format.into()).to_string())
    }

    /// 创建解析错误
    pub fn parse_error(detail: impl Into<String>) -> Self {
        Self::Parse(t!("errors.document_parser.parse", detail = detail.into()).to_string())
    }

    /// 创建 MinerU 错误
    pub fn mineru_error(detail: impl Into<String>) -> Self {
        Self::MinerU(t!("errors.document_parser.mineru", detail = detail.into()).to_string())
    }

    /// 创建 MarkItDown 错误
    pub fn markitdown_error(detail: impl Into<String>) -> Self {
        Self::MarkItDown(t!("errors.document_parser.markitdown", detail = detail.into()).to_string())
    }

    /// 创建 OSS 错误
    pub fn oss_error(detail: impl Into<String>) -> Self {
        Self::Oss(t!("errors.document_parser.oss", detail = detail.into()).to_string())
    }

    /// 创建数据库错误
    pub fn database_error(detail: impl Into<String>) -> Self {
        Self::Database(t!("errors.document_parser.database", detail = detail.into()).to_string())
    }

    /// 创建网络错误
    pub fn network_error(detail: impl Into<String>) -> Self {
        Self::Network(t!("errors.document_parser.network", detail = detail.into()).to_string())
    }

    /// 创建任务错误
    pub fn task_error(detail: impl Into<String>) -> Self {
        Self::Task(t!("errors.document_parser.task", detail = detail.into()).to_string())
    }

    /// 创建内部错误
    pub fn internal_error(detail: impl Into<String>) -> Self {
        Self::Internal(t!("errors.document_parser.internal", detail = detail.into()).to_string())
    }

    /// 创建超时错误
    pub fn timeout_error(detail: impl Into<String>) -> Self {
        Self::Timeout(t!("errors.document_parser.timeout", detail = detail.into()).to_string())
    }

    /// 创建验证错误
    pub fn validation_error(detail: impl Into<String>) -> Self {
        Self::Validation(t!("errors.document_parser.validation", detail = detail.into()).to_string())
    }

    /// 创建环境错误
    pub fn environment_error(detail: impl Into<String>) -> Self {
        Self::Environment(t!("errors.document_parser.environment", detail = detail.into()).to_string())
    }

    /// 创建虚拟环境路径错误
    pub fn virtual_environment_path_error(message: String, path: &std::path::Path) -> Self {
        let path_str = path.display().to_string();
        Self::VirtualEnvironmentPath(
            t!("errors.document_parser.virtual_environment_path", detail = format!("{} (path: {})", message, path_str)).to_string()
        )
    }

    /// 创建权限错误
    pub fn permission_error(message: String, path: &std::path::Path) -> Self {
        let path_str = path.display().to_string();
        Self::Permission(
            t!("errors.document_parser.permission", detail = format!("{} (path: {})", message, path_str)).to_string()
        )
    }

    /// 创建路径错误
    pub fn path_error(message: String, path: &std::path::Path) -> Self {
        let path_str = path.display().to_string();
        Self::Path(
            t!("errors.document_parser.path", detail = format!("{} (path: {})", message, path_str)).to_string()
        )
    }

    /// 创建队列错误
    pub fn queue_error(detail: impl Into<String>) -> Self {
        Self::Queue(t!("errors.document_parser.queue", detail = detail.into()).to_string())
    }

    /// 创建处理错误
    pub fn processing_error(detail: impl Into<String>) -> Self {
        Self::Processing(t!("errors.document_parser.processing", detail = detail.into()).to_string())
    }

    /// 获取路径相关错误的详细恢复建议
    pub fn get_path_recovery_suggestions(&self) -> Vec<String> {
        match self {
            AppError::VirtualEnvironmentPath(msg) => {
                let mut suggestions = vec![
                    "检查当前目录是否有写入权限".to_string(),
                    "确保当前目录下没有名为 'venv' 的文件（非目录）".to_string(),
                    "尝试删除损坏的虚拟环境目录: rm -rf ./venv".to_string(),
                    "检查磁盘空间是否充足".to_string(),
                ];

                if msg.contains("权限") || msg.contains("permission") {
                    suggestions.insert(0, "使用 sudo 或管理员权限运行命令".to_string());
                    suggestions.push("检查目录所有者和权限: ls -la".to_string());
                }

                if msg.contains("存在") || msg.contains("exists") {
                    suggestions.push("备份现有虚拟环境后重新创建".to_string());
                }

                suggestions
            }
            AppError::Permission(_msg) => {
                let mut suggestions = vec![
                    "检查文件和目录权限设置".to_string(),
                    "确保当前用户有足够的权限".to_string(),
                ];

                if cfg!(unix) {
                    suggestions.extend(vec![
                        "使用 chmod 修改权限: chmod 755 <目录>".to_string(),
                        "使用 chown 修改所有者: chown $USER <目录>".to_string(),
                        "检查 SELinux 或 AppArmor 安全策略".to_string(),
                    ]);
                } else if cfg!(windows) {
                    suggestions.extend(vec![
                        "以管理员身份运行命令提示符".to_string(),
                        "检查 Windows 用户账户控制 (UAC) 设置".to_string(),
                        "确保目录不在受保护的系统路径中".to_string(),
                    ]);
                }

                suggestions
            }
            AppError::Path(msg) => {
                let mut suggestions = vec![
                    "检查路径是否正确拼写".to_string(),
                    "确保路径存在且可访问".to_string(),
                    "检查路径中是否包含特殊字符".to_string(),
                ];

                if msg.contains("不存在") || msg.contains("not found") {
                    suggestions.push("创建缺失的目录结构".to_string());
                }

                if msg.contains("长度") || msg.contains("length") {
                    suggestions.push("使用较短的路径名称".to_string());
                }

                suggestions
            }
            _ => vec!["检查系统环境和配置".to_string()],
        }
    }

    /// 获取错误代码
    pub fn get_error_code(&self) -> &'static str {
        match self {
            AppError::Config(_) => "E001",
            AppError::File(_) => "E002",
            AppError::UnsupportedFormat(_) => "E003",
            AppError::Parse(_) => "E004",
            AppError::MinerU(_) => "E005",
            AppError::MarkItDown(_) => "E006",
            AppError::Oss(_) => "E007",
            AppError::Database(_) => "E008",
            AppError::Network(_) => "E009",
            AppError::Task(_) => "E010",
            AppError::Internal(_) => "E011",
            AppError::Timeout(_) => "E012",
            AppError::Validation(_) => "E013",
            AppError::Environment(_) => "E014",
            AppError::Queue(_) => "E015",
            AppError::Processing(_) => "E016",
            AppError::VirtualEnvironmentPath(_) => "E017",
            AppError::Permission(_) => "E018",
            AppError::Path(_) => "E019",
        }
    }

    /// 获取错误建议（国际化）
    pub fn get_suggestion(&self) -> String {
        match self {
            AppError::Config(_) => t!("errors.document_parser.suggestions.config").to_string(),
            AppError::File(_) => t!("errors.document_parser.suggestions.file").to_string(),
            AppError::UnsupportedFormat(_) => t!("errors.document_parser.suggestions.unsupported_format").to_string(),
            AppError::Parse(_) => t!("errors.document_parser.suggestions.parse").to_string(),
            AppError::MinerU(_) => t!("errors.document_parser.suggestions.mineru").to_string(),
            AppError::MarkItDown(_) => t!("errors.document_parser.suggestions.markitdown").to_string(),
            AppError::Oss(_) => t!("errors.document_parser.suggestions.oss").to_string(),
            AppError::Database(_) => t!("errors.document_parser.suggestions.database").to_string(),
            AppError::Network(_) => t!("errors.document_parser.suggestions.network").to_string(),
            AppError::Task(_) => t!("errors.document_parser.suggestions.task").to_string(),
            AppError::Internal(_) => t!("errors.document_parser.suggestions.internal").to_string(),
            AppError::Timeout(_) => t!("errors.document_parser.suggestions.timeout").to_string(),
            AppError::Validation(_) => t!("errors.document_parser.suggestions.validation").to_string(),
            AppError::Environment(_) => t!("errors.document_parser.suggestions.environment").to_string(),
            AppError::Queue(_) => t!("errors.document_parser.suggestions.queue").to_string(),
            AppError::Processing(_) => t!("errors.document_parser.suggestions.processing").to_string(),
            AppError::VirtualEnvironmentPath(_) => t!("errors.document_parser.suggestions.virtual_environment_path").to_string(),
            AppError::Permission(_) => t!("errors.document_parser.suggestions.permission").to_string(),
            AppError::Path(_) => t!("errors.document_parser.suggestions.path").to_string(),
        }
    }

    /// 转换为HTTP响应格式
    pub fn to_http_result<T>(&self) -> crate::models::HttpResult<T> {
        use crate::models::HttpResult;

        HttpResult::<T>::error(
            self.get_error_code().to_string(),
            format!("{} - {}", self, self.get_suggestion()),
        )
    }
}

/// 从标准库错误转换
impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::file_error(err.to_string())
    }
}

/// 从serde错误转换
impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        Self::parse_error(format!("JSON: {err}"))
    }
}

/// 从serde_yaml错误转换
impl From<serde_yaml::Error> for AppError {
    fn from(err: serde_yaml::Error) -> Self {
        Self::config_error(format!("YAML: {err}"))
    }
}

/// 从sled错误转换
impl From<sled::Error> for AppError {
    fn from(err: sled::Error) -> Self {
        Self::database_error(format!("Sled: {err}"))
    }
}

/// 从reqwest错误转换
impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            Self::timeout_error("HTTP request")
        } else if err.is_connect() {
            Self::network_error("connection failed")
        } else {
            Self::network_error(err.to_string())
        }
    }
}

/// 从anyhow错误转换
impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        Self::internal_error(err.to_string())
    }
}

/// 从std::env::VarError转换
impl From<std::env::VarError> for AppError {
    fn from(err: std::env::VarError) -> Self {
        Self::config_error(format!("environment variable: {err}"))
    }
}

/// 从std::num::ParseIntError转换
impl From<std::num::ParseIntError> for AppError {
    fn from(err: std::num::ParseIntError) -> Self {
        Self::config_error(format!("integer parse: {err}"))
    }
}

/// 从std::str::ParseBoolError转换
impl From<std::str::ParseBoolError> for AppError {
    fn from(err: std::str::ParseBoolError) -> Self {
        Self::config_error(format!("boolean parse: {err}"))
    }
}

/// 从std::time::SystemTimeError转换
impl From<std::time::SystemTimeError> for AppError {
    fn from(err: std::time::SystemTimeError) -> Self {
        Self::internal_error(err.to_string())
    }
}
