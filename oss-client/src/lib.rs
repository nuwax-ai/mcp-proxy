//! 简洁的阿里云OSS操作库
//!
//! 提供基本的文件上传、下载、删除功能，以及预签名URL生成。
//!
//! # 快速开始
//!
//! ```rust,no_run
//! use oss_client::{PrivateOssClient, OssConfig, OssClientTrait};
//!
//! #[tokio::main]
//! async fn main() -> oss_client::Result<()> {
//!     // 创建配置
//!     let config = OssConfig::new(
//!         "oss-rg-china-mainland.aliyuncs.com".to_string(),
//!         "bucket_name".to_string(),
//!         "access_key_id".to_string(),
//!         "access_key_secret".to_string(),
//!         "oss-rg-china-mainland".to_string(),
//!         "upload_directory".to_string(),
//!     );
//!
//!     // 创建客户端
//!     let client = PrivateOssClient::new(config)?;
//!
//!     // 上传文件
//!     let url = client.upload_file("local/file.txt", "remote/file.txt").await?;
//!     println!("文件上传成功: {}", url);
//!     Ok(())
//! }
//! ```

// 初始化 i18n，使用 crate 内置翻译文件
#[macro_use]
extern crate rust_i18n;

// 初始化翻译文件，使用 crate 内置 locales（支持独立发布）
i18n!("locales", fallback = "en");

pub mod config;
pub mod error;
pub mod private_client;
pub mod public_client;
pub mod utils;

// 重新导出主要类型
pub use config::{OssConfig, defaults};
pub use error::{OssError, Result};
pub use private_client::PrivateOssClient;
pub use public_client::PublicOssClient;

// 重新导出常用工具函数
pub use utils::{
    detect_mime_type, detect_mime_type_by_extension, format_file_size, generate_random_filename,
    get_file_extension, get_filename, get_filename_without_extension, is_audio_file,
    is_document_file, is_image_file, is_video_file, parse_file_size, replace_oss_domain,
    replace_oss_domains_batch, sanitize_filename,
};

/// OSS客户端公共接口trait
///
/// 定义了OSS客户端的基本操作接口，包括文件操作、签名URL生成等
/// 私有bucket和公有bucket客户端都实现这个trait
#[async_trait::async_trait]
pub trait OssClientTrait: Send + Sync {
    /// 获取配置信息
    fn get_config(&self) -> &OssConfig;

    /// 获取基础URL
    fn get_base_url(&self) -> String;

    /// 生成上传签名URL
    ///
    /// # 参数
    /// * `object_key` - 对象键
    /// * `expires_in` - 过期时间
    /// * `content_type` - 内容类型（可选）
    ///
    /// # 返回
    /// * 带签名的上传URL
    fn generate_upload_url(
        &self,
        object_key: &str,
        expires_in: std::time::Duration,
        content_type: Option<&str>,
    ) -> Result<String>;

    /// 生成下载签名URL
    ///
    /// # 参数
    /// * `object_key` - 对象键
    /// * `expires_in` - 过期时间（可选）
    ///
    /// # 返回
    /// * 带签名的下载URL
    fn generate_download_url(
        &self,
        object_key: &str,
        expires_in: Option<std::time::Duration>,
    ) -> Result<String>;

    /// 上传文件
    ///
    /// # 参数
    /// * `local_path` - 本地文件路径
    /// * `object_key` - 对象键
    ///
    /// # 返回
    /// * 上传后的文件URL
    async fn upload_file(&self, local_path: &str, object_key: &str) -> Result<String>;

    /// 上传内容
    ///
    /// # 参数
    /// * `content` - 要上传的内容
    /// * `object_key` - 对象键
    /// * `content_type` - 内容类型（可选）
    ///
    /// # 返回
    /// * 上传后的文件URL
    async fn upload_content(
        &self,
        content: &[u8],
        object_key: &str,
        content_type: Option<&str>,
    ) -> Result<String>;

    /// 删除文件
    ///
    /// # 参数
    /// * `object_key` - 对象键
    ///
    /// # 返回
    /// * 删除操作结果
    async fn delete_file(&self, object_key: &str) -> Result<()>;

    /// 检查文件是否存在
    ///
    /// # 参数
    /// * `object_key` - 对象键
    ///
    /// # 返回
    /// * 文件是否存在
    async fn file_exists(&self, object_key: &str) -> Result<bool>;

    /// 测试连接
    ///
    /// # 返回
    /// * 连接测试结果
    async fn test_connection(&self) -> Result<()>;

    /// 生成唯一的object key
    ///
    /// # 参数
    /// * `prefix` - 前缀
    /// * `filename` - 原始文件名（可选）
    ///
    /// # 返回
    /// * 唯一的对象键
    fn generate_object_key(&self, prefix: &str, filename: Option<&str>) -> String;
}

/// 获取库版本信息
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 获取库名称
pub fn name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

/// 获取库描述
pub fn description() -> &'static str {
    env!("CARGO_PKG_DESCRIPTION")
}
