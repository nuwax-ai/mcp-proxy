use crate::models::Config;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;
use tracing::info;

/// 服务类型枚举，定义服务模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceType {
    /// 单节点服务器模式
    Server,
}

impl ServiceType {
    /// 获取默认配置文件名
    pub fn default_config_filename(&self) -> &'static str {
        match self {
            ServiceType::Server => "server-config.yml",
        }
    }

    /// 获取服务显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            ServiceType::Server => "Server",
        }
    }

    /// 获取所有支持的服务类型
    pub fn all() -> &'static [ServiceType] {
        &[ServiceType::Server]
    }
}

/// 配置模板生成器
pub struct ConfigTemplateGenerator;

impl ConfigTemplateGenerator {
    /// 生成指定服务类型的配置文件
    pub fn generate_config_file(
        service_type: ServiceType,
        output_path: &PathBuf,
    ) -> crate::Result<()> {
        let template_content = Self::get_template_content(service_type)?;

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(output_path, template_content)?;

        info!(
            "Generated {} configuration file: {:?}",
            service_type.display_name(),
            output_path
        );

        Ok(())
    }

    /// 获取服务类型对应的模板内容
    fn get_template_content(service_type: ServiceType) -> crate::Result<&'static str> {
        match service_type {
            ServiceType::Server => Ok(include_str!("../templates/server-config.yml.template")),
        }
    }

    /// 生成所有类型的配置文件到指定目录
    pub fn generate_all_configs(
        output_dir: &PathBuf,
    ) -> crate::Result<HashMap<ServiceType, PathBuf>> {
        let mut generated_files = HashMap::new();

        for &service_type in ServiceType::all() {
            let filename = service_type.default_config_filename();
            let output_path = output_dir.join(filename);

            Self::generate_config_file(service_type, &output_path)?;
            generated_files.insert(service_type, output_path);
        }

        Ok(generated_files)
    }
}

/// Configuration change notification
#[derive(Debug, Clone)]
pub struct ConfigChangeNotification {
    pub old_config: Config,
    pub new_config: Config,
    pub changed_at: SystemTime,
}
