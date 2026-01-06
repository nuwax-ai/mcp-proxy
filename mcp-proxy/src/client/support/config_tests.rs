//! 配置解析单元测试
//!
//! 测试各种 MCP JSON 配置格式的解析

use super::args::{ConvertArgs, LoggingArgs};
use super::config::{McpConfigSource, parse_convert_config};
use std::collections::HashMap;
use std::path::PathBuf;

#[cfg(test)]
mod config_parsing_tests {
    use super::*;

    /// 测试 1: 解析简单的本地命令配置（command 类型）
    #[test]
    fn test_parse_simple_command_config() {
        let config_json = r#"{
            "mcpServers": {
                "test-service": {
                    "command": "node",
                    "args": ["server.js"]
                }
            }
        }"#;

        let args = ConvertArgs {
            url: None,
            config: Some(config_json.to_string()),
            config_file: None,
            name: None,
            protocol: None,
            auth: None,
            header: vec![],
            retries: 0,
            allow_tools: None,
            deny_tools: None,
            ping_interval: 30,
            ping_timeout: 10,
            logging: LoggingArgs {
                diagnostic: true,
                log_dir: None,
                log_file: None,
                otlp_endpoint: None,
                service_name: "mcp-proxy".to_string(),
            },
        };

        let result = parse_convert_config(&args);
        assert!(result.is_ok(), "解析应该成功");

        let config_source = result.unwrap();
        match config_source {
            McpConfigSource::LocalCommand {
                name,
                command,
                args,
                ..
            } => {
                assert_eq!(name, "test-service");
                assert_eq!(command, "node");
                assert_eq!(args, vec!["server.js"]);
            }
            _ => panic!("应该解析为 LocalCommand 类型"),
        }
    }

    /// 测试 2: 解析带环境变量的本地命令配置
    #[test]
    fn test_parse_command_config_with_env() {
        let config_json = r#"{
            "mcpServers": {
                "my-service": {
                    "command": "python",
                    "args": ["-m", "mcp_server"],
                    "env": {
                        "API_KEY": "test-key",
                        "DEBUG": "true"
                    }
                }
            }
        }"#;

        let args = ConvertArgs {
            url: None,
            config: Some(config_json.to_string()),
            config_file: None,
            name: None,
            protocol: None,
            auth: None,
            header: vec![],
            retries: 0,
            allow_tools: None,
            deny_tools: None,
            ping_interval: 30,
            ping_timeout: 10,
            logging: LoggingArgs {
                diagnostic: true,
                log_dir: None,
                log_file: None,
                otlp_endpoint: None,
                service_name: "mcp-proxy".to_string(),
            },
        };

        let result = parse_convert_config(&args);
        assert!(result.is_ok());

        let config_source = result.unwrap();
        match config_source {
            McpConfigSource::LocalCommand {
                name,
                command,
                args,
                env,
            } => {
                assert_eq!(name, "my-service");
                assert_eq!(command, "python");
                assert_eq!(args, vec!["-m", "mcp_server"]);
                assert_eq!(env.get("API_KEY"), Some(&"test-key".to_string()));
                assert_eq!(env.get("DEBUG"), Some(&"true".to_string()));
            }
            _ => panic!("应该解析为 LocalCommand 类型"),
        }
    }

    /// 测试 3: 解析远程 URL 配置（使用 baseUrl）
    #[test]
    fn test_parse_remote_service_config_baseurl() {
        let config_json = r#"{
            "mcpServers": {
                "remote-service": {
                    "baseUrl": "https://api.example.com/mcp",
                    "headers": {
                        "X-Custom-Header": "custom-value"
                    },
                    "authToken": "Bearer token123",
                    "timeout": 30
                }
            }
        }"#;

        let args = ConvertArgs {
            url: None,
            config: Some(config_json.to_string()),
            config_file: None,
            name: None,
            protocol: None,
            auth: None,
            header: vec![],
            retries: 0,
            allow_tools: None,
            deny_tools: None,
            ping_interval: 30,
            ping_timeout: 10,
            logging: LoggingArgs {
                diagnostic: true,
                log_dir: None,
                log_file: None,
                otlp_endpoint: None,
                service_name: "mcp-proxy".to_string(),
            },
        };

        let result = parse_convert_config(&args);
        assert!(result.is_ok());

        let config_source = result.unwrap();
        match config_source {
            McpConfigSource::RemoteService {
                name,
                url,
                headers,
                timeout,
                ..
            } => {
                assert_eq!(name, "remote-service");
                assert_eq!(url, "https://api.example.com/mcp");
                assert_eq!(
                    headers.get("X-Custom-Header"),
                    Some(&"custom-value".to_string())
                );
                assert_eq!(
                    headers.get("Authorization"),
                    Some(&"Bearer token123".to_string())
                );
                assert_eq!(timeout, Some(30));
            }
            _ => panic!("应该解析为 RemoteService 类型"),
        }
    }

    /// 测试 4: 解析远程 URL 配置（使用 url 字段）
    #[test]
    fn test_parse_remote_service_config_url() {
        let config_json = r#"{
            "mcpServers": {
                "sse-service": {
                    "url": "https://api.example.com/sse",
                    "type": "sse"
                }
            }
        }"#;

        let args = ConvertArgs {
            url: None,
            config: Some(config_json.to_string()),
            config_file: None,
            name: None,
            protocol: None,
            auth: None,
            header: vec![],
            retries: 0,
            allow_tools: None,
            deny_tools: None,
            ping_interval: 30,
            ping_timeout: 10,
            logging: LoggingArgs {
                diagnostic: true,
                log_dir: None,
                log_file: None,
                otlp_endpoint: None,
                service_name: "mcp-proxy".to_string(),
            },
        };

        let result = parse_convert_config(&args);
        assert!(result.is_ok());

        let config_source = result.unwrap();
        match config_source {
            McpConfigSource::RemoteService {
                name,
                url,
                protocol,
                ..
            } => {
                assert_eq!(name, "sse-service");
                assert_eq!(url, "https://api.example.com/sse");
                assert_eq!(format!("{:?}", protocol), "Some(Sse)"); // SSE 协议
            }
            _ => panic!("应该解析为 RemoteService 类型"),
        }
    }

    /// 测试 5: 多服务配置，使用 --name 参数选择
    #[test]
    fn test_parse_multi_server_config_with_name() {
        let config_json = r#"{
            "mcpServers": {
                "service-a": {
                    "command": "node",
                    "args": ["a.js"]
                },
                "service-b": {
                    "command": "python",
                    "args": ["b.py"]
                },
                "service-c": {
                    "baseUrl": "https://api.example.com/mcp"
                }
            }
        }"#;

        let args = ConvertArgs {
            url: None,
            config: Some(config_json.to_string()),
            config_file: None,
            name: Some("service-b".to_string()),
            protocol: None,
            auth: None,
            header: vec![],
            retries: 0,
            allow_tools: None,
            deny_tools: None,
            ping_interval: 30,
            ping_timeout: 10,
            logging: LoggingArgs {
                diagnostic: true,
                log_dir: None,
                log_file: None,
                otlp_endpoint: None,
                service_name: "mcp-proxy".to_string(),
            },
        };

        let result = parse_convert_config(&args);
        assert!(result.is_ok());

        let config_source = result.unwrap();
        match config_source {
            McpConfigSource::LocalCommand { name, command, .. } => {
                assert_eq!(name, "service-b");
                assert_eq!(command, "python");
            }
            _ => panic!("应该解析为 LocalCommand 类型"),
        }
    }

    /// 测试 6: 多服务配置，未指定 --name 参数应该失败
    #[test]
    fn test_parse_multi_server_config_without_name_should_fail() {
        let config_json = r#"{
            "mcpServers": {
                "service-a": {
                    "command": "node"
                },
                "service-b": {
                    "command": "python"
                }
            }
        }"#;

        let args = ConvertArgs {
            url: None,
            config: Some(config_json.to_string()),
            config_file: None,
            name: None, // 未指定 name
            protocol: None,
            auth: None,
            header: vec![],
            retries: 0,
            allow_tools: None,
            deny_tools: None,
            ping_interval: 30,
            ping_timeout: 10,
            logging: LoggingArgs {
                diagnostic: true,
                log_dir: None,
                log_file: None,
                otlp_endpoint: None,
                service_name: "mcp-proxy".to_string(),
            },
        };

        let result = parse_convert_config(&args);
        assert!(result.is_err(), "多服务配置未指定 --name 应该失败");

        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("请使用 --name 指定"),
            "错误消息应该提示使用 --name"
        );
    }

    /// 测试 7: 指定的服务不存在应该失败
    #[test]
    fn test_parse_nonexistent_service_should_fail() {
        let config_json = r#"{
            "mcpServers": {
                "service-a": {
                    "command": "node"
                }
            }
        }"#;

        let args = ConvertArgs {
            url: None,
            config: Some(config_json.to_string()),
            config_file: None,
            name: Some("nonexistent".to_string()), // 不存在的服务
            protocol: None,
            auth: None,
            header: vec![],
            retries: 0,
            allow_tools: None,
            deny_tools: None,
            ping_interval: 30,
            ping_timeout: 10,
            logging: LoggingArgs {
                diagnostic: true,
                log_dir: None,
                log_file: None,
                otlp_endpoint: None,
                service_name: "mcp-proxy".to_string(),
            },
        };

        let result = parse_convert_config(&args);
        assert!(result.is_err(), "不存在的服务应该失败");

        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("'nonexistent' 不存在"),
            "错误消息应该提示服务不存在"
        );
    }

    /// 测试 8: 空配置应该失败
    #[test]
    fn test_parse_empty_config_should_fail() {
        let config_json = r#"{
            "mcpServers": {}
        }"#;

        let args = ConvertArgs {
            url: None,
            config: Some(config_json.to_string()),
            config_file: None,
            name: None,
            protocol: None,
            auth: None,
            header: vec![],
            retries: 0,
            allow_tools: None,
            deny_tools: None,
            ping_interval: 30,
            ping_timeout: 10,
            logging: LoggingArgs {
                diagnostic: true,
                log_dir: None,
                log_file: None,
                otlp_endpoint: None,
                service_name: "mcp-proxy".to_string(),
            },
        };

        let result = parse_convert_config(&args);
        assert!(result.is_err(), "空配置应该失败");

        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("没有找到任何 MCP 服务"),
            "错误消息应该提示没有服务"
        );
    }

    /// 测试 9: URL 配置缺少 url 和 baseUrl 应该失败
    #[test]
    fn test_parse_url_config_without_url_should_fail() {
        let config_json = r#"{
            "mcpServers": {
                "invalid-service": {
                    "type": "sse"
                }
            }
        }"#;

        let args = ConvertArgs {
            url: None,
            config: Some(config_json.to_string()),
            config_file: None,
            name: None,
            protocol: None,
            auth: None,
            header: vec![],
            retries: 0,
            allow_tools: None,
            deny_tools: None,
            ping_interval: 30,
            ping_timeout: 10,
            logging: LoggingArgs {
                diagnostic: true,
                log_dir: None,
                log_file: None,
                otlp_endpoint: None,
                service_name: "mcp-proxy".to_string(),
            },
        };

        let result = parse_convert_config(&args);
        assert!(result.is_err(), "缺少 URL 的配置应该失败");

        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("缺少 url 或 baseUrl"),
            "错误消息应该提示缺少 URL"
        );
    }

    /// 测试 10: 直接 URL 模式（不使用 JSON 配置）
    #[test]
    fn test_parse_direct_url_mode() {
        let args = ConvertArgs {
            url: Some("https://api.example.com/mcp".to_string()),
            config: None,
            config_file: None,
            name: None,
            protocol: None,
            auth: None,
            header: vec![],
            retries: 0,
            allow_tools: None,
            deny_tools: None,
            ping_interval: 30,
            ping_timeout: 10,
            logging: LoggingArgs {
                diagnostic: true,
                log_dir: None,
                log_file: None,
                otlp_endpoint: None,
                service_name: "mcp-proxy".to_string(),
            },
        };

        let result = parse_convert_config(&args);
        assert!(result.is_ok());

        let config_source = result.unwrap();
        match config_source {
            McpConfigSource::DirectUrl { url } => {
                assert_eq!(url, "https://api.example.com/mcp");
            }
            _ => panic!("应该解析为 DirectUrl 类型"),
        }
    }

    /// 测试 11: 既没有 URL 也没有配置应该失败
    #[test]
    fn test_parse_no_url_no_config_should_fail() {
        let args = ConvertArgs {
            url: None,
            config: None,
            config_file: None,
            name: None,
            protocol: None,
            auth: None,
            header: vec![],
            retries: 0,
            allow_tools: None,
            deny_tools: None,
            ping_interval: 30,
            ping_timeout: 10,
            logging: LoggingArgs {
                diagnostic: true,
                log_dir: None,
                log_file: None,
                otlp_endpoint: None,
                service_name: "mcp-proxy".to_string(),
            },
        };

        let result = parse_convert_config(&args);
        assert!(result.is_err(), "既没有 URL 也没有配置应该失败");

        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("必须提供 URL"),
            "错误消息应该提示需要提供配置"
        );
    }

    /// 测试 12: 无效的 JSON 应该失败
    #[test]
    fn test_parse_invalid_json_should_fail() {
        let invalid_json = r#"{
            "mcpServers": {
                "test": {
                    "command": "node"
                }
            // 缺少闭合括号
        }"#;

        let args = ConvertArgs {
            url: None,
            config: Some(invalid_json.to_string()),
            config_file: None,
            name: None,
            protocol: None,
            auth: None,
            header: vec![],
            retries: 0,
            allow_tools: None,
            deny_tools: None,
            ping_interval: 30,
            ping_timeout: 10,
            logging: LoggingArgs {
                diagnostic: true,
                log_dir: None,
                log_file: None,
                otlp_endpoint: None,
                service_name: "mcp-proxy".to_string(),
            },
        };

        let result = parse_convert_config(&args);
        assert!(result.is_err(), "无效的 JSON 应该失败");

        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("配置解析失败"),
            "错误消息应该提示解析失败"
        );
    }

    /// 测试 13: Stream 协议类型解析
    #[test]
    fn test_parse_stream_protocol_type() {
        let config_json = r#"{
            "mcpServers": {
                "stream-service": {
                    "url": "https://api.example.com/mcp",
                    "type": "stream"
                }
            }
        }"#;

        let args = ConvertArgs {
            url: None,
            config: Some(config_json.to_string()),
            config_file: None,
            name: None,
            protocol: None,
            auth: None,
            header: vec![],
            retries: 0,
            allow_tools: None,
            deny_tools: None,
            ping_interval: 30,
            ping_timeout: 10,
            logging: LoggingArgs {
                diagnostic: true,
                log_dir: None,
                log_file: None,
                otlp_endpoint: None,
                service_name: "mcp-proxy".to_string(),
            },
        };

        let result = parse_convert_config(&args);
        assert!(result.is_ok());

        let config_source = result.unwrap();
        match config_source {
            McpConfigSource::RemoteService { protocol, .. } => {
                assert_eq!(format!("{:?}", protocol), "Some(Stream)");
            }
            _ => panic!("应该解析为 RemoteService 类型"),
        }
    }
}
