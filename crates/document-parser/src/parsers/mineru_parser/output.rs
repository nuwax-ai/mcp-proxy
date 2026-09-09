//! `MinerUParser` 的输出读取族方法（从 mineru_parser.rs 拆出）：工作目录清理、
//! 输出目录调试信息、Markdown 输出定位与读取。纯代码搬移，无行为变化。

use crate::error::AppError;
use std::path::Path;
use tokio::fs;
use tracing::{debug, error, info, warn};

impl super::MinerUParser {
    /// 清理工作目录
    pub(super) async fn cleanup_work_dir(&self, work_dir: &Path) {
        if let Err(e) = fs::remove_dir_all(work_dir).await {
            warn!(
                "Failed to clean working directory: {} - {}",
                work_dir.display(),
                e
            );
        } else {
            debug!("Cleaned working directory: {}", work_dir.display());
        }
    }

    /// 调试输出目录内容
    pub(super) async fn debug_output_directory(
        &self,
        output_dir: &Path,
    ) -> Result<String, AppError> {
        let mut debug_info = format!("输出目录: {}\n", output_dir.display());

        if !output_dir.exists() {
            return Ok(format!("{debug_info} (目录不存在)"));
        }

        if !output_dir.is_dir() {
            return Ok(format!("{debug_info} (不是目录)"));
        }

        let mut entries = fs::read_dir(output_dir)
            .await
            .map_err(|e| AppError::File(format!("读取输出目录失败: {e}")))?;

        let mut file_count = 0;
        let mut dir_count = 0;
        let mut total_size = 0u64;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AppError::File(format!("遍历输出目录失败: {e}")))?
        {
            let path = entry.path();
            let metadata = match fs::metadata(&path).await {
                Ok(m) => m,
                Err(_) => continue,
            };

            if metadata.is_file() {
                file_count += 1;
                total_size += metadata.len();
                debug_info.push_str(&format!(
                    "  文件: {} ({} 字节)\n",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    metadata.len()
                ));
            } else if metadata.is_dir() {
                dir_count += 1;
                debug_info.push_str(&format!(
                    "  目录: {}\n",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
        }

        debug_info.push_str(&format!(
            "总计: {file_count} 个文件, {dir_count} 个目录, 总大小: {total_size} 字节"
        ));

        Ok(debug_info)
    }

    /// 读取Markdown输出
    pub(super) async fn read_markdown_output(&self, output_dir: &Path) -> Result<String, AppError> {
        debug!("Read Markdown output: {}", output_dir.display());

        // 递归查找所有markdown文件
        let mut markdown_files = Vec::new();
        self.find_markdown_files_recursively(output_dir, &mut markdown_files)
            .await?;

        if markdown_files.is_empty() {
            error!(
                "No Markdown files found in the output directory: {}",
                output_dir.display()
            );
            // 提供调试信息
            match self.debug_output_directory(output_dir).await {
                Ok(debug_info) => {
                    error!("Output directory debugging information: {}", debug_info);
                }
                Err(debug_err) => {
                    error!(
                        "Unable to obtain output directory debugging information: {}",
                        debug_err
                    );
                }
            }
            return Err(AppError::MinerU("未找到Markdown输出文件".to_string()));
        }

        // 按文件大小排序，选择最大的文件（通常是主要内容）
        markdown_files.sort_by_key(|path| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0));
        markdown_files.reverse();

        let selected_file = &markdown_files[0];
        let file_size = std::fs::metadata(selected_file)
            .map(|m| m.len())
            .unwrap_or(0);
        debug!(
            "Select Markdown file: {} (Size: {} bytes)",
            selected_file.display(),
            file_size
        );

        // 读取markdown文件
        let content = fs::read_to_string(selected_file)
            .await
            .map_err(|e| AppError::File(format!("读取Markdown文件失败: {e}")))?;

        // 验证内容不为空
        if content.trim().is_empty() {
            return Err(AppError::MinerU("Markdown文件内容为空".to_string()));
        }

        info!(
            "Successfully read Markdown file: {}, size: {} bytes",
            selected_file.display(),
            content.len()
        );
        Ok(content)
    }

    /// 递归查找所有Markdown文件
    async fn find_markdown_files_recursively(
        &self,
        dir: &Path,
        markdown_files: &mut Vec<std::path::PathBuf>,
    ) -> Result<(), AppError> {
        self.find_markdown_files_recursively_impl(dir, markdown_files)
            .await
    }

    /// 递归查找所有Markdown文件的实现（使用Box避免递归Future问题）
    async fn find_markdown_files_recursively_impl(
        &self,
        dir: &Path,
        markdown_files: &mut Vec<std::path::PathBuf>,
    ) -> Result<(), AppError> {
        if !dir.exists() || !dir.is_dir() {
            return Ok(());
        }

        let mut entries = fs::read_dir(dir)
            .await
            .map_err(|e| AppError::File(format!("读取目录失败: {e}")))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AppError::File(format!("遍历目录失败: {e}")))?
        {
            let path = entry.path();

            if path.is_file() {
                // 检查是否是Markdown文件
                if let Some(ext) = path.extension().and_then(|s| s.to_str())
                    && ext.to_lowercase() == "md"
                {
                    markdown_files.push(path.clone());
                    debug!("Markdown file found: {}", path.display());
                }
            } else if path.is_dir() {
                // 递归搜索子目录
                Box::pin(self.find_markdown_files_recursively_impl(&path, markdown_files)).await?;
            }
        }

        Ok(())
    }
}
