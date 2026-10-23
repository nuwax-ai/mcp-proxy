//! 配置构建器：以默认 YAML 为基础逐项覆盖，用于测试与灵活配置创建。

use super::*;

/// 默认配置文件内容（目录化后相对路径上移一级）
const DEFAULT_CONFIG_YAML: &str = include_str!("../../config.yml");

/// 配置构建器，用于测试和灵活配置创建
#[derive(Debug, Default)]
pub struct ConfigBuilder {
    environment: Option<String>,
    server: Option<ServerConfig>,
    log: Option<LogConfig>,
    document_parser: Option<DocumentParserConfig>,
    mineru: Option<MinerUConfig>,
    markitdown: Option<MarkItDownConfig>,
    storage: Option<StorageConfig>,
    external_integration: Option<ExternalIntegrationConfig>,
    file_size_config: Option<GlobalFileSizeConfig>,
}

impl ConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn environment(mut self, environment: String) -> Self {
        self.environment = Some(environment);
        self
    }

    pub fn server(mut self, server: ServerConfig) -> Self {
        self.server = Some(server);
        self
    }

    pub fn log(mut self, log: LogConfig) -> Self {
        self.log = Some(log);
        self
    }

    pub fn document_parser(mut self, document_parser: DocumentParserConfig) -> Self {
        self.document_parser = Some(document_parser);
        self
    }

    pub fn mineru(mut self, mineru: MinerUConfig) -> Self {
        self.mineru = Some(mineru);
        self
    }

    pub fn markitdown(mut self, markitdown: MarkItDownConfig) -> Self {
        self.markitdown = Some(markitdown);
        self
    }

    pub fn storage(mut self, storage: StorageConfig) -> Self {
        self.storage = Some(storage);
        self
    }

    pub fn external_integration(mut self, external_integration: ExternalIntegrationConfig) -> Self {
        self.external_integration = Some(external_integration);
        self
    }

    pub fn file_size_config(mut self, file_size_config: GlobalFileSizeConfig) -> Self {
        self.file_size_config = Some(file_size_config);
        self
    }

    pub fn build(self) -> Result<AppConfig, ConfigError> {
        // 使用默认配置作为基础
        let mut config: AppConfig = serde_yaml::from_str(DEFAULT_CONFIG_YAML)
            .map_err(|e| ConfigError::Parse(e.to_string()))?;

        // 应用构建器中的配置
        if let Some(environment) = self.environment {
            config.environment = environment;
        }
        if let Some(server) = self.server {
            config.server = server;
        }
        if let Some(log) = self.log {
            config.log = log;
        }
        if let Some(document_parser) = self.document_parser {
            config.document_parser = document_parser;
        }
        if let Some(mineru) = self.mineru {
            config.mineru = mineru;
        }
        if let Some(markitdown) = self.markitdown {
            config.markitdown = markitdown;
        }
        if let Some(storage) = self.storage {
            config.storage = storage;
        }
        if let Some(external_integration) = self.external_integration {
            config.external_integration = external_integration;
        }
        if let Some(file_size_config) = self.file_size_config {
            config.file_size_config = file_size_config;
        }

        // 验证配置
        config.validate()?;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let server_config = ServerConfig {
            port: 9000,
            host: "127.0.0.1".to_string(),
        };

        let config = ConfigBuilder::new().server(server_config).build().unwrap();

        assert_eq!(config.server.port, 9000);
        assert_eq!(config.server.host, "127.0.0.1");
    }
}
