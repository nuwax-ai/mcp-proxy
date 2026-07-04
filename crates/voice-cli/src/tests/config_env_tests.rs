#[cfg(test)]
mod tests {
    use crate::models::config::MapEnv;
    use crate::models::Config;
    use tempfile::TempDir;

    /// 构造注入式 env（零全局态，测试可并行）
    fn map_env(pairs: &[(&str, &str)]) -> MapEnv {
        MapEnv(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    /// 临时目录里不存在的 config_path —— load_with_env_overrides 会创建默认配置后再应用 env 覆盖
    fn fresh_config_path() -> PathBuf {
        let temp_dir = TempDir::new().unwrap();
        temp_dir.path().join("config.yml")
    }

    use std::path::PathBuf;

    #[test]
    fn test_http_port_environment_override() {
        let env = map_env(&[("VOICE_CLI_PORT", "9090")]);
        let config = Config::load_with_env_overrides(&fresh_config_path(), &env).unwrap();
        assert_eq!(config.server.port, 9090);
    }

    #[test]
    fn test_invalid_port_environment_variable() {
        let env = map_env(&[("VOICE_CLI_PORT", "invalid_port")]);
        let result = Config::load_with_env_overrides(&fresh_config_path(), &env);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Invalid VOICE_CLI_PORT value 'invalid_port'"));
    }

    #[test]
    fn test_log_level_environment_override() {
        let env = map_env(&[("VOICE_CLI_LOG_LEVEL", "DEBUG")]);
        let config = Config::load_with_env_overrides(&fresh_config_path(), &env).unwrap();
        // Verify log level was overridden and normalized to lowercase
        assert_eq!(config.logging.level, "debug");
    }

    #[test]
    fn test_invalid_log_level_environment_variable() {
        let env = map_env(&[("VOICE_CLI_LOG_LEVEL", "invalid_level")]);
        let result = Config::load_with_env_overrides(&fresh_config_path(), &env);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Invalid VOICE_CLI_LOG_LEVEL"));
        assert!(error_msg.contains("invalid_level"));
    }

    #[test]
    fn test_comprehensive_validation() {
        // 使用有效的模型名称（large-v3 而不是 large）
        let env = map_env(&[
            ("VOICE_CLI_PORT", "8081"),
            ("VOICE_CLI_LOG_LEVEL", "warn"),
            ("VOICE_CLI_DEFAULT_MODEL", "large-v3"),
        ]);
        let config = Config::load_with_env_overrides(&fresh_config_path(), &env).unwrap();
        assert_eq!(config.server.port, 8081);
        assert_eq!(config.logging.level, "warn");
        assert_eq!(config.whisper.default_model, "large-v3");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_empty_environment_variable_validation() {
        // VOICE_CLI_HOST 为空白 → 应报错
        let env = map_env(&[("VOICE_CLI_HOST", "   ")]);
        let result = Config::load_with_env_overrides(&fresh_config_path(), &env);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("VOICE_CLI_HOST environment variable cannot be empty"));
    }
}
