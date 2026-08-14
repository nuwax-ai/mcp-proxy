//! 解析引擎配置：DocumentParser 通用配置、MinerU / MarkItDown 引擎配置与默认值。

use super::*;

/// 文档解析配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentParserConfig {
    pub max_concurrent: usize,
    pub queue_size: usize,
    pub download_timeout: u32,
    pub processing_timeout: u32,
    /// 同步解析接口（/parse-sync）请求体大小上限，由路由级 `DefaultBodyLimit`
    /// middleware 承载（超限返回 413），仅供测试验证使用
    #[serde(default = "default_sync_parse_max_file_size")]
    pub sync_parse_max_file_size: FileSize,
    /// 同步解析接口（/parse-sync）最大并发数，MinerU 为重资源，默认限流 2 路
    #[serde(default = "default_sync_parse_max_concurrent")]
    pub sync_parse_max_concurrent: usize,
    /// 同步解析接口（/parse-sync）整体超时（秒），默认 600 = 10 分钟
    #[serde(default = "default_sync_parse_timeout_secs")]
    pub sync_parse_timeout_secs: u32,
}

/// 同步解析接口默认最大文件大小（500MB）
fn default_sync_parse_max_file_size() -> FileSize {
    FileSize::from_mb(500)
}

/// 同步解析接口默认最大并发数
fn default_sync_parse_max_concurrent() -> usize {
    2
}

/// 同步解析接口默认整体超时（10 分钟）
fn default_sync_parse_timeout_secs() -> u32 {
    600
}

impl DocumentParserConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.max_concurrent == 0 {
            return Err(ConfigError::Validation {
                field: "document_parser.max_concurrent".to_string(),
                message: "最大并发数不能为0".to_string(),
            });
        }

        if self.max_concurrent > 100 {
            return Err(ConfigError::Validation {
                field: "document_parser.max_concurrent".to_string(),
                message: "最大并发数不能超过100".to_string(),
            });
        }

        if self.queue_size == 0 {
            return Err(ConfigError::Validation {
                field: "document_parser.queue_size".to_string(),
                message: "队列大小不能为0".to_string(),
            });
        }

        // 文件大小限制现在由全局配置管理

        if self.download_timeout == 0 {
            return Err(ConfigError::Validation {
                field: "document_parser.download_timeout".to_string(),
                message: "下载超时时间不能为0".to_string(),
            });
        }

        if self.processing_timeout == 0 {
            return Err(ConfigError::Validation {
                field: "document_parser.processing_timeout".to_string(),
                message: "处理超时时间不能为0".to_string(),
            });
        }

        // 同步解析接口配置校验
        if self.sync_parse_max_file_size.bytes() == 0 {
            return Err(ConfigError::Validation {
                field: "document_parser.sync_parse_max_file_size".to_string(),
                message: "同步解析接口最大文件大小不能为0".to_string(),
            });
        }

        if self.sync_parse_max_file_size.bytes() > 10 * 1024 * 1024 * 1024 {
            return Err(ConfigError::Validation {
                field: "document_parser.sync_parse_max_file_size".to_string(),
                message: "同步解析接口最大文件大小不能超过10GB".to_string(),
            });
        }

        if self.sync_parse_max_concurrent == 0 {
            return Err(ConfigError::Validation {
                field: "document_parser.sync_parse_max_concurrent".to_string(),
                message: "同步解析接口最大并发数不能为0".to_string(),
            });
        }

        if self.sync_parse_max_concurrent > 16 {
            return Err(ConfigError::Validation {
                field: "document_parser.sync_parse_max_concurrent".to_string(),
                message: "同步解析接口最大并发数不能超过16".to_string(),
            });
        }

        if self.sync_parse_timeout_secs == 0 {
            return Err(ConfigError::Validation {
                field: "document_parser.sync_parse_timeout_secs".to_string(),
                message: "同步解析接口超时时间不能为0".to_string(),
            });
        }

        Ok(())
    }
}

/// MinerU配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinerUConfig {
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default = "default_python_path")]
    pub python_path: String,
    pub max_concurrent: usize,
    pub queue_size: usize,
    #[serde(default)]
    pub timeout: u32, // 0表示使用统一的processing_timeout
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    #[serde(default)]
    pub quality_level: QualityLevel,
    #[serde(default = "default_device")]
    pub device: String, // 推理设备：cpu/cuda/cuda:0/npu/mps等
    #[serde(default = "default_vram")]
    pub vram: u32, // 单进程最大GPU显存占用(GB)，仅对pipeline后端且支持CUDA时有效
    /// vllm 显存占用比例(0.0-1.0)，仅对 hybrid-engine/vlm-engine 等走 vllm 的后端有效。
    /// 0 表示不传，用 mineru 默认(约0.5)；多 GPU 进程共存时调低(如 0.3)避免 OOM。
    #[serde(default)]
    pub gpu_memory_utilization: f32,
}

/// 质量级别
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum QualityLevel {
    Fast,
    #[default]
    Balanced,
    HighQuality,
}

fn default_batch_size() -> usize {
    1
}

fn default_backend() -> String {
    "pipeline".to_string()
}

fn default_python_path() -> String {
    // Default to virtual environment python if available, otherwise system python
    if cfg!(windows) {
        "./venv/Scripts/python.exe".to_string()
    } else {
        "./venv/bin/python".to_string()
    }
}

fn default_device() -> String {
    "cpu".to_string()
}

/// Platform-aware default for `MinerUConfig::default()` / auto-generated config.yml.
fn default_device_for_platform() -> String {
    if cfg!(target_os = "macos") {
        "mps".to_string()
    } else {
        "cpu".to_string()
    }
}

fn default_vram() -> u32 {
    0 // 默认不限制显存（mineru 3.4 改用 MINERU_VIRTUAL_VRAM_SIZE 环境变量）
}

impl MinerUConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let valid_backends = [
            "pipeline",
            "vlm-engine",
            "hybrid-engine",
            "vlm-http-client",
            "hybrid-http-client",
        ];
        if !valid_backends.contains(&self.backend.as_str()) {
            return Err(ConfigError::Validation {
                field: "mineru.backend".to_string(),
                message: format!(
                    "无效的后端类型: {}，支持的类型: {:?}",
                    self.backend, valid_backends
                ),
            });
        }

        if self.python_path.is_empty() {
            return Err(ConfigError::Validation {
                field: "mineru.python_path".to_string(),
                message: "Python路径不能为空".to_string(),
            });
        }

        if self.max_concurrent == 0 {
            return Err(ConfigError::Validation {
                field: "mineru.max_concurrent".to_string(),
                message: "最大并发数不能为0".to_string(),
            });
        }

        if self.queue_size == 0 {
            return Err(ConfigError::Validation {
                field: "mineru.queue_size".to_string(),
                message: "队列大小不能为0".to_string(),
            });
        }

        // timeout 为 0 表示使用统一的 processing_timeout，这是允许的

        if self.batch_size == 0 {
            return Err(ConfigError::Validation {
                field: "mineru.batch_size".to_string(),
                message: "批处理大小不能为0".to_string(),
            });
        }

        Ok(())
    }

    /// Get the effective python path, auto-detecting virtual environment if needed
    pub fn get_effective_python_path(&self) -> String {
        // If the configured path is the default and a virtual environment exists, use it
        let default_path = default_python_path();
        if self.python_path == default_path
            || self.python_path == "python3"
            || self.python_path == "python"
        {
            let venv_python = if cfg!(windows) {
                std::path::Path::new("./venv/Scripts/python.exe")
            } else {
                std::path::Path::new("./venv/bin/python")
            };

            if venv_python.exists() {
                return venv_python.to_string_lossy().to_string();
            }
        }

        self.python_path.clone()
    }
}

/// MarkItDown配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkItDownConfig {
    #[serde(default = "default_python_path")]
    pub python_path: String,
    #[serde(default)]
    pub timeout: u32, // 0表示使用统一的processing_timeout
    pub enable_plugins: bool,
    pub features: MarkItDownFeatures,
}

impl MarkItDownConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.python_path.is_empty() {
            return Err(ConfigError::Validation {
                field: "markitdown.python_path".to_string(),
                message: "Python路径不能为空".to_string(),
            });
        }

        // timeout 为 0 表示使用统一的 processing_timeout，这是允许的

        Ok(())
    }

    /// Get the effective python path, auto-detecting virtual environment if needed
    pub fn get_effective_python_path(&self) -> String {
        // If the configured path is the default and a virtual environment exists, use it
        let default_path = default_python_path();
        if self.python_path == default_path
            || self.python_path == "python3"
            || self.python_path == "python"
        {
            let venv_python = if cfg!(windows) {
                std::path::Path::new("./venv/Scripts/python.exe")
            } else {
                std::path::Path::new("./venv/bin/python")
            };

            if venv_python.exists() {
                return venv_python.to_string_lossy().to_string();
            }
        }

        self.python_path.clone()
    }
}

/// MarkItDown功能配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkItDownFeatures {
    pub ocr: bool,
    pub audio_transcription: bool,
    pub azure_doc_intel: bool,
    pub youtube_transcription: bool,
}

impl Default for DocumentParserConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 5,
            queue_size: 1000,
            download_timeout: 3600,
            processing_timeout: 3600,
            sync_parse_max_file_size: default_sync_parse_max_file_size(),
            sync_parse_max_concurrent: default_sync_parse_max_concurrent(),
            sync_parse_timeout_secs: default_sync_parse_timeout_secs(),
        }
    }
}

impl Default for MinerUConfig {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            python_path: default_python_path(),
            max_concurrent: 3,
            queue_size: 100,
            timeout: 0,
            batch_size: default_batch_size(),
            quality_level: QualityLevel::default(),
            // Prefer platform default: macOS → mps, others → cpu (see default_device).
            device: default_device_for_platform(),
            vram: default_vram(),
            gpu_memory_utilization: 0.3,
        }
    }
}

impl Default for MarkItDownFeatures {
    fn default() -> Self {
        Self {
            ocr: true,
            audio_transcription: true,
            azure_doc_intel: false,
            youtube_transcription: false,
        }
    }
}

impl Default for MarkItDownConfig {
    fn default() -> Self {
        Self {
            python_path: default_python_path(),
            timeout: 0,
            enable_plugins: false,
            features: MarkItDownFeatures::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_parser_config_validation() {
        let config = DocumentParserConfig {
            max_concurrent: 3,
            queue_size: 100,
            download_timeout: 3600,
            processing_timeout: 1800,
            sync_parse_max_file_size: default_sync_parse_max_file_size(),
            sync_parse_max_concurrent: default_sync_parse_max_concurrent(),
            sync_parse_timeout_secs: default_sync_parse_timeout_secs(),
        };

        assert!(config.validate().is_ok());

        // 测试零并发
        let mut invalid_config = config.clone();
        invalid_config.max_concurrent = 0;
        assert!(invalid_config.validate().is_err());

        // 测试过大的并发数
        invalid_config.max_concurrent = 200;
        assert!(invalid_config.validate().is_err());
    }

    #[test]
    fn test_document_parser_sync_config_validation() {
        let config = DocumentParserConfig {
            max_concurrent: 3,
            queue_size: 100,
            download_timeout: 3600,
            processing_timeout: 1800,
            sync_parse_max_file_size: default_sync_parse_max_file_size(),
            sync_parse_max_concurrent: default_sync_parse_max_concurrent(),
            sync_parse_timeout_secs: default_sync_parse_timeout_secs(),
        };

        assert!(config.validate().is_ok());

        // 默认同步文件大小上限为 500MB
        assert_eq!(config.sync_parse_max_file_size.bytes(), 500 * 1024 * 1024);

        // 同步文件大小上限不能为 0
        let mut invalid_config = config.clone();
        invalid_config.sync_parse_max_file_size = FileSize(0);
        assert!(invalid_config.validate().is_err());

        // 同步并发数不能为 0
        invalid_config = config.clone();
        invalid_config.sync_parse_max_concurrent = 0;
        assert!(invalid_config.validate().is_err());

        // 同步超时不能为 0
        invalid_config = config.clone();
        invalid_config.sync_parse_timeout_secs = 0;
        assert!(invalid_config.validate().is_err());
    }

    #[test]
    fn test_mineru_config_validation() {
        let config = MinerUConfig {
            backend: "pipeline".to_string(),

            python_path: "/usr/bin/python3".to_string(),
            max_concurrent: 3,
            queue_size: 100,
            timeout: 0, // 使用统一超时配置
            batch_size: 10,
            quality_level: QualityLevel::Balanced,
            device: "cpu".to_string(),
            vram: 8, // 默认显存限制
            gpu_memory_utilization: 0.0,
        };

        assert!(config.validate().is_ok());

        // 测试无效后端
        let mut invalid_config = config.clone();
        invalid_config.backend = "invalid".to_string();
        assert!(invalid_config.validate().is_err());
    }
}
