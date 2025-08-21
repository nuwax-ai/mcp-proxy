use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::RwLock;
use tracing::{info, instrument, warn};
use tracing_subscriber::{
    EnvFilter, Registry,
    util::SubscriberInitExt,
};
use uuid::Uuid;

/// 日志级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Trace => write!(f, "TRACE"),
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
        }
    }
}

impl From<&str> for LogLevel {
    fn from(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "TRACE" => LogLevel::Trace,
            "DEBUG" => LogLevel::Debug,
            "INFO" => LogLevel::Info,
            "WARN" => LogLevel::Warn,
            "ERROR" => LogLevel::Error,
            _ => LogLevel::Info,
        }
    }
}

/// 关联ID管理器
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct CorrelationContext {
    pub request_id: Option<String>,
    pub task_id: Option<String>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
}


impl CorrelationContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_request_id(mut self, request_id: String) -> Self {
        self.request_id = Some(request_id);
        self
    }

    pub fn with_task_id(mut self, task_id: String) -> Self {
        self.task_id = Some(task_id);
        self
    }

    pub fn with_user_id(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }

    pub fn with_session_id(mut self, session_id: String) -> Self {
        self.session_id = Some(session_id);
        self
    }

    pub fn with_trace_id(mut self, trace_id: String) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    pub fn with_span_id(mut self, span_id: String) -> Self {
        self.span_id = Some(span_id);
        self
    }

    pub fn generate_request_id(&mut self) -> String {
        let id = Uuid::new_v4().to_string();
        self.request_id = Some(id.clone());
        id
    }

    pub fn generate_trace_id(&mut self) -> String {
        let id = Uuid::new_v4().to_string();
        self.trace_id = Some(id.clone());
        id
    }

    pub fn to_fields(&self) -> HashMap<String, String> {
        let mut fields = HashMap::new();

        if let Some(ref request_id) = self.request_id {
            fields.insert("request_id".to_string(), request_id.clone());
        }
        if let Some(ref task_id) = self.task_id {
            fields.insert("task_id".to_string(), task_id.clone());
        }
        if let Some(ref user_id) = self.user_id {
            fields.insert("user_id".to_string(), user_id.clone());
        }
        if let Some(ref session_id) = self.session_id {
            fields.insert("session_id".to_string(), session_id.clone());
        }
        if let Some(ref trace_id) = self.trace_id {
            fields.insert("trace_id".to_string(), trace_id.clone());
        }
        if let Some(ref span_id) = self.span_id {
            fields.insert("span_id".to_string(), span_id.clone());
        }

        fields
    }
}

/// 结构化日志条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: String,
    pub timestamp: SystemTime,
    pub level: LogLevel,
    pub message: String,
    pub module: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub target: String,
    pub fields: HashMap<String, serde_json::Value>,
    pub correlation: CorrelationContext,
    pub service_name: String,
    pub service_version: String,
    pub environment: String,
}

impl LogEntry {
    pub fn new(level: LogLevel, message: String, target: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: SystemTime::now(),
            level,
            message,
            module: None,
            file: None,
            line: None,
            target,
            fields: HashMap::new(),
            correlation: CorrelationContext::default(),
            service_name: "document-parser".to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            environment: std::env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string()),
        }
    }

    /// 添加字段
    pub fn with_field<T: Serialize>(mut self, key: &str, value: T) -> Self {
        if let Ok(json_value) = serde_json::to_value(value) {
            self.fields.insert(key.to_string(), json_value);
        }
        self
    }

    /// 设置关联上下文
    pub fn with_correlation(mut self, correlation: CorrelationContext) -> Self {
        self.correlation = correlation;
        self
    }

    /// 设置服务信息
    pub fn with_service_info(mut self, name: String, version: String, environment: String) -> Self {
        self.service_name = name;
        self.service_version = version;
        self.environment = environment;
        self
    }

    /// 设置源码位置
    pub fn with_location(mut self, module: String, file: String, line: u32) -> Self {
        self.module = Some(module);
        self.file = Some(file);
        self.line = Some(line);
        self
    }

    /// 脱敏处理
    pub fn sanitize(&mut self) {
        // 脱敏消息中的敏感信息
        self.message = self.sanitize_string(&self.message);

        // 脱敏字段中的敏感信息
        let mut sanitized_fields = HashMap::new();
        for (key, value) in &self.fields {
            let sanitized_value = match value {
                serde_json::Value::String(s) => serde_json::Value::String(self.sanitize_string(s)),
                _ => value.clone(),
            };
            sanitized_fields.insert(key.clone(), sanitized_value);
        }
        self.fields = sanitized_fields;
    }

    /// 脱敏字符串
    fn sanitize_string(&self, input: &str) -> String {
        let mut result = input.to_string();

        // 脱敏常见的敏感信息模式
        let patterns = vec![
            (r"password[\s]*[:=][\s]*[\S]+", "password: ***"),
            (r"token[\s]*[:=][\s]*[\S]+", "token: ***"),
            (r"key[\s]*[:=][\s]*[\S]+", "key: ***"),
            (r"secret[\s]*[:=][\s]*[\S]+", "secret: ***"),
            (
                r"\b\d{4}[\s-]?\d{4}[\s-]?\d{4}[\s-]?\d{4}\b",
                "****-****-****-****",
            ), // 信用卡号
            (r"\b\d{3}-\d{2}-\d{4}\b", "***-**-****"), // SSN
        ];

        for (pattern, replacement) in patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                result = re.replace_all(&result, replacement).to_string();
            }
        }

        result
    }

    /// 格式化为JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// 格式化为人类可读格式
    pub fn to_human_readable(&self) -> String {
        let timestamp = self
            .timestamp
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let location = if let (Some(file), Some(line)) = (&self.file, self.line) {
            format!(" [{file}:{line}]")
        } else {
            String::new()
        };

        let context = if !self.fields.is_empty() {
            format!(
                " {}",
                serde_json::to_string(&self.fields).unwrap_or_default()
            )
        } else {
            String::new()
        };

        format!(
            "{} [{}] {}{}: {}{}",
            timestamp, self.level, self.target, location, self.message, context
        )
    }
}

/// 日志输出器trait
#[async_trait::async_trait]
pub trait LogOutput: Send + Sync {
    async fn write_log(
        &self,
        entry: &LogEntry,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn flush(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// 控制台日志输出器
pub struct ConsoleOutput {
    use_json: bool,
}

impl ConsoleOutput {
    pub fn new(use_json: bool) -> Self {
        Self { use_json }
    }
}

#[async_trait::async_trait]
impl LogOutput for ConsoleOutput {
    async fn write_log(
        &self,
        entry: &LogEntry,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let output = if self.use_json {
            entry.to_json()?
        } else {
            entry.to_human_readable()
        };

        println!("{}", output);
        Ok(())
    }

    async fn flush(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use std::io::{self, Write};
        io::stdout().flush()?;
        Ok(())
    }
}

/// 文件日志输出器
pub struct FileOutput {
    file_path: String,
    use_json: bool,
    max_file_size: u64,
    max_files: usize,
}

impl FileOutput {
    pub fn new(file_path: String, use_json: bool, max_file_size: u64, max_files: usize) -> Self {
        Self {
            file_path,
            use_json,
            max_file_size,
            max_files,
        }
    }

    /// 检查并轮转日志文件
    async fn rotate_if_needed(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use std::fs;

        if let Ok(metadata) = fs::metadata(&self.file_path) {
            if metadata.len() > self.max_file_size {
                // 轮转日志文件
                for i in (1..self.max_files).rev() {
                    let old_file = format!("{}.{}", self.file_path, i);
                    let new_file = format!("{}.{}", self.file_path, i + 1);

                    if fs::metadata(&old_file).is_ok() {
                        fs::rename(&old_file, &new_file)?;
                    }
                }

                // 移动当前文件
                let backup_file = format!("{}.1", self.file_path);
                fs::rename(&self.file_path, &backup_file)?;
            }
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl LogOutput for FileOutput {
    async fn write_log(
        &self,
        entry: &LogEntry,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use std::fs::OpenOptions;
        use std::io::Write;

        self.rotate_if_needed().await?;

        let output = if self.use_json {
            format!("{}\n", entry.to_json()?)
        } else {
            format!("{}\n", entry.to_human_readable())
        };

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)?;

        file.write_all(output.as_bytes())?;
        file.flush()?;

        Ok(())
    }

    async fn flush(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 文件输出器在每次写入时都会刷新
        Ok(())
    }
}

/// 日志配置
#[derive(Debug, Clone)]
pub struct LoggingConfig {
    pub level: String,
    pub format: LogFormat,
    pub output: LogOutputTarget,
    pub file_path: Option<String>,
    pub max_file_size: u64,
    pub max_files: usize,
    pub enable_console: bool,
    pub enable_json: bool,
    pub enable_correlation: bool,
    pub service_name: String,
    pub service_version: String,
    pub environment: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: LogFormat::Human,
            output: LogOutputTarget::Console,
            file_path: None,
            max_file_size: 100 * 1024 * 1024, // 100MB
            max_files: 10,
            enable_console: true,
            enable_json: false,
            enable_correlation: true,
            service_name: "document-parser".to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            environment: std::env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string()),
        }
    }
}

/// 日志格式
#[derive(Debug, Clone, PartialEq)]
pub enum LogFormat {
    Human,
    Json,
    Compact,
}

/// 日志输出目标
#[derive(Debug, Clone, PartialEq)]
pub enum LogOutputTarget {
    Console,
    File,
    Both,
}

/// 增强的日志系统
pub struct EnhancedLoggingSystem {
    config: LoggingConfig,
    correlation_context: Arc<RwLock<CorrelationContext>>,
    _guards: Vec<tracing_appender::non_blocking::WorkerGuard>,
}

impl EnhancedLoggingSystem {
    /// 初始化日志系统
    #[instrument(skip(config))]
    pub fn init(config: LoggingConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let guards = Vec::new();
        let layers: Vec<Box<dyn tracing_subscriber::Layer<Registry> + Send + Sync>> =
            Vec::new();

        // 设置环境过滤器
        let env_filter = EnvFilter::try_from_default_env()
            .or_else(|_| EnvFilter::try_new(&config.level))
            .unwrap_or_else(|_| EnvFilter::new("info"));

        // 控制台输出层
        if config.enable_console
            && (config.output == LogOutputTarget::Console || config.output == LogOutputTarget::Both)
        {
            // 简化的控制台层配置
            // 在实际实现中，这里需要更复杂的配置
        }

        // 文件输出层
        if let Some(ref file_path) = config.file_path {
            if config.output == LogOutputTarget::File || config.output == LogOutputTarget::Both {
                // 简化的文件层配置
                // 在实际实现中，这里需要更复杂的配置
            }
        }

        // 简化的订阅者初始化
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true)
            .init();

        info!(
            service_name = %config.service_name,
            service_version = %config.service_version,
            environment = %config.environment,
            log_level = %config.level,
            "日志系统初始化完成"
        );

        Ok(Self {
            config,
            correlation_context: Arc::new(RwLock::new(CorrelationContext::default())),
            _guards: guards,
        })
    }

    /// 设置关联上下文
    pub async fn set_correlation_context(&self, context: CorrelationContext) {
        let mut correlation = self.correlation_context.write().await;
        *correlation = context;
    }

    /// 获取关联上下文
    pub async fn get_correlation_context(&self) -> CorrelationContext {
        let correlation = self.correlation_context.read().await;
        correlation.clone()
    }

    /// 生成新的请求ID
    pub async fn generate_request_id(&self) -> String {
        let mut correlation = self.correlation_context.write().await;
        correlation.generate_request_id()
    }

    /// 生成新的跟踪ID
    pub async fn generate_trace_id(&self) -> String {
        let mut correlation = self.correlation_context.write().await;
        correlation.generate_trace_id()
    }

    /// 创建带有关联上下文的span
    pub async fn create_span(&self, name: &str) -> tracing::Span {
        let correlation = self.correlation_context.read().await;
        let fields = correlation.to_fields();

        let span = tracing::info_span!(
            "custom_span",
            name = name,
            service_name = %self.config.service_name,
            service_version = %self.config.service_version,
            environment = %self.config.environment,
        );

        // 添加关联字段
        for (key, value) in fields {
            span.record(key.as_str(), tracing::field::display(&value));
        }

        span
    }

    /// 获取配置
    pub fn config(&self) -> &LoggingConfig {
        &self.config
    }
}

/// 结构化日志记录器（保持向后兼容）
pub struct StructuredLogger {
    min_level: LogLevel,
    outputs: Vec<Arc<dyn LogOutput>>,
    context: Arc<RwLock<HashMap<String, serde_json::Value>>>,
}

impl StructuredLogger {
    /// 创建新的结构化日志器
    pub fn new(min_level: LogLevel) -> Self {
        Self {
            min_level,
            outputs: Vec::new(),
            context: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 添加输出器
    pub fn add_output(mut self, output: Arc<dyn LogOutput>) -> Self {
        self.outputs.push(output);
        self
    }

    /// 设置全局上下文
    pub async fn set_context<T: Serialize>(&self, key: &str, value: T) {
        if let Ok(json_value) = serde_json::to_value(value) {
            let mut context = self.context.write().await;
            context.insert(key.to_string(), json_value);
        }
    }

    /// 移除全局上下文
    pub async fn remove_context(&self, key: &str) {
        let mut context = self.context.write().await;
        context.remove(key);
    }

    /// 记录日志
    pub async fn log(&self, mut entry: LogEntry) {
        if entry.level < self.min_level {
            return;
        }

        // 添加全局上下文
        {
            let context = self.context.read().await;
            for (key, value) in context.iter() {
                entry.fields.insert(key.clone(), value.clone());
            }
        }

        // 脱敏处理
        entry.sanitize();

        // 输出到所有输出器
        for output in &self.outputs {
            if let Err(e) = output.write_log(&entry).await {
                eprintln!("日志输出错误: {e}");
            }
        }
    }

    /// 刷新所有输出器
    pub async fn flush(&self) {
        for output in &self.outputs {
            if let Err(e) = output.flush().await {
                eprintln!("日志刷新错误: {e}");
            }
        }
    }

    /// 便捷方法：记录trace级别日志
    pub async fn trace(&self, message: &str, target: &str) {
        let entry = LogEntry::new(LogLevel::Trace, message.to_string(), target.to_string());
        self.log(entry).await;
    }

    /// 便捷方法：记录debug级别日志
    pub async fn debug(&self, message: &str, target: &str) {
        let entry = LogEntry::new(LogLevel::Debug, message.to_string(), target.to_string());
        self.log(entry).await;
    }

    /// 便捷方法：记录info级别日志
    pub async fn info(&self, message: &str, target: &str) {
        let entry = LogEntry::new(LogLevel::Info, message.to_string(), target.to_string());
        self.log(entry).await;
    }

    /// 便捷方法：记录warn级别日志
    pub async fn warn(&self, message: &str, target: &str) {
        let entry = LogEntry::new(LogLevel::Warn, message.to_string(), target.to_string());
        self.log(entry).await;
    }

    /// 便捷方法：记录error级别日志
    pub async fn error(&self, message: &str, target: &str) {
        let entry = LogEntry::new(LogLevel::Error, message.to_string(), target.to_string());
        self.log(entry).await;
    }
}

/// 日志宏
#[macro_export]
macro_rules! structured_log {
    ($logger:expr, $level:expr, $message:expr) => {
        {
            let entry = $crate::utils::logging::LogEntry::new(
                $level,
                $message.to_string(),
                module_path!().to_string(),
            ).with_location(
                module_path!().to_string(),
                file!().to_string(),
                line!(),
            );
            $logger.log(entry).await;
        }
    };

    ($logger:expr, $level:expr, $message:expr, $($key:expr => $value:expr),+) => {
        {
            let mut entry = $crate::utils::logging::LogEntry::new(
                $level,
                $message.to_string(),
                module_path!().to_string(),
            ).with_location(
                module_path!().to_string(),
                file!().to_string(),
                line!(),
            );

            $(
                entry = entry.with_field($key, $value);
            )+

            $logger.log(entry).await;
        }
    };
}

/// 便捷宏
#[macro_export]
macro_rules! log_trace {
    ($logger:expr, $message:expr) => {
        structured_log!($logger, $crate::utils::logging::LogLevel::Trace, $message)
    };
    ($logger:expr, $message:expr, $($key:expr => $value:expr),+) => {
        structured_log!($logger, $crate::utils::logging::LogLevel::Trace, $message, $($key => $value),+)
    };
}

#[macro_export]
macro_rules! log_debug {
    ($logger:expr, $message:expr) => {
        structured_log!($logger, $crate::utils::logging::LogLevel::Debug, $message)
    };
    ($logger:expr, $message:expr, $($key:expr => $value:expr),+) => {
        structured_log!($logger, $crate::utils::logging::LogLevel::Debug, $message, $($key => $value),+)
    };
}

#[macro_export]
macro_rules! log_info {
    ($logger:expr, $message:expr) => {
        structured_log!($logger, $crate::utils::logging::LogLevel::Info, $message)
    };
    ($logger:expr, $message:expr, $($key:expr => $value:expr),+) => {
        structured_log!($logger, $crate::utils::logging::LogLevel::Info, $message, $($key => $value),+)
    };
}

#[macro_export]
macro_rules! log_warn {
    ($logger:expr, $message:expr) => {
        structured_log!($logger, $crate::utils::logging::LogLevel::Warn, $message)
    };
    ($logger:expr, $message:expr, $($key:expr => $value:expr),+) => {
        structured_log!($logger, $crate::utils::logging::LogLevel::Warn, $message, $($key => $value),+)
    };
}

#[macro_export]
macro_rules! log_error {
    ($logger:expr, $message:expr) => {
        structured_log!($logger, $crate::utils::logging::LogLevel::Error, $message)
    };
    ($logger:expr, $message:expr, $($key:expr => $value:expr),+) => {
        structured_log!($logger, $crate::utils::logging::LogLevel::Error, $message, $($key => $value),+)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestOutput {
        entries: Arc<Mutex<Vec<LogEntry>>>,
        write_count: Arc<AtomicUsize>,
    }

    impl TestOutput {
        fn new() -> Self {
            Self {
                entries: Arc::new(Mutex::new(Vec::new())),
                write_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn get_entries(&self) -> Vec<LogEntry> {
            self.entries.lock().unwrap().clone()
        }

        fn get_write_count(&self) -> usize {
            self.write_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl LogOutput for TestOutput {
        async fn write_log(
            &self,
            entry: &LogEntry,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.entries.lock().unwrap().push(entry.clone());
            self.write_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn flush(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_structured_logger() {
        let test_output = Arc::new(TestOutput::new());
        let logger = StructuredLogger::new(LogLevel::Debug).add_output(test_output.clone());

        // 测试基本日志记录
        logger.info("测试消息", "test_module").await;

        let entries = test_output.get_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, LogLevel::Info);
        assert_eq!(entries[0].message, "测试消息");
        assert_eq!(entries[0].target, "test_module");
    }

    #[tokio::test]
    async fn test_log_level_filtering() {
        let test_output = Arc::new(TestOutput::new());
        let logger = StructuredLogger::new(LogLevel::Warn).add_output(test_output.clone());

        // 这些日志应该被过滤掉
        logger.debug("debug消息", "test").await;
        logger.info("info消息", "test").await;

        // 这些日志应该被记录
        logger.warn("warn消息", "test").await;
        logger.error("error消息", "test").await;

        let entries = test_output.get_entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].level, LogLevel::Warn);
        assert_eq!(entries[1].level, LogLevel::Error);
    }

    #[tokio::test]
    async fn test_context() {
        let test_output = Arc::new(TestOutput::new());
        let logger = StructuredLogger::new(LogLevel::Debug).add_output(test_output.clone());

        // 设置全局上下文
        logger.set_context("service", "document-parser").await;
        logger.set_context("version", "1.0.0").await;

        logger.info("测试消息", "test").await;

        let entries = test_output.get_entries();
        assert_eq!(entries.len(), 1);

        let entry = &entries[0];
        assert!(entry.fields.contains_key("service"));
        assert!(entry.fields.contains_key("version"));
    }

    #[test]
    fn test_log_entry_sanitization() {
        let mut entry = LogEntry::new(
            LogLevel::Info,
            "用户登录: password=secret123 token=abc123".to_string(),
            "auth".to_string(),
        );

        entry.sanitize();

        assert!(entry.message.contains("password: ***"));
        assert!(entry.message.contains("token: ***"));
        assert!(!entry.message.contains("secret123"));
        assert!(!entry.message.contains("abc123"));
    }
}
