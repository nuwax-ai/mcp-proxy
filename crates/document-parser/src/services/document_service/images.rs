//! `DocumentService` 的图片/上传后端族方法（从 document_service.rs 拆出）：
//! 解析产物图片上传（OSS / 自定义后端 / 换签）、Markdown 图片路径替换、
//! 产物 Markdown 上传。纯代码搬移，无行为变化。
//!
//! 子模块可访问父模块私有字段（Rust 可见性：定义模块及其后代），
//! `impl super::DocumentService` 保持方法解析与调用点不变。

use anyhow::{Context, Result as AnyhowResult};
use oss_client::ApiFileClient;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use pulldown_cmark_to_cmark::cmark;

use tokio::fs;

use crate::models::ParseResult;
use crate::services::upload_pipeline::{
    CustomImageOps, IMAGE_SIGN_BUDGET, IMAGE_SIGN_CONCURRENCY, IMAGE_UPLOAD_CONCURRENCY,
    ImageUploadOps, OssImageOps, ResolvedUploader, TaskUploader, build_markdown_object_key,
    extract_inline_dest_spans, rebuild_with_replacements,
};
use crate::{ImageInfo, ProcessingStage};

impl super::DocumentService {
    /// 使用指定上传器处理图片上传和路径替换（不负责解析任务的上传后端）
    pub(super) async fn process_images_with_uploader(
        &self,
        task_id: &str,
        parse_result: ParseResult,
        resolved: &ResolvedUploader,
    ) -> AnyhowResult<ParseResult> {
        // 1. 检查上传后端是否可用
        match &resolved.uploader {
            TaskUploader::Disabled => {
                warn!("The OSS client is not configured and image uploading is skipped.");
                return Ok(parse_result);
            }
            TaskUploader::Oss(_) => {
                info!("Using OSS backend for image upload: {}", task_id);
            }
            TaskUploader::Custom(_) => {
                info!(
                    "Using custom upload backend for image upload: {} -> {}",
                    resolved.custom_base_url.as_deref().unwrap_or(""),
                    task_id
                );
            }
        }

        // 2. 扫描 MinerU 输出目录图片：优先 auto/images，再兼容 images
        let mut local_image_paths: Vec<PathBuf> = Vec::new();
        if let Some(output_path) = parse_result.output_dir.as_ref() {
            let collected = Self::collect_local_images_from_output(output_path).await?;
            info!("Scan to {} image files for upload", collected.len());
            local_image_paths.extend(collected);
        }

        // 3. 更新任务状态
        self.update_task_stage_safe(task_id, ProcessingStage::UploadingImages)
            .await;

        // 4. 按后端分派上传，生成文件名->URL 的映射列表
        let image_results: Vec<ImageInfo> = match &resolved.uploader {
            TaskUploader::Disabled => Vec::new(),
            TaskUploader::Oss(client) => {
                let ops = OssImageOps {
                    client,
                    bucket_dir: resolved.bucket_dir.as_deref(),
                    task_id,
                };
                self.upload_images_via(&ops, local_image_paths).await?
            }
            TaskUploader::Custom(client) => {
                let ops = CustomImageOps { client };
                self.upload_images_via(&ops, local_image_paths).await?
            }
        };
        info!(
            "Successfully uploaded {} pictures to the upload backend",
            image_results.len()
        );
        debug!("image_results: {:?}", image_results);

        // 5. 更新任务状态
        self.update_task_stage_safe(task_id, ProcessingStage::ReplacingImagePaths)
            .await;

        // 6. 替换Markdown中的图片路径
        let updated_content = self
            .replace_image_paths_in_markdown(&parse_result.markdown_content, &image_results)
            .await?;

        // 7. 创建新的解析结果（move 而非 clone：markdown_content 已在上方借用结束，
        //    深拷贝整个 Markdown 再立即覆盖是无谓开销）
        let mut final_result = parse_result;
        final_result.markdown_content = updated_content;

        info!(
            "Image path replacement completed, content length: {} characters, {} image paths replaced",
            final_result.markdown_content.len(),
            image_results.len()
        );
        Ok(final_result)
    }

    /// 同步解析链路：把解析产物图片上传到自定义后端并替换 Markdown 内路径
    ///
    /// Markdown 本身不上传（同步接口在响应体内直接返回内容）。
    /// 不创建任务、不更新任务状态。
    pub async fn upload_images_for_custom_endpoint(
        &self,
        endpoint: &crate::models::UploadEndpoint,
        parse_result: ParseResult,
    ) -> AnyhowResult<ParseResult> {
        let client = ApiFileClient::with_client(endpoint.to_api_config(), self.http_client.clone())
            .map_err(|e| anyhow::anyhow!("自定义上传客户端初始化失败: {e}"))?;
        let ops = CustomImageOps { client: &client };

        let mut local_image_paths: Vec<PathBuf> = Vec::new();
        if let Some(output_path) = parse_result.output_dir.as_ref() {
            let collected = Self::collect_local_images_from_output(output_path).await?;
            info!("Scan to {} image files for upload", collected.len());
            local_image_paths.extend(collected);
        }

        let image_results = self.upload_images_via(&ops, local_image_paths).await?;
        let updated_content = self
            .replace_image_paths_in_markdown(&parse_result.markdown_content, &image_results)
            .await?;

        let mut final_result = parse_result;
        final_result.markdown_content = updated_content;
        Ok(final_result)
    }

    /// 递归查找 `images` 目录，并收集其下所有图片文件
    pub(super) async fn collect_local_images_from_output(
        output_path: &str,
    ) -> AnyhowResult<Vec<PathBuf>> {
        debug!(
            "Scan the directory named 'images' under the output directory: {}",
            output_path
        );
        let base_dir = Path::new(output_path);
        let mut found: Vec<PathBuf> = Vec::new();

        if !base_dir.exists() || !base_dir.is_dir() {
            return Ok(found);
        }

        // 第一步：在整个 output_path 下递归查找名为 "images" 的目录
        let mut to_visit: Vec<PathBuf> = vec![base_dir.to_path_buf()];
        let mut images_dirs: Vec<PathBuf> = Vec::new();

        while let Some(dir) = to_visit.pop() {
            let mut rd = fs::read_dir(&dir)
                .await
                .with_context(|| format!("读取目录失败: {}", dir.display()))?;
            while let Some(entry) = rd
                .next_entry()
                .await
                .with_context(|| format!("遍历目录失败: {}", dir.display()))?
            {
                let path = entry.path();
                // 优先通过 metadata 判断类型，避免竞态
                match fs::metadata(&path).await {
                    Ok(meta) if meta.is_dir() => {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str())
                            && name == "images"
                        {
                            images_dirs.push(path.clone());
                        }
                        to_visit.push(path);
                    }
                    _ => {}
                }
            }
        }

        // 第二步：对每个 images 目录进行递归遍历，收集图片文件
        for root in images_dirs {
            let mut stack: Vec<PathBuf> = vec![root.clone()];
            while let Some(dir) = stack.pop() {
                let mut rd = fs::read_dir(&dir)
                    .await
                    .with_context(|| format!("读取图片目录失败: {}", dir.display()))?;
                while let Some(entry) = rd
                    .next_entry()
                    .await
                    .with_context(|| format!("遍历图片目录失败: {}", dir.display()))?
                {
                    let path = entry.path();
                    match fs::metadata(&path).await {
                        Ok(meta) if meta.is_file() => {
                            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
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
                                    found.push(path);
                                }
                            }
                        }
                        Ok(meta) if meta.is_dir() => {
                            stack.push(path);
                        }
                        _ => {}
                    }
                }
            }
        }

        // 去重（同名同路径不重复）
        found.sort();
        found.dedup();
        Ok(found)
    }

    /// 替换Markdown中的图片路径
    pub async fn replace_image_paths_in_markdown(
        &self,
        markdown_content: &str,
        image_results: &[ImageInfo],
    ) -> AnyhowResult<String> {
        info!("Replace {} image paths in Markdown", image_results.len());

        // 创建文件名到OSS URL的映射表
        let mut filename_to_oss_url = HashMap::new();
        for result in image_results {
            // 使用文件名作为键（用于匹配Markdown中的图片引用）
            filename_to_oss_url.insert(result.original_filename.clone(), result.oss_url.clone());
            // 同时支持以哈希命名的文件匹配（去重后场景，Markdown 里可能是原名，也可能是路径/原名）
            if let Some(stem) = std::path::Path::new(&result.original_filename)
                .file_stem()
                .and_then(|s| s.to_str())
            {
                filename_to_oss_url
                    .entry(stem.to_string())
                    .or_insert(result.oss_url.clone());
            }
        }

        info!("Created {} filename mappings", filename_to_oss_url.len());

        // 使用pulldown-cmark解析Markdown，直接修改Event
        let parser = Parser::new(markdown_content);
        let mut updated_events = Vec::new();
        let mut replacements_count = 0;

        for event in parser {
            match event {
                Event::Start(Tag::Image {
                    dest_url,
                    link_type,
                    id,
                    title,
                }) => {
                    let original_url = dest_url.to_string();
                    if let Some(oss_url) =
                        self.find_oss_url_for_filename(&original_url, &filename_to_oss_url)
                    {
                        // 创建新的Image标签，使用OSS URL
                        let new_tag = Tag::Image {
                            dest_url: pulldown_cmark::CowStr::from(oss_url),
                            link_type,
                            id,
                            title,
                        };
                        updated_events.push(Event::Start(new_tag));
                        replacements_count += 1;
                    } else {
                        // 如果没有找到匹配的OSS URL，保持原样
                        updated_events.push(Event::Start(Tag::Image {
                            dest_url,
                            link_type,
                            id,
                            title,
                        }));
                    }
                }
                Event::End(TagEnd::Image) => {
                    updated_events.push(Event::End(TagEnd::Image));
                }
                _ => {
                    updated_events.push(event);
                }
            }
        }

        // 使用pulldown-cmark-to-cmark将修改后的Event转换回Markdown
        let mut output = String::new();
        cmark(updated_events.into_iter(), &mut output)?;

        info!(
            "Image path replacement completed, {} images processed, {} paths replaced",
            image_results.len(),
            replacements_count
        );
        Ok(output)
    }

    /// 查找匹配的OSS URL（通过文件名匹配）
    fn find_oss_url_for_filename(
        &self,
        image_path: &str,
        filename_to_oss_url: &std::collections::HashMap<String, String>,
    ) -> Option<String> {
        // 从 Markdown 图片路径中提取文件名进行匹配
        if let Some(filename) = Path::new(image_path).file_name()
            && let Some(filename_str) = filename.to_str()
            && let Some(oss_url) = filename_to_oss_url.get(filename_str)
        {
            return Some(oss_url.clone());
        }

        // 如果没有找到匹配，返回None
        None
    }

    pub(crate) async fn upload_images_via(
        &self,
        ops: &dyn ImageUploadOps,
        local_image_paths: Vec<PathBuf>,
    ) -> AnyhowResult<Vec<ImageInfo>> {
        use futures::StreamExt as _;
        use futures::TryStreamExt as _;

        // 预过滤：无文件名（如路径以 .. 结尾）的跳过（与旧串行版一致）
        let uploadable: Vec<PathBuf> = local_image_paths
            .into_iter()
            .filter(|p| {
                let has_name = p.file_name().and_then(|n| n.to_str()).is_some();
                if !has_name {
                    warn!("Unable to get file name, skipping: {}", p.display());
                }
                has_name
            })
            .collect();

        let image_results: Vec<ImageInfo> = futures::stream::iter(uploadable)
            .map(|path| async move {
                // 直接传播 ops 的 AppError：其错误信息已含文件名/object_key/后端错误
                // 完整上下文；外层再包 context 会令消费端 to_string() 只见外层
                ops.upload_image(&path).await
            })
            .buffer_unordered(IMAGE_UPLOAD_CONCURRENCY)
            .try_collect()
            .await?;

        Ok(image_results)
    }

    /// 把 Markdown 内容中指向自定义后端的内联图片/链接 URL 批量换签重写
    ///
    /// 基于 [`extract_inline_dest_spans`]（CommonMark 语义，跳过代码块）提取
    /// 后并发换签（失败单张 warn 保留原文）；全部失败或无匹配时**原样返回
    /// 输入 Vec（零拷贝）**。非 UTF-8 内容不处理，原样返回。
    /// 调用方负责整体预算（本方法为尽力而为的增强，不应让下载主体失败）。
    pub async fn sign_embedded_urls(
        &self,
        content: Vec<u8>,
        client: &ApiFileClient,
        base_url: &str,
    ) -> Vec<u8> {
        use futures::StreamExt as _;

        let Ok(text) = std::str::from_utf8(&content) else {
            return content; // 非 UTF-8（二进制）不处理
        };
        let base_prefix = format!("{}/api/f/", base_url.trim_end_matches('/'));

        let spans = extract_inline_dest_spans(text, &base_prefix);
        if spans.is_empty() {
            return content; // 无匹配：零拷贝
        }

        // 去重后并发换签（轻量元数据 GET，并发高于上传）。
        // 超时只包网络阶段（URL 字符串进出），content 所有权不进被取消的
        // future——超预算降级为返回原内容而非丢失内容
        let unique: Vec<String> = spans
            .iter()
            .map(|(dest, _)| dest.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        info!(
            "Exchanging signed URLs for {} embedded backend image/link(s)",
            unique.len()
        );
        let exchange = futures::StreamExt::map(futures::stream::iter(unique), |url| async move {
            client
                .exchange_signed_url(&url)
                .await
                .map(|signed| (url, signed))
        })
        .buffer_unordered(IMAGE_SIGN_CONCURRENCY)
        .collect::<Vec<_>>();
        let results = match tokio::time::timeout(IMAGE_SIGN_BUDGET, exchange).await {
            Ok(results) => results,
            Err(_) => {
                warn!(
                    "Embedded image signed-URL rewriting timed out ({}s), returning unsigned content",
                    IMAGE_SIGN_BUDGET.as_secs()
                );
                return content; // 降级：原内容原样返回（零拷贝）
            }
        };

        let signed: HashMap<String, String> = results
            .into_iter()
            .filter_map(|result| match result {
                Ok(pair) => Some(pair),
                Err(e) => {
                    warn!("Image URL signed-exchange failed, keeping original: {e}");
                    None
                }
            })
            .collect();
        if signed.is_empty() {
            return content; // 全部失败：零拷贝
        }

        rebuild_with_replacements(text, &spans, &signed).into_bytes()
    }

    pub(super) async fn upload_processed_markdown_to_oss(
        &self,
        task_id: &str,
        markdown_content: &str,
        resolved: &ResolvedUploader,
    ) -> AnyhowResult<(String, String)> {
        match &resolved.uploader {
            TaskUploader::Disabled => Err(anyhow::anyhow!("OSS客户端未配置，无法上传Markdown内容")),
            TaskUploader::Oss(oss_client) => {
                info!(
                    "Start uploading the processed Markdown content to OSS: {}",
                    task_id
                );
                let object_key = build_markdown_object_key(resolved.bucket_dir.as_deref(), task_id);
                let upload_result = oss_client
                    .upload_content(
                        markdown_content.as_bytes(),
                        &object_key,
                        Some("text/markdown"),
                    )
                    .await
                    .with_context(|| format!("上传Markdown内容到OSS失败: {object_key}"))?;
                info!(
                    "Markdown content uploaded successfully: {} -> {}",
                    object_key, upload_result
                );
                Ok((upload_result, object_key))
            }
            TaskUploader::Custom(client) => {
                info!(
                    "Start uploading the processed Markdown content to custom backend: {}",
                    task_id
                );
                let uploaded = client
                    .upload_bytes(
                        markdown_content.as_bytes(),
                        &format!("{task_id}.md"),
                        Some("text/markdown"),
                    )
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("上传Markdown内容到自定义后端失败 ({}): {e}", task_id)
                    })?;
                info!(
                    "Markdown content uploaded successfully to custom backend: {} -> {}",
                    uploaded.key, uploaded.url
                );
                Ok((uploaded.url, uploaded.key))
            }
        }
    }
}
