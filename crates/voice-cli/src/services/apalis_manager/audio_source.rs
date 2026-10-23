//! 音频来源处理：URL 流式下载、真实格式检测重命名、任务文件路径回写。

use super::*;

/// 从URL下载音频文件
pub(super) async fn download_audio_from_url(
    url: &str,
    task_id: &str,
    storage_dir: &std::path::Path,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    info!(
        "[Task {}] Start downloading audio files from URL: {}",
        task_id, url
    );

    // 创建HTTP客户端
    let client = reqwest::Client::new();

    // 发送GET请求
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载URL失败: {} - {}", url, e))?;

    // 检查响应状态
    if !response.status().is_success() {
        return Err(format!("URL下载失败，HTTP状态: {}", response.status()).into());
    }

    // 获取内容类型并确定文件扩展名
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|ct| ct.to_str().ok())
        .unwrap_or("application/octet-stream");

    let extension = get_file_extension(content_type, url);

    // 检查是否为支持的媒体格式
    if !is_supported_media_format(content_type) {
        warn!(
            "[Task {}] Possibly unsupported media format [{}], extension [{}], subsequent processing may fail",
            task_id, content_type, extension
        );
    }

    // 创建目标文件路径
    let filename = format!("task_{}.{}", task_id, extension);
    let file_path = storage_dir.join(&filename);

    // 确保目录存在
    if let Some(parent) = file_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("创建目录失败: {} - {}", parent.display(), e))?;
    }

    // 流式下载文件
    let mut file = tokio::fs::File::create(&file_path)
        .await
        .map_err(|e| format!("创建文件失败: {} - {}", file_path.display(), e))?;

    let mut stream = response.bytes_stream();
    let mut total_bytes = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("下载数据失败: {}", e))?;

        file.write_all(&chunk)
            .await
            .map_err(|e| format!("写入文件失败: {} - {}", file_path.display(), e))?;

        total_bytes += chunk.len();
    }

    file.flush()
        .await
        .map_err(|e| format!("刷新文件失败: {} - {}", file_path.display(), e))?;

    info!(
        "[Task {}] Download completed: {} bytes -> {}",
        task_id,
        total_bytes,
        file_path.display()
    );

    Ok(file_path)
}

/// 检测音频文件格式并重命名为正确扩展名
pub(super) async fn detect_and_rename_audio_file(
    file_path: &PathBuf,
    task_id: &str,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    info!(
        "[Task {}] Detect audio file format: {:?}",
        task_id, file_path
    );

    // 使用 AudioFormatDetector 检测文件真实格式
    let format_result = AudioFormatDetector::detect_format_from_path(file_path)
        .map_err(|e| format!("检测文件格式失败: {}", e))?;

    let detected_extension = if let Some(format_type) = format_result {
        format_type.extension().to_lowercase()
    } else {
        // 如果无法检测格式，使用文件扩展名作为后备
        if let Some(extension) = file_path.extension().and_then(|ext| ext.to_str()) {
            extension.to_lowercase()
        } else {
            return Err(format!("无法检测文件格式且文件无扩展名: {:?}", file_path).into());
        }
    };
    info!(
        "[Task {}] File format detected: {}",
        task_id, detected_extension
    );

    // 获取当前文件扩展名
    let current_extension = file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    // 如果扩展名不匹配，重命名文件
    if current_extension != detected_extension {
        info!(
            "[Task {}] File extension mismatch: current {} -> detecting {}",
            task_id, current_extension, detected_extension
        );

        let parent_dir = file_path
            .parent()
            .ok_or_else(|| format!("无法获取文件父目录: {:?}", file_path))?;

        let new_filename = format!("task_{}.{}", task_id, detected_extension);
        let new_file_path = parent_dir.join(&new_filename);

        // 重命名文件
        tokio::fs::rename(file_path, &new_file_path)
            .await
            .map_err(|e| {
                format!(
                    "重命名文件失败: {} -> {}: {}",
                    file_path.display(),
                    new_file_path.display(),
                    e
                )
            })?;

        info!(
            "[Task {}] The file has been renamed: {} -> {}",
            task_id,
            file_path.display(),
            new_file_path.display()
        );

        Ok(new_file_path)
    } else {
        info!(
            "[Task {}] The file extension is correct: {}",
            task_id, current_extension
        );
        Ok(file_path.clone())
    }
}

/// 更新数据库中任务的文件路径
pub(super) async fn update_task_file_path_in_db(
    task_id: &str,
    file_path: &std::path::Path,
    ctx: &StepContext,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let file_path_str = file_path.to_string_lossy().to_string();

    sqlx::query("UPDATE task_info SET file_path = ?, updated_at = ? WHERE task_id = ?")
        .bind(&file_path_str)
        .bind(chrono::Utc::now().timestamp())
        .bind(task_id)
        .execute(&ctx.pool)
        .await
        .map_err(|e| format!("更新任务文件路径失败: {}", e))?;

    info!(
        "[Task {}] The database file path has been updated: {}",
        task_id, file_path_str
    );
    Ok(())
}
