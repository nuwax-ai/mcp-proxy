//! `MarkItDownParser` 的输出后处理族方法（从 markitdown_parser.rs 拆出）：
//! 按格式的内容后处理（Excel/PowerPoint/Word）、工作目录清理与图片收集。
//! 纯代码搬移，无行为变化。

use crate::error::AppError;
use crate::models::DocumentFormat;
use std::path::Path;
use tokio::fs;
use tracing::{debug, info, warn};

impl super::MarkItDownParser {
    /// 后处理内容
    pub(super) async fn post_process_content(
        &self,
        content: &str,
        format: &DocumentFormat,
    ) -> Result<String, AppError> {
        let mut processed_content = content.to_string();

        if self.config.quality_settings.clean_output {
            // 清理多余的空行
            processed_content = processed_content
                .lines()
                .collect::<Vec<_>>()
                .join("\n")
                .replace("\n\n\n", "\n\n");

            // 修复常见的格式问题
            processed_content = processed_content
                .replace("# #", "#")
                .replace("## ##", "##")
                .replace("### ###", "###");
        }

        // 根据格式进行特定的后处理
        match format {
            DocumentFormat::Excel => {
                // Excel表格的特殊处理
                processed_content = self.post_process_excel_content(&processed_content).await?;
            }
            DocumentFormat::PowerPoint => {
                // PowerPoint幻灯片的特殊处理
                processed_content = self
                    .post_process_powerpoint_content(&processed_content)
                    .await?;
            }
            DocumentFormat::Word => {
                // Word文档的特殊处理
                processed_content = self.post_process_word_content(&processed_content).await?;
            }
            _ => {}
        }

        Ok(processed_content)
    }

    /// 后处理Excel内容
    pub(super) async fn post_process_excel_content(
        &self,
        content: &str,
    ) -> Result<String, AppError> {
        // 改进表格格式
        let mut processed = content.to_string();

        // 确保表格有适当的标题
        if !processed.contains("# ") && processed.contains("|") {
            processed = format!("# Excel数据\n\n{processed}");
        }

        Ok(processed)
    }

    /// 后处理PowerPoint内容
    pub(super) async fn post_process_powerpoint_content(
        &self,
        content: &str,
    ) -> Result<String, AppError> {
        let mut processed = content.to_string();

        // 为幻灯片添加分隔符
        processed = processed.replace("Slide ", "\n---\n\n# Slide ");

        Ok(processed)
    }

    /// 后处理Word内容
    pub(super) async fn post_process_word_content(
        &self,
        content: &str,
    ) -> Result<String, AppError> {
        let mut processed = content.to_string();

        // 改进标题层次结构
        let lines: Vec<&str> = processed.lines().collect();
        let mut result_lines = Vec::new();

        for line in lines {
            if line.trim().is_empty() {
                result_lines.push(line.to_string());
                continue;
            }

            // 检测可能的标题
            if line.len() < 100
                && !line.starts_with('#')
                && (line
                    .chars()
                    .all(|c| c.is_uppercase() || c.is_whitespace() || c.is_numeric())
                    || line.ends_with(':'))
            {
                result_lines.push(format!("## {}", line.trim_end_matches(':')));
            } else {
                result_lines.push(line.to_string());
            }
        }

        processed = result_lines.join("\n");
        Ok(processed)
    }

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

    /// 收集图片文件
    #[allow(dead_code)]
    pub(super) async fn collect_images(&self, work_dir: &Path) -> Result<Vec<String>, AppError> {
        debug!("Collect picture files: {}", work_dir.display());

        let mut images = Vec::new();

        if !work_dir.exists() {
            return Ok(images);
        }

        let collected = self.collect_images_from_dir(work_dir).await?;
        images.extend(collected);

        // 去重
        images.sort();
        images.dedup();

        info!("{} picture files collected", images.len());
        Ok(images)
    }

    /// 从指定目录收集图片
    #[allow(dead_code)]
    fn collect_images_from_dir<'a>(
        &'a self,
        dir: &'a Path,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<String>, AppError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let mut images = Vec::new();

            let mut entries = fs::read_dir(dir)
                .await
                .map_err(|e| AppError::File(format!("读取目录失败: {} - {}", dir.display(), e)))?;

            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| AppError::File(format!("遍历目录失败: {} - {}", dir.display(), e)))?
            {
                let path = entry.path();

                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                        let ext_lower = ext.to_lowercase();
                        if matches!(
                            ext_lower.as_str(),
                            "png"
                                | "jpg"
                                | "jpeg"
                                | "gif"
                                | "bmp"
                                | "webp"
                                | "svg"
                                | "tiff"
                                | "tif"
                        ) {
                            // 验证文件不为空
                            if let Ok(metadata) = fs::metadata(&path).await
                                && metadata.len() > 0
                            {
                                images.push(path.to_string_lossy().to_string());
                                debug!("Image file found: {}", path.display());
                            }
                        }
                    }
                } else if path.is_dir() {
                    // 递归搜索子目录
                    let sub_images = self.collect_images_from_dir(&path).await?;
                    images.extend(sub_images);
                }
            }

            Ok(images)
        })
    }
}
