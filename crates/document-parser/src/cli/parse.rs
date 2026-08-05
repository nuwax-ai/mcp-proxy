//! `parse` 子命令：本地单文件解析（不依赖 OSS）。

use anyhow::{Context as _, Result};
use document_parser::{
    AppConfig, ParserEngine,
    parsers::DualEngineParser,
    processors::{MarkdownProcessor, MarkdownProcessorConfig},
    services::{DocumentService, DocumentServiceConfig, TaskService},
    utils::environment_manager::EnvironmentManager,
};
use log::{info, warn};
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

/// 处理文件解析命令
pub async fn handle_parse_command(
    app_config: &AppConfig,
    environment_manager: &EnvironmentManager,
    input: PathBuf,
    output: Option<PathBuf>,
    parser: String,
) -> Result<()> {
    info!("Start parsing file: {input:?}");
    info!("Use parser: {parser}");

    // 检查输入文件是否存在
    if !input.exists() {
        return Err(anyhow::anyhow!("输入文件不存在: {:?}", input));
    }

    // 检查环境
    let env_status = environment_manager
        .check_environment()
        .await
        .context("环境检查失败")?;

    match parser.as_str() {
        "mineru" => {
            if !env_status.mineru_available {
                return Err(anyhow::anyhow!(
                    "MinerU未安装，请先运行 'document-parser install'"
                ));
            }
        }
        "markitdown" => {
            if !env_status.markitdown_available {
                return Err(anyhow::anyhow!(
                    "MarkItDown未安装，请先运行 'document-parser install'"
                ));
            }
        }
        _ => {
            return Err(anyhow::anyhow!(
                "不支持的解析器: {}，支持的解析器: mineru, markitdown",
                parser
            ));
        }
    }

    // 确定输出路径
    let output_path = output.unwrap_or_else(|| {
        let mut path = input.clone();
        path.set_extension("md");
        path
    });

    // 轻量构造文档解析服务：CLI 本地解析不需要 OSS 上传/任务队列/任务持久化，
    // 因此不复用 AppState（AppState 会强制校验 OSS 凭证并启动后台 worker 池）
    let document_service = build_local_document_service(app_config)?;

    // 执行解析（引擎由文件格式自动选择：PDF→MinerU，其他→MarkItDown）
    let input_str = input.to_string_lossy().to_string();
    info!("Start document parsing");
    let parse_result = document_service
        .parse_document_local(&input_str)
        .await
        .with_context(|| format!("解析文件失败: {}", input.display()))?;

    // 自动检测选择的引擎与 --parser 请求不一致时，以检测结果为准并提示
    let engine_matches_request = matches!(
        (parser.as_str(), &parse_result.engine),
        ("mineru", ParserEngine::MinerU) | ("markitdown", ParserEngine::MarkItDown)
    );
    if !engine_matches_request {
        info!(
            "Requested parser is {parser}, but format detection selected {:?}; using the detected engine",
            parse_result.engine
        );
    }
    info!(
        "Parsing finished: engine={:?}, format={:?}, elapsed={}",
        parse_result.engine,
        parse_result.format,
        parse_result
            .processing_time
            .map(|t| format!("{t:.2}s"))
            .unwrap_or_else(|| "unknown".to_string())
    );

    // 图片资源 best-effort 处理：将解析输出目录中的图片拷贝到输出文件旁的
    // <stem>_assets/ 并改写 markdown 中的图片引用；失败仅 warn，保留原始内容
    let markdown_content = if let Some(output_dir) = parse_result.output_dir.as_deref() {
        match copy_image_assets_and_rewrite(
            &output_path,
            output_dir,
            &parse_result.markdown_content,
        )
        .await
        {
            Ok(rewritten) => rewritten,
            Err(e) => {
                warn!("Failed to process image assets, keeping original content: {e}");
                parse_result.markdown_content.clone()
            }
        }
    } else {
        parse_result.markdown_content.clone()
    };

    // 写出解析结果
    tokio::fs::write(&output_path, markdown_content.as_bytes())
        .await
        .with_context(|| format!("写入解析结果失败: {}", output_path.display()))?;

    info!("The analysis is completed and the results will be saved to: {output_path:?}");

    Ok(())
}

/// 为 CLI parse 命令构造轻量 DocumentService。
/// 与服务端 AppState 的区别：不创建/校验 OSS 客户端、不启动任务队列，
/// 因此本地解析无需 OSS 凭证。
fn build_local_document_service(app_config: &AppConfig) -> Result<DocumentService> {
    // 任务服务依赖本地 sled 数据库（复用服务端配置的存储路径）
    let db_path = app_config.storage.sled.path.clone();
    if let Some(parent) = Path::new(&db_path).parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).context("无法创建数据库目录")?;
    }
    let db = sled::open(&db_path).with_context(|| format!("无法打开数据库: {db_path}"))?;
    let task_service = Arc::new(TaskService::new(Arc::new(db)).context("无法创建任务服务")?);

    // 双引擎解析器：优先自动检测虚拟环境，失败回退配置（与 AppState 行为一致）
    let dual_parser = match DualEngineParser::with_auto_venv_detection() {
        Ok(parser) => parser,
        Err(e) => {
            warn!(
                "Automatic detection of virtual environment failed, fell back to configuration: {e}"
            );
            DualEngineParser::with_timeout(
                &app_config.mineru,
                &app_config.markitdown,
                app_config.document_parser.processing_timeout,
            )
        }
    };

    let markdown_processor =
        MarkdownProcessor::new(MarkdownProcessorConfig::with_global_config(), None);

    Ok(DocumentService::with_config(
        dual_parser,
        markdown_processor,
        task_service,
        None, // CLI 本地解析不使用 OSS
        DocumentServiceConfig::from_app_config(app_config),
    ))
}

/// Markdown 图片引用模式：![alt](dest)
static MARKDOWN_IMAGE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"!\[([^\]]*)\]\(([^)]+)\)").expect("静态正则编译失败"));

/// 需要作为资产拷贝的图片扩展名
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "tiff", "tif",
];

/// 将解析输出目录中的图片拷贝到输出文件旁的 `<stem>_assets/` 目录，
/// 并把 markdown 中的图片引用改写为该目录的相对路径，保证输出的 .md 自包含。
/// 返回改写后的 markdown 内容。
async fn copy_image_assets_and_rewrite(
    output_path: &Path,
    source_dir: &str,
    markdown_content: &str,
) -> Result<String> {
    let source_root = Path::new(source_dir);
    if !source_root.exists() {
        return Ok(markdown_content.to_string());
    }

    // 递归收集图片文件（含 images/ 等子目录）
    let mut images: Vec<PathBuf> = Vec::new();
    let mut stack = vec![source_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut rd = tokio::fs::read_dir(&dir)
            .await
            .with_context(|| format!("读取目录失败: {}", dir.display()))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .with_context(|| format!("遍历目录失败: {}", dir.display()))?
        {
            let path = entry.path();
            let meta = match tokio::fs::metadata(&path).await {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file()
                && path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
            {
                images.push(path);
            }
        }
    }

    if images.is_empty() {
        return Ok(markdown_content.to_string());
    }

    // 资产目录：输出文件同级的 <输出文件名主干>_assets/
    let assets_name = format!(
        "{}_assets",
        output_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
    );
    let assets_dir = output_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join(&assets_name);
    tokio::fs::create_dir_all(&assets_dir)
        .await
        .with_context(|| format!("创建资产目录失败: {}", assets_dir.display()))?;

    // 拷贝图片（同名冲突时追加数字后缀），记录 原文件名 → 新文件名 映射
    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut name_map: HashMap<String, String> = HashMap::new();
    for img in images {
        let Some(original_name) = img.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let mut target_name = original_name.to_string();
        let mut index = 1usize;
        while used_names.contains(&target_name) {
            target_name = format!(
                "{}_{}{}",
                Path::new(original_name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(original_name),
                index,
                img.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| format!(".{e}"))
                    .unwrap_or_default()
            );
            index += 1;
        }
        tokio::fs::copy(&img, assets_dir.join(&target_name))
            .await
            .with_context(|| format!("拷贝图片失败: {}", img.display()))?;
        used_names.insert(target_name.clone());
        // markdown 中按文件名匹配，同名文件保留首次映射
        name_map
            .entry(original_name.to_string())
            .or_insert(target_name);
    }

    // 改写 markdown 中的图片路径（按文件名匹配）
    let rewritten = MARKDOWN_IMAGE_RE
        .replace_all(markdown_content, |caps: &regex::Captures| {
            let dest = &caps[2];
            let file_name = Path::new(dest)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(dest);
            match name_map.get(file_name) {
                Some(new_name) => format!("![{}]({}/{})", &caps[1], assets_name, new_name),
                None => caps[0].to_string(),
            }
        })
        .to_string();

    info!(
        "Copied {} images to {}",
        name_map.len(),
        assets_dir.display()
    );
    Ok(rewritten)
}
