use anyhow::Result;
use serde::Deserialize;
use std::env;
use std::fs::File;
use std::path::Path;

/// The default config file
const DEFAULT_CONFIG_YAML: &str = include_str!("../config.yml");

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub log: LogConfig,
}
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    /// The port to listen on for incoming connections
    pub port: u16,
}
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct LogConfig {
    /// The log level to use
    pub level: String,
    /// The path to the log file
    pub path: String,
    /// The number of log files to retain (default: 20)
    #[serde(default = "default_retain_days")]
    pub retain_days: u32,
}

/// Default log files to retain
fn default_retain_days() -> u32 {
    5
}

#[allow(dead_code)]
impl AppConfig {
    /// Load the config file from the following sources:
    /// 1. /app/config.yml
    /// 2. config.yml
    /// 3. BOT_SERVER_CONFIG environment variable
    pub fn load_config() -> Result<Self> {
        let ret = match (
            File::open("/app/config.yml"),
            File::open("config.yml"),
            env::var("BOT_SERVER_CONFIG"),
        ) {
            (Ok(file), _, _) => serde_yaml::from_reader(file),
            (_, Ok(file), _) => serde_yaml::from_reader(file),
            (_, _, Ok(file_path)) => serde_yaml::from_reader(File::open(file_path)?),
            _ => {
                // 如果都没有，则使用默认配置
                serde_yaml::from_str::<AppConfig>(DEFAULT_CONFIG_YAML)
            }
        };

        Ok(ret?)
    }

    pub fn log_path_init(&self) -> Result<()> {
        let log_path = &self.log.path;

        // 获取日志文件的父目录
        if let Some(parent) = Path::new(log_path).parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        Ok(())
    }
}
