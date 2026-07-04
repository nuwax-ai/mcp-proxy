use anyhow::{Context, Result};
use log::info;
use std::path::{Path, PathBuf};
use tokio::fs::{self, File, create_dir_all};
use tokio::io::AsyncWriteExt;

use crate::model::LanguageScript;
///针对用的代码，进行检测和缓存
pub struct CodeFileCache;

impl CodeFileCache {
    /// 检查代码文件缓存
    pub fn obtain_code_hash(code: &str) -> String {
        // 使用BLAKE3计算哈希，更快速且安全
        let hash = blake3::hash(code.as_bytes());
        let hash_str = hash.to_hex().to_string();
        info!("计算代码hash值: {}", &hash_str);
        hash_str
    }

    /// 根据代码的hash检查是否存在缓存
    pub async fn check_code_file_cache_exisht(hash: &str, language: &LanguageScript) -> bool {
        fs::try_exists(Self::get_cache_file_path(hash, language))
            .await
            .unwrap_or(false)
    }

    /// 获取代码文件缓存
    pub async fn get_code_file_cache(
        hash: &str,
        language: &LanguageScript,
    ) -> Result<(File, PathBuf)> {
        let file_path = Self::get_cache_file_path(hash, language);
        let file = File::open(&file_path)
            .await
            .with_context(|| format!("无法打开缓存文件: {}", file_path.display()))?;
        Ok((file, file_path))
    }

    /// 保存代码文件缓存
    pub async fn save_code_file_cache(
        hash: &str,
        code: &str,
        language: &LanguageScript,
    ) -> Result<(File, PathBuf)> {
        // 确保缓存目录存在
        let cache_dir = Self::get_cache_dir();
        //检查目录是否存在
        if !fs::try_exists(&cache_dir).await.unwrap_or(false) {
            create_dir_all(&cache_dir)
                .await
                .with_context(|| format!("无法创建缓存目录: {}", cache_dir.display()))?;
        }

        // 构建文件路径并保存代码
        let file_path = Self::get_cache_file_path(hash, language);
        let mut file = File::create(&file_path)
            .await
            .with_context(|| format!("无法创建缓存文件: {}", file_path.display()))?;

        file.write_all(code.as_bytes())
            .await
            .with_context(|| "写入代码到缓存文件失败")?;

        // 设置文件权限
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&file_path).await?.permissions();
            perms.set_mode(0o755); // rwxr-xr-x
            fs::set_permissions(&file_path, perms).await?;
        }

        let file_path = Self::get_cache_file_path(hash, language);
        Ok((file, file_path))
    }

    /// 清除指定语言的缓存文件
    pub async fn clear_cache_by_language(language: &LanguageScript) -> Result<()> {
        let cache_dir = Self::get_cache_dir();
        if !fs::try_exists(&cache_dir).await.unwrap_or(false) {
            return Ok(());
        }

        let suffix = language.get_file_suffix();
        let mut entries = fs::read_dir(cache_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if let Some(file_name) = path.file_name()
                && let Some(name_str) = file_name.to_str()
                && name_str.ends_with(suffix)
            {
                fs::remove_file(&path)
                    .await
                    .with_context(|| format!("无法删除缓存文件: {}", path.display()))?;
            }
        }

        Ok(())
    }

    /// 清除所有缓存文件
    pub async fn clear_all_cache() -> Result<()> {
        let cache_dir = Self::get_cache_dir();
        if fs::try_exists(&cache_dir).await.unwrap_or(false) {
            fs::remove_dir_all(&cache_dir)
                .await
                .with_context(|| format!("无法删除缓存目录: {}", cache_dir.display()))?;
        }
        Ok(())
    }

    /// 获取缓存目录路径
    fn get_cache_dir() -> PathBuf {
        // 在容器环境中使用固定路径
        Path::new("/tmp/code_cache").to_path_buf()
    }

    /// 获取缓存文件路径
    fn get_cache_file_path(hash: &str, language: &LanguageScript) -> PathBuf {
        let mut path = Self::get_cache_dir();

        // 根据语言类型添加对应的文件扩展名
        let file_name = format!("{}{}", hash, language.get_file_suffix());

        path.push(file_name);
        path
    }
}
