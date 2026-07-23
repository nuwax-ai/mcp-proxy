use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Env override macros ──────────────────────────────────────────────────────

macro_rules! env_str {
    ($self:expr, $var:literal, $field:expr) => {
        if let Ok(val) = std::env::var($var) {
            tracing::info!("Env {} overrides: {}", $var, val);
            $field = val;
        }
    };
}

macro_rules! env_num {
    ($self:expr, $var:literal, $ty:ty, $field:expr) => {
        if let Ok(val) = std::env::var($var) {
            if let Ok(parsed) = val.parse::<$ty>() {
                tracing::info!("Env {} overrides: {}", $var, parsed);
                $field = parsed;
            } else {
                tracing::warn!("Env {} invalid ({}), ignored", $var, val);
            }
        }
    };
}

macro_rules! env_opt_str {
    ($self:expr, $var:literal, $field:expr) => {
        if let Ok(val) = std::env::var($var) {
            if val.is_empty() {
                $field = None;
            } else {
                tracing::info!("Env {} overrides", $var);
                $field = Some(val);
            }
        }
    };
}

macro_rules! env_device {
    ($self:expr) => {
        if let Ok(val) = std::env::var("FASTEMBED_DEVICE") {
            match serde_json::from_str::<$crate::config::Device>(&format!(
                "\"{}\"",
                val.to_lowercase()
            )) {
                Ok(d) => {
                    tracing::info!("Env FASTEMBED_DEVICE overrides: {}", d);
                    $self.fastembed.device = d;
                }
                Err(_) => tracing::warn!("Env FASTEMBED_DEVICE invalid ({}), ignored", val),
            }
        }
    };
}

// ── Config structs ───────────────────────────────────────────────────────────
/// Compute device for ONNX inference
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Device {
    /// Auto-detect per platform (CoreML on macOS, CUDA on Linux, DirectML on Windows)
    Auto,
    /// CPU only
    Cpu,
    /// Apple CoreML (macOS GPU/Neural Engine)
    #[serde(rename = "coreml")]
    CoreML,
    /// NVIDIA CUDA GPU
    Cuda,
    /// Windows DirectML GPU
    #[serde(rename = "directml")]
    DirectML,
}

impl Device {
    pub fn as_str(self) -> &'static str {
        match self {
            Device::Auto => "auto",
            Device::Cpu => "cpu",
            Device::CoreML => "coreml",
            Device::Cuda => "cuda",
            Device::DirectML => "directml",
        }
    }
}

impl std::fmt::Display for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 服务器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// 监听地址
    #[serde(default = "default_host")]
    pub host: String,

    /// 监听端口
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8068
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

/// FastEmbed 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FastEmbedConfig {
    /// 缓存目录
    #[serde(default = "default_cache_dir")]
    pub cache_dir: String,

    /// 默认文本模型
    #[serde(default = "default_model")]
    pub default_model: String,

    /// 默认图像模型
    #[serde(default = "default_image_model")]
    pub default_image_model: String,

    /// 默认稀疏模型
    #[serde(default = "default_sparse_model")]
    pub default_sparse_model: String,

    /// 批处理大小
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    /// 计算设备：auto（按平台自动选 GPU EP）| cpu | coreml | cuda | directml
    #[serde(default = "default_device")]
    pub device: Device,

    /// 每个模型的实例池大小（并发推理上限）。
    /// =1：单实例，并发请求排队（CPU 推理下通常最优，避免线程超订阅）。
    /// >1：创建 N 个独立 ONNX 会话，允许 N 路并发推理（代价 N× 内存）。
    #[serde(default = "default_pool_size")]
    pub pool_size: usize,

    /// 模型包下载 URL（可选）。
    /// 服务启动时自动从此 URL 下载模型包（.tar.gz）并解压到 cache_dir，
    /// 下载完成后 fastembed 初始化时发现缓存已存在即跳过 HuggingFace 下载。
    /// 留空则不自动下载，依赖已有的缓存或 HuggingFace 按需下载。
    #[serde(default)]
    pub model_url: Option<String>,
}

fn default_cache_dir() -> String {
    ".fastembed_cache".to_string()
}

fn default_model() -> String {
    "BGELargeZHV15".to_string()
}

fn default_image_model() -> String {
    "ClipVitB32".to_string()
}

fn default_sparse_model() -> String {
    "SPLADEPPV1".to_string()
}

fn default_batch_size() -> usize {
    256
}

fn default_device() -> Device {
    Device::Auto
}

fn default_pool_size() -> usize {
    1
}

impl Default for FastEmbedConfig {
    fn default() -> Self {
        Self {
            cache_dir: default_cache_dir(),
            default_model: default_model(),
            default_image_model: default_image_model(),
            default_sparse_model: default_sparse_model(),
            batch_size: default_batch_size(),
            device: default_device(),
            pool_size: default_pool_size(),
            model_url: None,
        }
    }
}

/// 应用配置
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,

    #[serde(default)]
    pub fastembed: FastEmbedConfig,
}

impl AppConfig {
    /// 从文件加载配置
    pub fn from_file(path: &PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("无法读取配置文件: {:?}", path))?;

        let config: AppConfig = serde_yaml::from_str(&content)
            .with_context(|| format!("无法解析配置文件: {:?}", path))?;

        Ok(config)
    }

    /// 生成默认配置文件
    pub fn generate_default_config(path: &PathBuf) -> Result<()> {
        let default_config = AppConfig::default();
        let yaml = serde_yaml::to_string(&default_config).context("无法序列化默认配置")?;

        std::fs::write(path, yaml).with_context(|| format!("无法写入配置文件: {:?}", path))?;

        tracing::info!("Default configuration file has been generated: {:?}", path);
        Ok(())
    }

    /// Apply environment variable overrides
    pub fn apply_env_overrides(&mut self) {
        // Server
        env_str!(self, "FASTEMBED_HOST", self.server.host);
        env_num!(self, "FASTEMBED_PORT", u16, self.server.port);

        // FastEmbed
        env_str!(self, "FASTEMBED_CACHE_DIR", self.fastembed.cache_dir);
        env_str!(self, "FASTEMBED_MODEL", self.fastembed.default_model);
        env_str!(
            self,
            "FASTEMBED_IMAGE_MODEL",
            self.fastembed.default_image_model
        );
        env_str!(
            self,
            "FASTEMBED_SPARSE_MODEL",
            self.fastembed.default_sparse_model
        );
        env_device!(self);
        env_num!(
            self,
            "FASTEMBED_BATCH_SIZE",
            usize,
            self.fastembed.batch_size
        );
        env_num!(self, "FASTEMBED_POOL_SIZE", usize, self.fastembed.pool_size);
        env_opt_str!(self, "FASTEMBED_MODEL_URL", self.fastembed.model_url);
    }

    /// 加载或生成配置
    pub fn load_or_generate(config_path: Option<PathBuf>) -> Result<Self> {
        let path = config_path.unwrap_or_else(|| PathBuf::from("./config.yml"));

        let mut config = if path.exists() {
            tracing::info!("Load configuration from file: {:?}", path);
            Self::from_file(&path)?
        } else {
            tracing::warn!(
                "Configuration file does not exist: {:?}, generate default configuration",
                path
            );
            Self::generate_default_config(&path)?;
            Self::default()
        };

        // 应用环境变量覆盖
        config.apply_env_overrides();

        // 打印最终配置
        tracing::info!("Final configuration: {:?}", config);

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    // env 是进程级共享，多个 env 测试并发会互相串值；用此锁串行化所有读改 env 的测试。
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    // Rust 2024 起 std::env::set_var/remove_var 为 unsafe（多线程读 env 存在数据竞争）。
    // 这里仅在持锁的测试内、且为 DI 验证 env 覆盖逻辑使用，集中收口 unsafe。
    fn set_env(k: &str, v: &str) {
        // SAFETY: 调用方持有 ENV_LOCK，且其余测试不读 FASTEMBED_* 变量。
        unsafe { std::env::set_var(k, v) }
    }
    fn remove_env(k: &str) {
        // SAFETY: 同上。
        unsafe { std::env::remove_var(k) }
    }

    #[test]
    fn config_default_values() {
        let cfg = FastEmbedConfig::default();
        assert_eq!(cfg.default_model, "BGELargeZHV15");
        assert_eq!(cfg.default_image_model, "ClipVitB32");
        assert_eq!(cfg.default_sparse_model, "SPLADEPPV1");
        assert_eq!(cfg.device, Device::Auto);
        assert_eq!(cfg.batch_size, 256);
        assert_eq!(cfg.pool_size, 1);
    }

    #[test]
    fn config_serde_roundtrip() {
        let cfg = AppConfig::default();
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        // 关键字段都序列化出来
        assert!(yaml.contains("default_image_model"));
        assert!(yaml.contains("default_sparse_model"));
        assert!(yaml.contains("device: auto"));
        // 反序列化回来等价
        let back: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back.fastembed.default_model, cfg.fastembed.default_model);
        assert_eq!(back.fastembed.device, Device::Auto);
        assert_eq!(back.server.port, 8068);
    }

    #[test]
    fn config_from_file_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.yml");
        AppConfig::generate_default_config(&path).unwrap();
        assert!(path.exists());
        let loaded = AppConfig::from_file(&path).unwrap();
        assert_eq!(loaded.fastembed.default_model, "BGELargeZHV15");
        assert_eq!(loaded.fastembed.default_image_model, "ClipVitB32");
    }

    /// env 覆盖集中在一个测试里，避免并行测试间 env 串扰
    #[test]
    fn apply_env_overrides_all_vars() {
        let _guard = env_lock().lock().unwrap();
        let keys = [
            "FASTEMBED_HOST",
            "FASTEMBED_PORT",
            "FASTEMBED_CACHE_DIR",
            "FASTEMBED_MODEL",
            "FASTEMBED_IMAGE_MODEL",
            "FASTEMBED_SPARSE_MODEL",
            "FASTEMBED_DEVICE",
            "FASTEMBED_BATCH_SIZE",
            "FASTEMBED_POOL_SIZE",
        ];
        // 先清掉可能存在的旧值（CI 环境可能有）
        for k in keys {
            remove_env(k);
        }

        set_env("FASTEMBED_HOST", "127.0.0.1");
        set_env("FASTEMBED_PORT", "9999");
        set_env("FASTEMBED_CACHE_DIR", "/tmp/cache_x");
        set_env("FASTEMBED_MODEL", "AllMiniLML6V2");
        set_env("FASTEMBED_IMAGE_MODEL", "ClipVitB32");
        set_env("FASTEMBED_SPARSE_MODEL", "BGEM3");
        set_env("FASTEMBED_DEVICE", "cpu");
        set_env("FASTEMBED_BATCH_SIZE", "128");
        set_env("FASTEMBED_POOL_SIZE", "4");

        let mut cfg = AppConfig::default();
        cfg.apply_env_overrides();

        assert_eq!(cfg.server.host, "127.0.0.1");
        assert_eq!(cfg.server.port, 9999);
        assert_eq!(cfg.fastembed.cache_dir, "/tmp/cache_x");
        assert_eq!(cfg.fastembed.default_model, "AllMiniLML6V2");
        assert_eq!(cfg.fastembed.default_image_model, "ClipVitB32");
        assert_eq!(cfg.fastembed.default_sparse_model, "BGEM3");
        assert_eq!(cfg.fastembed.device, Device::Cpu);
        assert_eq!(cfg.fastembed.batch_size, 128);
        assert_eq!(cfg.fastembed.pool_size, 4);

        // 清理
        for k in keys {
            remove_env(k);
        }
    }

    #[test]
    fn apply_env_overrides_invalid_port_ignored() {
        let _guard = env_lock().lock().unwrap();
        set_env("FASTEMBED_PORT", "not-a-number");
        set_env("FASTEMBED_BATCH_SIZE", "oops");
        let mut cfg = AppConfig::default();
        let orig_port = cfg.server.port;
        let orig_batch = cfg.fastembed.batch_size;
        cfg.apply_env_overrides();
        // 非法值不影响原配置
        assert_eq!(cfg.server.port, orig_port);
        assert_eq!(cfg.fastembed.batch_size, orig_batch);
        remove_env("FASTEMBED_PORT");
        remove_env("FASTEMBED_BATCH_SIZE");
    }
}
