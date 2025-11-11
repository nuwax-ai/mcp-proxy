//! 单元测试模块
//!
//! 包含所有核心组件的单元测试

pub mod handlers;
pub mod models;
pub mod parsers;
pub mod processors;
pub mod property_tests;
pub mod services;
pub mod test_config;
pub mod utils;
// pub mod coverage_tests; // 模块不存在，暂时禁用
pub mod current_directory_workflow_tests;
pub mod environment_manager_enhanced_tests;
pub mod path_error_handling_tests;
// pub mod comprehensive_unit_tests; // 暂时禁用，需要重构
pub mod section_id_duplicate_tests;

#[cfg(test)]
pub mod test_helpers {
    use crate::app_state::AppState;
    use crate::config::AppConfig;

    /// 安全地初始化全局配置，避免重复初始化错误
    /// 这个函数可以在测试中多次调用而不会出错
    pub fn safe_init_global_config() {
        // 使用 std::panic::catch_unwind 来捕获可能的初始化错误
        let _ = std::panic::catch_unwind(|| {
            let app_config = create_real_environment_test_config();
            crate::config::init_global_config(app_config)
        });
    }

    /// 安全地初始化全局配置，使用自定义配置
    /// 这个函数可以在测试中多次调用而不会出错
    pub fn safe_init_global_config_with_config(config: AppConfig) {
        // 使用 std::panic::catch_unwind 来捕获可能的初始化错误
        let _ = std::panic::catch_unwind(|| crate::config::init_global_config(config));
    }

    /// 创建测试用的应用状态
    pub async fn create_test_app_state() -> AppState {
        let config = create_test_config();
        AppState::new(config)
            .await
            .expect("Failed to create test app state")
    }

    /// 创建用于文件大小测试的应用状态
    pub async fn create_test_app_state_for_file_size_test(
        max_mb: u64,
        threshold_mb: u64,
    ) -> AppState {
        let config = create_test_config_with_file_size(max_mb, threshold_mb);
        AppState::new(config)
            .await
            .expect("Failed to create test app state")
    }

    /// 创建测试用的配置
    /// 优先从配置文件加载，如果失败则使用测试专用的默认值
    pub fn create_test_config() -> AppConfig {
        create_test_config_with_overrides(None)
    }

    /// 创建测试用的配置，支持自定义覆盖
    pub fn create_test_config_with_overrides(
        overrides: Option<Box<dyn Fn(&mut AppConfig)>>,
    ) -> AppConfig {
        use std::time::{SystemTime, UNIX_EPOCH};
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let unique_id = format!("{timestamp}");

        // 尝试从配置文件加载基础配置
        let mut config = match crate::config::AppConfig::load_base_config() {
            Ok(base_config) => {
                // 成功加载配置文件，使用配置文件的值作为基础
                let mut config = base_config;

                // 覆盖测试专用的配置项
                config.environment = "test".to_string();
                config.server.port = 0; // 使用随机端口
                config.log.level = "debug".to_string();
                config.log.path = format!("/tmp/test_{unique_id}.log");
                config.storage.sled.path =
                    format!("/tmp/test_sled_{}_{}.db", unique_id, std::process::id());
                config.storage.sled.cache_capacity = 1024 * 1024;
                // temp_dir removed - now uses current directory approach

                // 调整并发和队列大小以适合测试环境
                config.document_parser.max_concurrent = 2;
                config.document_parser.queue_size = 10;
                config.mineru.max_concurrent = 1;
                config.mineru.queue_size = 5;

                config
            }
            Err(_) => {
                // 配置文件加载失败，使用完全的测试默认配置
                AppConfig {
                    environment: "test".to_string(),
                    server: crate::config::ServerConfig {
                        port: 0, // 使用随机端口
                        host: "127.0.0.1".to_string(),
                    },
                    log: crate::config::LogConfig {
                        level: "debug".to_string(),
                        path: format!("/tmp/test_{unique_id}.log"),
                    },
                    document_parser: crate::config::DocumentParserConfig {
                        max_concurrent: 2,
                        queue_size: 10,
                        download_timeout: 300,
                        processing_timeout: 1800,
                    },
                    file_size_config: crate::config::GlobalFileSizeConfig {
                        max_file_size: crate::config::FileSize::from_mb(100), // 100MB
                        large_document_threshold: crate::config::FileSize::from_mb(50), // 50MB
                    },
                    storage: crate::config::StorageConfig {
                        sled: crate::config::SledConfig {
                            path: format!("/tmp/test_sled_{}_{}.db", unique_id, std::process::id()),
                            cache_capacity: 1024 * 1024,
                        },
                        oss: crate::config::OssConfig {
                            endpoint: "https://test-endpoint.com".to_string(),
                            public_bucket: "test-bucket".to_string(),
                            private_bucket: "test-bucket".to_string(),
                            access_key_id: "test-key-id-placeholder".to_string(),
                            access_key_secret: "test-key-secret-placeholder".to_string(),
                            upload_directory: "test".to_string(),
                            region: "oss-rg-china-mainland".to_string(),
                        },
                    },
                    external_integration: crate::config::ExternalIntegrationConfig {
                        webhook_url: "https://test-webhook.com".to_string(),
                        api_key: "test-api-key".to_string(),
                        timeout: 30,
                    },
                    mineru: crate::config::MinerUConfig {
                        backend: "pipeline".to_string(),
                        python_path: "python3".to_string(),
                        max_concurrent: 1,
                        queue_size: 5,
                        timeout: 0, // 使用统一超时配置
                        batch_size: 1,
                        quality_level: crate::config::QualityLevel::Balanced,
                        device: "cpu".to_string(),
                        vram: 8,
                    },
                    markitdown: crate::config::MarkItDownConfig {
                        python_path: "python3".to_string(),
                        timeout: 0, // 使用统一超时配置
                        enable_plugins: false,
                        features: crate::config::MarkItDownFeatures {
                            ocr: false,
                            audio_transcription: false,
                            azure_doc_intel: false,
                            youtube_transcription: false,
                        },
                    },
                }
            }
        };

        // 应用自定义覆盖
        if let Some(override_fn) = overrides {
            override_fn(&mut config);
        }

        config
    }

    /// 创建带有自定义文件大小限制的测试配置
    pub fn create_test_config_with_file_size(max_mb: u64, threshold_mb: u64) -> AppConfig {
        create_test_config_with_overrides(Some(Box::new(move |config| {
            config.file_size_config.max_file_size = crate::config::FileSize::from_mb(max_mb);
            config.file_size_config.large_document_threshold =
                crate::config::FileSize::from_mb(threshold_mb);
        })))
    }

    /// 创建带有自定义服务器配置的测试配置
    pub fn create_test_config_with_server(port: u16, host: &str) -> AppConfig {
        let host = host.to_string();
        create_test_config_with_overrides(Some(Box::new(move |config| {
            config.server.port = port;
            config.server.host = host.clone();
        })))
    }

    /// 创建带有自定义并发设置的测试配置
    pub fn create_test_config_with_concurrency(max_concurrent: u32, queue_size: u32) -> AppConfig {
        create_test_config_with_overrides(Some(Box::new(move |config| {
            config.document_parser.max_concurrent = max_concurrent as usize;
            config.document_parser.queue_size = queue_size as usize;
        })))
    }

    /// 创建用于真实环境测试的配置（使用虚拟环境中的MinerU和MarkItDown）
    pub fn create_real_environment_test_config() -> AppConfig {
        create_test_config_with_overrides(Some(Box::new(|config| {
            // 使用当前目录下的虚拟环境
            let current_dir =
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let venv_python = current_dir.join("venv").join("bin").join("python");

            // 设置MinerU使用虚拟环境中的Python
            if venv_python.exists() {
                config.mineru.python_path = venv_python.to_string_lossy().to_string();
            }

            // 设置MarkItDown使用虚拟环境中的Python
            if venv_python.exists() {
                config.markitdown.python_path = venv_python.to_string_lossy().to_string();
            }

            // 启用插件和功能以进行更全面的测试
            config.markitdown.enable_plugins = true;
            config.markitdown.features.ocr = true;
            config.markitdown.features.audio_transcription = true;
            config.markitdown.features.azure_doc_intel = true;
            config.markitdown.features.youtube_transcription = true;

            // 设置合理的超时时间
            config.mineru.timeout = 600; // 10分钟
            config.markitdown.timeout = 300; // 5分钟
        })))
    }

    /// 创建测试用的Markdown内容
    pub fn create_test_markdown() -> String {
        r#"# 测试文档

这是一个测试文档。

## 第一章

这是第一章的内容。

### 1.1 小节

这是1.1小节的内容。

## 第二章

这是第二章的内容。

### 2.1 小节

这是2.1小节的内容。

### 2.2 小节

这是2.2小节的内容。
"#
        .to_string()
    }

    /// 创建测试用的任务ID
    pub fn create_test_task_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

/// 测试配置使用示例和最佳实践
///
/// # 基本使用
///
/// ```rust
/// use crate::tests::test_helpers::*;
///
/// #[tokio::test]
/// async fn test_basic_functionality() {
///     // 使用默认测试配置（会尝试从config.yml加载）
///     let app_state = create_test_app_state().await;
///     // 进行测试...
/// }
/// ```
///
/// # 自定义文件大小限制
///
/// ```rust
/// #[tokio::test]
/// async fn test_file_size_validation() {
///     // 创建文件大小限制为50MB的测试配置
///     let app_state = create_test_app_state_for_file_size_test(50, 25).await;
///     // 测试文件大小验证逻辑...
/// }
/// ```
///
/// # 性能测试配置
///
/// ```rust
/// #[tokio::test]
/// async fn test_high_concurrency() {
///     // 使用高并发配置进行性能测试
///     let config = create_performance_test_config();
///     let app_state = create_test_app_state_with_config(config).await;
///     // 进行并发测试...
/// }
/// ```
///
/// # 自定义配置覆盖
///
/// ```rust
/// #[tokio::test]
/// async fn test_custom_configuration() {
///     // 使用自定义配置覆盖
///     let config = create_test_config_with_overrides(Some(Box::new(|config| {
///         config.document_parser.processing_timeout = 60; // 1分钟超时
///         config.mineru.enable_gpu = true; // 启用GPU
///     })));
///     let app_state = create_test_app_state_with_config(config).await;
///     // 进行自定义配置测试...
/// }
/// ```
///
/// # 集成测试配置
///
/// ```rust
/// #[tokio::test]
/// async fn test_external_services() {
///     // 设置环境变量
///     std::env::set_var("TEST_OSS_ENDPOINT", "https://real-oss-endpoint.com");
///     std::env::set_var("TEST_OSS_BUCKET", "real-test-bucket");
///
///     // 使用集成测试配置
///     let config = create_integration_test_config();
///     let app_state = create_test_app_state_with_config(config).await;
///     // 进行集成测试...
/// }
/// ```
///
/// # 配置优先级
///
/// 1. 首先尝试从 `config.yml` 文件加载配置
/// 2. 如果加载失败，使用内置的测试默认配置
/// 3. 应用测试专用的覆盖（如临时目录、随机端口等）
/// 4. 应用用户自定义的覆盖函数
///
/// # 最佳实践
///
/// - 对于简单的单元测试，使用 `create_test_config()` 或 `create_test_app_state()`
/// - 对于需要特定配置的测试，使用相应的便利函数（如 `create_test_config_with_file_size`）
/// - 对于复杂的自定义需求，使用 `create_test_config_with_overrides`
/// - 对于性能测试，使用 `create_performance_test_config()`
/// - 对于集成测试，使用 `create_integration_test_config()` 并设置相应的环境变量
#[cfg(test)]
mod config_system_tests {
    use super::test_helpers::*;

    #[test]
    fn test_create_test_config_loads_successfully() {
        let config = create_test_config();
        assert_eq!(config.environment, "test");
        assert!(config.file_size_config.max_file_size.bytes() > 0);
    }

    #[test]
    fn test_create_test_config_with_file_size() {
        let config = create_test_config_with_file_size(200, 100);
        assert_eq!(
            config.file_size_config.max_file_size.bytes(),
            200 * 1024 * 1024
        );
        assert_eq!(
            config.file_size_config.large_document_threshold.bytes(),
            100 * 1024 * 1024
        );
    }

    #[test]
    fn test_create_test_config_with_server() {
        let config = create_test_config_with_server(8080, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.host, "0.0.0.0");
    }

    #[test]
    fn test_create_test_config_with_concurrency() {
        let config = create_test_config_with_concurrency(4, 20);
        assert_eq!(config.document_parser.max_concurrent, 4);
        assert_eq!(config.document_parser.queue_size, 20);
    }

    #[tokio::test]
    async fn test_create_test_app_state() {
        // 安全初始化全局配置
        safe_init_global_config();

        let app_state = create_test_app_state().await;
        // 验证 app_state 创建成功
        assert!(app_state.config.environment == "test");
    }

    #[tokio::test]
    async fn test_create_test_app_state_for_file_size_test() {
        // 安全初始化全局配置
        safe_init_global_config();

        let app_state = create_test_app_state_for_file_size_test(50, 10).await;
        assert_eq!(
            app_state
                .get_config()
                .file_size_config
                .max_file_size
                .bytes(),
            50 * 1024 * 1024
        );
        assert_eq!(
            app_state
                .get_config()
                .file_size_config
                .large_document_threshold
                .bytes(),
            10 * 1024 * 1024
        );
    }
}

pub mod config_examples {}
