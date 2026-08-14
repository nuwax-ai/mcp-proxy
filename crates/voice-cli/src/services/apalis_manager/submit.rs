//! 任务提交：文件上传任务与 URL 下载任务入队 + 初始状态落库。

use super::*;

impl LockFreeApalisManager {
    /// 提交任务
    #[allow(clippy::too_many_arguments)]
    pub async fn submit_task(
        &self,
        storage: &mut SqliteStorage<TranscriptionTask>,
        audio_file_path: PathBuf,
        original_filename: String,
        model: Option<String>,
        response_format: Option<String>,
        language: Option<String>,
        initial_prompt: Option<String>,
    ) -> Result<String, VoiceCliError> {
        info!("submit_task: Start creating task...");
        let task = AsyncTranscriptionTask::new(
            self.generate_task_id(),
            audio_file_path.clone(),
            original_filename.clone(),
            model.clone(),
            response_format.clone(),
            language.clone(),
            initial_prompt.clone(),
        );

        info!("submit_task: Task creation completed: {}", task.task_id);
        let apalis_task: TranscriptionTask = task.clone().into();

        info!("submit_task: Start pushing tasks to the queue...");
        info!("submit_task: task data: {:?}", apalis_task);

        // Use the storage directly without cloning
        info!("submit_task: Prepare to call storage.push()...");
        let push_result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            storage.push(apalis_task.clone()),
        )
        .await;
        info!("submit_task: storage.push() call completed");

        match push_result {
            Ok(Ok(_)) => {
                info!("submit_task: Task pushed successfully");
            }
            Ok(Err(e)) => {
                info!("submit_task: Task push failed: {}", e);
                return Err(VoiceCliError::Storage(format!("提交任务失败: {}", e)));
            }
            Err(_) => {
                info!("submit_task: task push timeout");
                return Err(VoiceCliError::Storage("推送任务到队列超时".to_string()));
            }
        };

        info!("Task pushed to Apalis storage: {:?}", apalis_task.task_id);

        // 初始状态
        info!("submit_task: Save the initial task status...");
        let initial_status = TaskStatus::Pending {
            queued_at: Utc::now(),
        };

        // 使用新的保存任务信息方法，包含文件路径
        self.save_task_info(SaveTaskInfoParams {
            task_id: &task.task_id,
            status: &initial_status,
            file_path: Some(&audio_file_path),
            original_filename: Some(&original_filename),
            model: model.as_deref(),
            response_format: response_format.as_deref(),
            retry_count: 0,
            error_message: None,
        })
        .await?;

        info!("Task submitted successfully: {}", task.task_id);
        Ok(task.task_id)
    }

    /// 提交URL转录任务
    #[allow(clippy::too_many_arguments)]
    pub async fn submit_task_for_url(
        &self,
        storage: &mut SqliteStorage<TranscriptionTask>,
        url: String,
        filename: String,
        model: Option<String>,
        response_format: Option<String>,
        language: Option<String>,
        initial_prompt: Option<String>,
    ) -> Result<String, VoiceCliError> {
        info!("submit_task_for_url: Start creating URL task...");

        // 生成任务ID
        let task_id = self.generate_task_id();

        // 创建临时文件路径（实际下载将在worker中执行）
        let temp_audio_path = PathBuf::from(format!("./data/audio/temp_{}.pending", task_id));

        // 创建任务对象
        let task = AsyncTranscriptionTask::new(
            task_id.clone(),
            temp_audio_path.clone(),
            filename.clone(),
            model.clone(),
            response_format.clone(),
            language.clone(),
            initial_prompt.clone(),
        );

        info!(
            "submit_task_for_url: Task creation completed: {}",
            task.task_id
        );

        // 转换为Apalis任务，设置URL任务类型
        let apalis_task = TranscriptionTask {
            task_id: task.task_id.clone(),
            audio_file_path: temp_audio_path,
            original_filename: filename.clone(),
            model: task.model,
            response_format: task.response_format,
            language: task.language,
            initial_prompt: task.initial_prompt,
            created_at: task.created_at,
            task_type: TaskType::UrlDownload,
            url: Some(url),
        };

        info!("submit_task_for_url: Start pushing the URL task to the queue...");
        info!("submit_task_for_url: task data: {:?}", apalis_task);

        // 推送任务到队列
        let push_result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            storage.push(apalis_task.clone()),
        )
        .await;
        info!("submit_task_for_url: storage.push() call completed");

        match push_result {
            Ok(Ok(_)) => {
                info!("submit_task_for_url: Task pushed successfully");
            }
            Ok(Err(e)) => {
                info!("submit_task_for_url: Task push failed: {}", e);
                return Err(VoiceCliError::Storage(format!("提交URL任务失败: {}", e)));
            }
            Err(_) => {
                info!("submit_task_for_url: task push timeout");
                return Err(VoiceCliError::Storage("推送URL任务到队列超时".to_string()));
            }
        };

        info!(
            "URL task has been pushed to Apalis storage: {:?}",
            apalis_task.task_id
        );

        // 初始状态
        info!("submit_task_for_url: Save initial task status...");
        let initial_status = TaskStatus::Pending {
            queued_at: Utc::now(),
        };

        // 保存任务信息，包含URL
        self.save_task_info(SaveTaskInfoParams {
            task_id: &task.task_id,
            status: &initial_status,
            file_path: None, // 文件路径将在下载后设置
            original_filename: Some(&filename),
            model: model.as_deref(),
            response_format: response_format.as_deref(),
            retry_count: 0,
            error_message: None,
        })
        .await?;

        info!("URL task submitted successfully: {}", task.task_id);
        Ok(task.task_id)
    }
}
