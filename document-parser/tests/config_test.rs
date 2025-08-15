use document_parser::config::*;
use std::env;
use tempfile::TempDir;

#[test]
fn test_file_size_parsing() {
    assert_eq!(parse_file_size("100B").unwrap(), 100);
    assert_eq!(parse_file_size("1KB").unwrap(), 1024);
    assert_eq!(parse_file_size("1MB").unwrap(), 1024 * 1024);
    assert_eq!(parse_file_size("1GB").unwrap(), 1024 * 1024 * 1024);
    assert_eq!(parse_file_size("2.5MB").unwrap(), (2.5 * 1024.0 * 1024.0) as u64);
    
    // 测试无效格式
    assert!(parse_file_size("invalid").is_err());
    assert!(parse_file_size("100XB").is_err());
    assert!(parse_file_size("").is_err());
}

#[test]
fn test_default_config_loading() {
    let config = AppConfig::load_base_config().unwrap();
    
    // 验证默认值
    assert_eq!(config.server.port, 8087);
    assert_eq!(config.server.host, "0.0.0.0");
    assert_eq!(config.log.level, "info");
    assert_eq!(config.document_parser.max_concurrent, 5);
}

#[test]
fn test_config_validation() {
    let mut config = AppConfig::load_base_config().unwrap();
    
    // 测试有效配置
    if let Err(e) = config.validate() {
        println!("Validation error: {:?}", e);
    }
    assert!(config.validate().is_ok());
    
    // 测试无效端口
    config.server.port = 0;
    assert!(config.validate().is_err());
    
    // 恢复有效端口，测试无效日志级别
    config.server.port = 8087;
    config.log.level = "invalid".to_string();
    assert!(config.validate().is_err());
}

#[test]
fn test_config_builder() {
    let server_config = ServerConfig {
        port: 9000,
        host: "127.0.0.1".to_string(),
    };
    
    let document_parser_config = DocumentParserConfig {
        max_concurrent: 5,
        queue_size: 100,
        download_timeout: 3600,
        processing_timeout: 3600,
    };
    
    let config = ConfigBuilder::new()
        .server(server_config)
        .document_parser(document_parser_config)
        .build()
        .unwrap();
    
    assert_eq!(config.server.port, 9000);
    assert_eq!(config.server.host, "127.0.0.1");
    assert_eq!(config.document_parser.max_concurrent, 5);
}

#[test]
fn test_environment_variable_override() {
    // 清理可能存在的环境变量
    unsafe {
        env::remove_var("SERVER_PORT");
        env::remove_var("LOG_LEVEL");
    }
    
    // 设置环境变量
    unsafe {
        env::set_var("SERVER_PORT", "9999");
        env::set_var("LOG_LEVEL", "debug");
    }
    
    let mut config = AppConfig::load_base_config().unwrap();
    config.load_all_from_env().unwrap();
    
    assert_eq!(config.server.port, 9999);
    assert_eq!(config.log.level, "debug");
    
    // 清理环境变量
    unsafe {
        env::remove_var("SERVER_PORT");
        env::remove_var("LOG_LEVEL");
    }
}

#[test]
fn test_invalid_environment_variables() {
    // 清理可能存在的环境变量
    unsafe {
        env::remove_var("SERVER_PORT");
    }
    
    // 设置无效的环境变量
    unsafe {
        env::set_var("SERVER_PORT", "invalid_port");
    }
    
    let mut config = AppConfig::load_base_config().unwrap();
    let result = config.load_all_from_env();
    
    assert!(result.is_err());
    
    // 清理环境变量
    unsafe {
        env::remove_var("SERVER_PORT");
    }
}

#[test]
fn test_cross_validation() {
    let mut config = AppConfig::load_base_config().unwrap();
    
    // 设置不一致的并发配置
    config.document_parser.max_concurrent = 1;
    config.mineru.max_concurrent = 5;
    
    assert!(config.validate().is_err());
}

#[test]
fn test_directory_initialization() {
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path().to_string_lossy().to_string();
    
    let mut config = AppConfig::load_base_config().unwrap();
    config.log.path = format!("{}/logs/app.log", temp_path);
    // temp_dir fields have been removed from config structures
    config.storage.sled.path = format!("{}/sled", temp_path);
    
    assert!(config.initialize_directories().is_ok());
    
    // 验证目录是否创建
    assert!(std::path::Path::new(&format!("{}/logs", temp_path)).exists());
    // 注意：initialize_directories 方法不再创建 temp/mineru 和 temp/markitdown
    assert!(std::path::Path::new(&format!("{}/sled", temp_path)).exists());
}

#[test]
fn test_server_config_validation() {
    let mut config = ServerConfig {
        port: 8080,
        host: "localhost".to_string(),
    };
    
    assert!(config.validate().is_ok());
    
    // 测试无效端口
    config.port = 0;
    assert!(config.validate().is_err());
    
    // 测试空主机
    config.port = 8080;
    config.host = "".to_string();
    assert!(config.validate().is_err());
}

#[test]
fn test_log_config_validation() {
    let mut config = LogConfig {
        level: "info".to_string(),
        path: "/tmp/test.log".to_string(),
    };
    
    assert!(config.validate().is_ok());
    
    // 测试无效日志级别
    config.level = "invalid".to_string();
    assert!(config.validate().is_err());
    
    // 测试空路径
    config.level = "info".to_string();
    config.path = "".to_string();
    assert!(config.validate().is_err());
}

#[test]
fn test_document_parser_config_validation() {
    let config = DocumentParserConfig {
        max_concurrent: 3,
        queue_size: 100,
        download_timeout: 3600,
        processing_timeout: 1800,
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
fn test_mineru_config_validation() {
    let config = MinerUConfig {
        backend: "pipeline".to_string(),

        python_path: "/usr/bin/python3".to_string(),
        max_concurrent: 3,
        queue_size: 100,
        timeout: 3600,
        batch_size: 1,
        quality_level: QualityLevel::Balanced,
    };
    
    assert!(config.validate().is_ok());
    
    // 测试无效后端
    let mut invalid_config = config.clone();
    invalid_config.backend = "invalid".to_string();
    assert!(invalid_config.validate().is_err());
}

#[test]
fn test_oss_config_validation() {
    let config = OssConfig {
        endpoint: "oss-cn-hangzhou.aliyuncs.com".to_string(),
        bucket: "test-bucket".to_string(),
        access_key_id: "".to_string(), // 可以为空
        access_key_secret: "".to_string(), // 可以为空
    };
    
    assert!(config.validate().is_ok());
    
    // 测试空端点
    let mut invalid_config = config.clone();
    invalid_config.endpoint = "".to_string();
    assert!(invalid_config.validate().is_err());
}

#[test]
fn test_external_integration_config_validation() {
    let config = ExternalIntegrationConfig {
        webhook_url: "https://example.com/webhook".to_string(),
        api_key: "test-key".to_string(),
        timeout: 30,
    };
    
    assert!(config.validate().is_ok());
    
    // 测试无效URL
    let mut invalid_config = config.clone();
    invalid_config.webhook_url = "invalid-url".to_string();
    assert!(invalid_config.validate().is_err());
    
    // 测试零超时
    invalid_config.webhook_url = "https://example.com/webhook".to_string();
    invalid_config.timeout = 0;
    assert!(invalid_config.validate().is_err());
}

#[test]
fn test_config_summary() {
    let config = AppConfig::load_base_config().unwrap();
    let summary = config.summary();
    
    assert!(summary.contains("AppConfig"));
    assert!(summary.contains("0.0.0.0:8087"));
    assert!(summary.contains("info"));
    assert!(!summary.contains("access_key")); // 确保敏感信息不在摘要中
}