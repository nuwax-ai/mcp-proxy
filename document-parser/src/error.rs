use thiserror::Error;

/// 应用错误类型
#[derive(Error, Debug, Clone)]
pub enum AppError {
    /// 配置错误
    #[error("配置错误: {0}")]
    Config(String),

    /// 文件操作错误
    #[error("文件操作错误: {0}")]
    File(String),

    /// 格式不支持错误
    #[error("不支持的文件格式: {0}")]
    UnsupportedFormat(String),

    /// 解析错误
    #[error("解析错误: {0}")]
    Parse(String),

    /// MinerU错误
    #[error("MinerU错误: {0}")]
    MinerU(String),

    /// MarkItDown错误
    #[error("MarkItDown错误: {0}")]
    MarkItDown(String),

    /// OSS操作错误
    #[error("OSS操作错误: {0}")]
    Oss(String),

    /// 数据库错误
    #[error("数据库错误: {0}")]
    Database(String),

    /// 网络错误
    #[error("网络错误: {0}")]
    Network(String),

    /// 任务错误
    #[error("任务错误: {0}")]
    Task(String),

    /// 内部错误
    #[error("内部错误: {0}")]
    Internal(String),

    /// 超时错误
    #[error("操作超时: {0}")]
    Timeout(String),

    /// 验证错误
    #[error("验证错误: {0}")]
    Validation(String),

    /// 环境错误
    #[error("环境错误: {0}")]
    Environment(String),

    /// 虚拟环境路径错误
    #[error("虚拟环境路径错误: {0}")]
    VirtualEnvironmentPath(String),

    /// 权限错误
    #[error("权限错误: {0}")]
    Permission(String),

    /// 路径错误
    #[error("路径错误: {0}")]
    Path(String),

    /// 队列错误
    #[error("队列错误: {0}")]
    Queue(String),

    /// 处理错误
    #[error("处理错误: {0}")]
    Processing(String),
}

impl AppError {
    /// 创建虚拟环境路径错误
    pub fn virtual_environment_path_error(message: String, path: &std::path::Path) -> Self {
        AppError::VirtualEnvironmentPath(format!("{} (路径: {})", message, path.display()))
    }

    /// 创建权限错误
    pub fn permission_error(message: String, path: &std::path::Path) -> Self {
        AppError::Permission(format!("{} (路径: {})", message, path.display()))
    }

    /// 创建路径错误
    pub fn path_error(message: String, path: &std::path::Path) -> Self {
        AppError::Path(format!("{} (路径: {})", message, path.display()))
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
            },
            AppError::Permission(msg) => {
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
            },
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
            },
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

    /// 获取错误建议
    pub fn get_suggestion(&self) -> &'static str {
        match self {
            AppError::Config(_) => "检查配置文件和环境变量",
            AppError::File(_) => "检查文件路径和权限",
            AppError::UnsupportedFormat(_) => "检查文件格式是否支持",
            AppError::Parse(_) => "检查文件内容是否完整",
            AppError::MinerU(_) => "检查MinerU环境配置",
            AppError::MarkItDown(_) => "检查MarkItDown环境配置",
            AppError::Oss(_) => "检查OSS配置和网络连接",
            AppError::Database(_) => "检查数据库连接和权限",
            AppError::Network(_) => "检查网络连接和防火墙设置",
            AppError::Task(_) => "检查任务参数和状态",
            AppError::Internal(_) => "联系技术支持",
            AppError::Timeout(_) => "检查网络延迟或增加超时时间",
            AppError::Validation(_) => "检查输入参数格式",
            AppError::Environment(_) => "检查系统环境和依赖安装",
            AppError::Queue(_) => "检查队列服务状态和配置",
            AppError::Processing(_) => "检查处理流程和数据格式",
            AppError::VirtualEnvironmentPath(_) => "检查虚拟环境路径和目录权限",
            AppError::Permission(_) => "检查文件和目录权限设置",
            AppError::Path(_) => "检查路径是否存在和可访问",
        }
    }

    /// 转换为HTTP响应格式
    pub fn to_http_result<T>(&self) -> crate::models::HttpResult<T> {
        use crate::models::HttpResult;
        
        HttpResult::<T>::error(
            self.get_error_code().to_string(),
            format!("{} - {}", self.to_string(), self.get_suggestion()),
        )
    }
}

/// 从标准库错误转换
impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::File(err.to_string())
    }
}

/// 从serde错误转换
impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Parse(format!("JSON解析错误: {}", err))
    }
}

/// 从serde_yaml错误转换
impl From<serde_yaml::Error> for AppError {
    fn from(err: serde_yaml::Error) -> Self {
        AppError::Config(format!("YAML配置错误: {}", err))
    }
}

/// 从sled错误转换
impl From<sled::Error> for AppError {
    fn from(err: sled::Error) -> Self {
        AppError::Database(format!("Sled数据库错误: {}", err))
    }
}

/// 从reqwest错误转换
impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            AppError::Timeout("HTTP请求超时".to_string())
        } else if err.is_connect() {
            AppError::Network("网络连接失败".to_string())
        } else {
            AppError::Network(format!("HTTP请求错误: {}", err))
        }
    }
}

/// 从anyhow错误转换
impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err.to_string())
    }
}

/// 从std::env::VarError转换
impl From<std::env::VarError> for AppError {
    fn from(err: std::env::VarError) -> Self {
        AppError::Config(format!("环境变量错误: {}", err))
    }
}

/// 从std::num::ParseIntError转换
impl From<std::num::ParseIntError> for AppError {
    fn from(err: std::num::ParseIntError) -> Self {
        AppError::Config(format!("数字解析错误: {}", err))
    }
}

/// 从std::str::ParseBoolError转换
impl From<std::str::ParseBoolError> for AppError {
    fn from(err: std::str::ParseBoolError) -> Self {
        AppError::Config(format!("布尔值解析错误: {}", err))
    }
}

/// 从std::time::SystemTimeError转换
impl From<std::time::SystemTimeError> for AppError {
    fn from(err: std::time::SystemTimeError) -> Self {
        AppError::Internal(err.to_string())
    }
}
