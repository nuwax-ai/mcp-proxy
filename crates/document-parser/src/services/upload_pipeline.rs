//! 文档解析产物的上传与换签管线。
//!
//! 从 [`crate::services::document_service`] 拆出：上传后端解析（OSS/自定义
//! 二选一，含请求级覆盖）、Markdown 内嵌图片的并发上传与 URL 重写、
//! AK 换签重写（sign_embedded_urls）以及相关的纯函数（span 提取/重建/
//! object key 构造）。

use anyhow::Result as AnyhowResult;
use async_trait::async_trait;
use oss_client::ApiFileClient;
use pulldown_cmark::{Event, LinkType, Parser, Tag};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::fs::{self, File};
use tokio::io::AsyncReadExt;

use crate::ImageInfo;
use crate::error::AppError;

/// 任务产物上传器：按 `task.upload_config` 选择后端（编译期穷举分派）
pub(crate) enum TaskUploader {
    /// 阿里云 OSS（现状默认）
    Oss(Arc<dyn oss_client::OssClientTrait + Send + Sync>),
    /// 自定义上传后端（nuwax 风格 REST）
    Custom(ApiFileClient),
    /// 无可用上传后端（OSS 未配置且任务未指定自定义端点）
    Disabled,
}

/// 一次解析出的上传上下文
pub(crate) struct ResolvedUploader {
    pub(crate) uploader: TaskUploader,
    /// 仅 OSS 分支使用：对象键前缀子目录（已 trim `/`）
    pub(crate) bucket_dir: Option<String>,
    /// 仅 Custom 分支使用：后端基地址（日志溯源）
    pub(crate) custom_base_url: Option<String>,
}

/// OSS 后端实现：行为与旧版完全一致（sha256 对象键去重 + bucket_dir 前缀）
pub(crate) struct OssImageOps<'a> {
    pub(crate) client: &'a Arc<dyn oss_client::OssClientTrait + Send + Sync>,
    pub(crate) bucket_dir: Option<&'a str>,
    pub(crate) task_id: &'a str,
}

#[async_trait]
impl ImageUploadOps for OssImageOps<'_> {
    async fn upload_image(&self, local_path: &Path) -> Result<ImageInfo, AppError> {
        let original_filename = local_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        // 计算文件SHA-256哈希作为对象键名称，确保相同图片去重
        let hash_hex = compute_file_sha256_hex(local_path).await.map_err(|e| {
            AppError::File(format!("计算图片哈希失败: {}: {}", local_path.display(), e))
        })?;

        // 保留原始扩展名
        let ext_lower = local_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        let ext_suffix = if ext_lower.is_empty() {
            String::new()
        } else {
            format!(".{ext_lower}")
        };

        let object_key =
            build_image_object_key(self.bucket_dir, self.task_id, &hash_hex, &ext_suffix);

        let oss_url = self
            .client
            .upload_file(local_path.to_string_lossy().as_ref(), &object_key)
            .await
            .map_err(|e| {
                AppError::Oss(format!(
                    "上传图片失败: {original_filename} -> {object_key}: {e}"
                ))
            })?;

        let file_size = read_file_size(local_path).await?;
        let mime_type = oss_client::detect_mime_type(local_path.to_string_lossy().as_ref());

        Ok(ImageInfo::with_full_info(
            local_path.to_string_lossy().to_string(),
            original_filename,
            object_key,
            oss_url,
            file_size,
            mime_type,
        ))
    }
}

/// 自定义后端实现：服务端自管 key（忽略哈希去重），上传后端返回的 url/key 为准
pub(crate) struct CustomImageOps<'a> {
    pub(crate) client: &'a ApiFileClient,
}

#[async_trait]
impl ImageUploadOps for CustomImageOps<'_> {
    async fn upload_image(&self, local_path: &Path) -> Result<ImageInfo, AppError> {
        let original_filename = local_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        let uploaded = self
            .client
            .upload_file_by_path(
                local_path.to_string_lossy().as_ref(),
                Some(&original_filename),
            )
            .await
            .map_err(|e| {
                AppError::Oss(format!(
                    "上传图片到自定义后端失败: {original_filename}: {e}"
                ))
            })?;

        let file_size = read_file_size(local_path).await?;
        let mime_type = oss_client::detect_mime_type(local_path.to_string_lossy().as_ref());

        Ok(ImageInfo::with_full_info(
            local_path.to_string_lossy().to_string(),
            original_filename,
            uploaded.key,
            uploaded.url,
            file_size,
            mime_type,
        ))
    }
}

/// 图片上传操作抽象：OSS 与自定义后端各一个实现
#[async_trait]
pub(crate) trait ImageUploadOps: Sync {
    /// 上传单张图片并返回填好后端真实值的 [`ImageInfo`]
    async fn upload_image(&self, local_path: &Path) -> Result<ImageInfo, AppError>;
}

/// 提取匹配后端前缀的内联图片/链接 dest 及其在原文中的字节 span
///
/// 使用 pulldown-cmark 事件流（offset 迭代器）——**跳过 code span 与 fenced
/// code block 内的文本**（解析器将二者折叠为 Code/CodeBlock 事件，不产生
/// Image/Link 事件），因此代码示例里的后端 URL 不会被误改写。仅处理
/// `LinkType::Inline`（dest 原文保证在事件 span 内；引用式链接的 dest 在
/// 文档别处，跳过）。span 内从 `](` 之后定位 dest 原文子串（避免命中
/// alt/链接文本中的同串——`![url](url)` 回显形态），转义/实体导致原文与
/// unescape 后的 dest 不一致时定位失败即安全跳过。
///
/// 返回值**按 span 起点严格递增且互不重叠**：事件流的前序遍历序在嵌套
/// Image-in-Link（可点击缩略图）下与位置序相反，故收集后排序，并跳过
/// 与前一 span 重叠的定位（同 URL 嵌套时 alt 内匹配等退化场景）。
pub(crate) fn extract_inline_dest_spans(
    content: &str,
    base_prefix: &str,
) -> Vec<(String, std::ops::Range<usize>)> {
    let mut spans: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    for (event, range) in Parser::new(content).into_offset_iter() {
        let (dest_url, link_type) = match event {
            Event::Start(Tag::Image {
                dest_url,
                link_type,
                ..
            }) => (dest_url, link_type),
            Event::Start(Tag::Link {
                dest_url,
                link_type,
                ..
            }) => (dest_url, link_type),
            _ => continue,
        };
        if link_type != LinkType::Inline {
            continue;
        }
        // 先前缀过滤再克隆：不匹配的链接（文档中绝大多数）零分配
        if !dest_url.starts_with(base_prefix) {
            continue;
        }
        let dest = dest_url.to_string();
        // 事件 span = `![alt](dest "title")` / `[text](dest)` 整体；dest 原文
        // 紧跟 text/dest 边界的 `](` 之后。用 `](dest` 模式 + **href 语法后验**
        // 定位（从右往左找，取最后一个满足者）：真 href 的 dest 之后必是 `)`
        // 或空白（进入 title）；title 内 `](` 字面量后的 dest 副本之后是
        // 引号等非法字符，被后验拒绝。从右往左保证嵌套 Image-in-Link 的
        // 外层命中自己的边界（内层 dest 在其 text 中，位置更靠左）。
        let needle = format!("]({dest}");
        let span_text = &content[range.clone()];
        let mut search_end = span_text.len();
        let mut located: Option<usize> = None; // span 内 dest 起点（局部坐标）
        while let Some(pos) = span_text[..search_end].rfind(needle.as_str()) {
            let after = pos + needle.len();
            let href_like = after >= span_text.len()
                || span_text[after..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_whitespace() || c == ')');
            if href_like {
                located = Some(pos + 2);
                break;
            }
            search_end = pos;
        }
        if let Some(local) = located {
            let start = range.start + local;
            let end = start + dest.len();
            spans.push((dest, start..end));
        }
    }
    // 事件序（前序遍历）≠ 位置序：嵌套 Image-in-Link 时外层 Link 的 span
    // 靠后却先入列——排序恢复位置序；再剔除重叠（同 URL 嵌套的退化定位）
    spans.sort_by_key(|(_, range)| range.start);
    let mut deduped: Vec<(String, std::ops::Range<usize>)> = Vec::with_capacity(spans.len());
    let mut last_end = 0;
    for (dest, range) in spans {
        if range.start >= last_end {
            last_end = range.end;
            deduped.push((dest, range));
        }
    }
    deduped
}

/// 按 span 单趟重建内容（O(M) 一次分配）
///
/// `signed` 缺失的 span（单张换签失败）保留原文；spans 必须按起点**严格
/// 递增且互不重叠**（重叠会使切片 start < end 而 panic）——
/// [`extract_inline_dest_spans`] 的输出即满足此契约。
pub(crate) fn rebuild_with_replacements(
    content: &str,
    spans: &[(String, std::ops::Range<usize>)],
    signed: &HashMap<String, String>,
) -> String {
    let mut out = String::with_capacity(content.len());
    let mut prev = 0;
    for (dest, range) in spans {
        out.push_str(&content[prev..range.start]);
        match signed.get(dest) {
            Some(replacement) => out.push_str(replacement),
            None => out.push_str(&content[range.clone()]),
        }
        prev = range.end;
    }
    out.push_str(&content[prev..]);
    out
}

/// 生成 Markdown 的对象键：`[bucket_dir/]processed_markdown/<task_id>/<task_id>.md`
///
/// 与旧 `generate_unified_oss_object_key` 的拼接逻辑逐字符一致。
pub(crate) fn build_markdown_object_key(bucket_dir: Option<&str>, task_id: &str) -> String {
    match bucket_dir
        .map(|d| d.trim_matches('/'))
        .filter(|d| !d.is_empty())
    {
        Some(dir) => format!("{dir}/processed_markdown/{task_id}/{task_id}.md"),
        None => format!("processed_markdown/{task_id}/{task_id}.md"),
    }
}

/// 生成图片的对象键：`[bucket_dir/]parsed_images/<task_id>/sha256/<hash><ext>`
///
/// 与旧 `generate_unified_oss_object_key` 的拼接逻辑逐字符一致。
pub(crate) fn build_image_object_key(
    bucket_dir: Option<&str>,
    task_id: &str,
    hash_hex: &str,
    ext_suffix: &str,
) -> String {
    match bucket_dir
        .map(|d| d.trim_matches('/'))
        .filter(|d| !d.is_empty())
    {
        Some(dir) => format!("{dir}/parsed_images/{task_id}/sha256/{hash_hex}{ext_suffix}"),
        None => format!("parsed_images/{task_id}/sha256/{hash_hex}{ext_suffix}"),
    }
}

/// 图片上传并发度：保守值（自定义后端多为用户系统单实例，避免压垮；
/// OSS 无压力）。后续需要时提升为配置项。
pub(crate) const IMAGE_UPLOAD_CONCURRENCY: usize = 4;
/// 图片 URL 换签并发度：换签是轻量元数据 GET（非文件上传），
/// 可高于上传并发（IMAGE_UPLOAD_CONCURRENCY）。
pub(crate) const IMAGE_SIGN_CONCURRENCY: usize = 8;
/// 内嵌图片换签整体预算：尽力而为的增强——超预算降级返回未换签
/// 内容（warn 日志），不让下载主体挂起或失败
pub(crate) const IMAGE_SIGN_BUDGET: Duration = Duration::from_secs(120);

/// 流式读取文件并计算 SHA-256（十六进制小写）。
pub(crate) async fn compute_file_sha256_hex(path: &Path) -> AnyhowResult<String> {
    let mut file = File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 读取文件大小（元数据）。
pub(crate) async fn read_file_size(local_path: &Path) -> Result<u64, AppError> {
    let meta = fs::metadata(local_path).await.map_err(|e| {
        AppError::file_error(format!("读取文件元数据失败 {}: {e}", local_path.display()))
    })?;
    Ok(meta.len())
}
