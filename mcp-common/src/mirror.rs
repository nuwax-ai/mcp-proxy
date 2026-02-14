//! 镜像源配置：通过进程级环境变量为 npx/uvx 子进程设置国内镜像源

/// 镜像源配置
#[derive(Debug, Clone, Default)]
pub struct MirrorConfig {
    pub npm_registry: Option<String>,
    pub pypi_index_url: Option<String>,
}

impl MirrorConfig {
    /// 从环境变量 `MCP_PROXY_NPM_REGISTRY` / `MCP_PROXY_PYPI_INDEX_URL` 加载
    pub fn from_env() -> Self {
        Self {
            npm_registry: std::env::var("MCP_PROXY_NPM_REGISTRY").ok(),
            pypi_index_url: std::env::var("MCP_PROXY_PYPI_INDEX_URL").ok(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.npm_registry.is_none() && self.pypi_index_url.is_none()
    }

    /// 设为进程级环境变量，所有子进程自动继承。
    ///
    /// # Safety
    /// 应在 main() 启动早期、单线程阶段调用。
    pub fn apply_to_process_env(&self) {
        unsafe {
            if let Some(ref registry) = self.npm_registry {
                if std::env::var("npm_config_registry").is_err() {
                    std::env::set_var("npm_config_registry", registry);
                }
            }
            if let Some(ref index_url) = self.pypi_index_url {
                if std::env::var("UV_INDEX_URL").is_err() {
                    std::env::set_var("UV_INDEX_URL", index_url);
                }
                if std::env::var("PIP_INDEX_URL").is_err() {
                    std::env::set_var("PIP_INDEX_URL", index_url);
                }
                if std::env::var("UV_INSECURE_HOST").is_err()
                    && index_url.starts_with("http://")
                {
                    if let Some(host) = extract_host(index_url) {
                        std::env::set_var("UV_INSECURE_HOST", &host);
                    }
                }
            }
        }
    }
}

/// 从 URL 中提取 host
fn extract_host(url: &str) -> Option<String> {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = without_scheme.split('/').next()?.split(':').next()?;
    if host.is_empty() { None } else { Some(host.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mirror_config_is_empty() {
        assert!(MirrorConfig::default().is_empty());
        assert!(!MirrorConfig {
            npm_registry: Some("test".to_string()),
            pypi_index_url: None,
        }
        .is_empty());
    }

    #[test]
    fn test_extract_host() {
        assert_eq!(
            extract_host("https://mirrors.aliyun.com/pypi/simple/"),
            Some("mirrors.aliyun.com".to_string())
        );
        assert_eq!(
            extract_host("https://example.com:8080/path"),
            Some("example.com".to_string())
        );
        assert_eq!(extract_host("not-a-url"), None);
    }
}
