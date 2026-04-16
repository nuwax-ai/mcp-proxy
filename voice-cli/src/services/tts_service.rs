use crate::VoiceCliError;
use crate::models::{TtsAsyncRequest, TtsSyncRequest, TtsTaskResponse};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::NamedTempFile;
use tracing::{debug, error, info};
use uuid::Uuid;

/// TTS服务 - 处理文本到语音转换
#[derive(Debug)]
pub struct TtsService {
    python_path: PathBuf,
    script_path: PathBuf,
    model_path: Option<PathBuf>,
}

impl TtsService {
    /// 创建新的TTS服务实例
    pub fn new(
        python_path: Option<PathBuf>,
        model_path: Option<PathBuf>,
    ) -> Result<Self, VoiceCliError> {
        let python_path = python_path.unwrap_or_else(|| {
            // 尝试在多个位置查找虚拟环境中的 Python
            let possible_venv_paths = vec![
                // 当前目录下的虚拟环境
                if cfg!(windows) {
                    PathBuf::from(".venv/Scripts/python.exe")
                } else {
                    PathBuf::from(".venv/bin/python")
                },
                // crate 目录下的虚拟环境
                if cfg!(windows) {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/Scripts/python.exe")
                } else {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python")
                },
            ];

            // 查找第一个存在的 Python 解释器
            for venv_python in possible_venv_paths {
                if venv_python.exists() {
                    return venv_python;
                }
            }

            // 回退到系统 Python
            if let Ok(_output) = Command::new("python3").arg("--version").output() {
                PathBuf::from("python3")
            } else if let Ok(_output) = Command::new("python").arg("--version").output() {
                PathBuf::from("python")
            } else {
                PathBuf::from("python3") // 默认使用python3
            }
        });

        // 获取脚本路径（首先尝试当前目录，然后尝试 crate 目录）
        let current_dir = std::env::current_dir()
            .map_err(|e| VoiceCliError::Config(format!("获取当前目录失败: {}", e)))?;

        let script_path = current_dir.join("tts_service.py");

        let final_script_path = if script_path.exists() {
            script_path
        } else {
            // 尝试在 crate 目录中查找
            let crate_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let crate_script_path = crate_path.join("tts_service.py");
            if crate_script_path.exists() {
                crate_script_path
            } else {
                return Err(VoiceCliError::Config(format!(
                    "TTS脚本不存在: 在 {:?} 或 {:?} 中都未找到",
                    script_path, crate_script_path
                )));
            }
        };

        info!("Use TTS script: {:?}", final_script_path);

        info!(
            "Initialize TTS service - Python: {:?}, script: {:?}",
            python_path, final_script_path
        );

        Ok(Self {
            python_path,
            script_path: final_script_path,
            model_path,
        })
    }

    /// 同步TTS合成
    pub async fn synthesize_sync(&self, request: TtsSyncRequest) -> Result<PathBuf, VoiceCliError> {
        let start_time = std::time::Instant::now();

        // 验证输入
        if request.text.trim().is_empty() {
            return Err(VoiceCliError::InvalidInput("文本不能为空".to_string()));
        }

        if let Some(speed) = request.speed {
            if !(0.5..=2.0).contains(&speed) {
                return Err(VoiceCliError::InvalidInput(
                    "语速必须在0.5-2.0之间".to_string(),
                ));
            }
        }

        if let Some(pitch) = request.pitch {
            if !(-20..=20).contains(&pitch) {
                return Err(VoiceCliError::InvalidInput(
                    "音调必须在-20到20之间".to_string(),
                ));
            }
        }

        if let Some(volume) = request.volume {
            if !(0.5..=2.0).contains(&volume) {
                return Err(VoiceCliError::InvalidInput(
                    "音量必须在0.5-2.0之间".to_string(),
                ));
            }
        }

        // 创建临时输出文件
        let output_format = request.format.as_deref().unwrap_or("mp3");
        let temp_file = NamedTempFile::new()
            .map_err(|e| VoiceCliError::Io(format!("创建临时文件失败: {}", e)))?;

        let output_path = temp_file.into_temp_path();
        let output_path_str = output_path
            .to_str()
            .ok_or_else(|| VoiceCliError::Io("临时文件路径无效".to_string()))?;

        info!(
            "Start TTS synthesis - text length: {}, format: {}",
            request.text.len(),
            output_format
        );

        // 使用 uv run 来确保在正确的虚拟环境中运行
        let mut cmd = Command::new("uv");
        cmd.arg("run")
            .arg(&self.script_path)
            .arg(&request.text)
            .arg("--output")
            .arg(output_path_str)
            .arg("--speed")
            .arg(request.speed.unwrap_or(1.0).to_string())
            .arg("--pitch")
            .arg(request.pitch.unwrap_or(0).to_string())
            .arg("--volume")
            .arg(request.volume.unwrap_or(1.0).to_string())
            .arg("--format")
            .arg(output_format)
            // 设置工作目录为脚本所在的目录
            .current_dir(
                self.script_path
                    .parent()
                    .unwrap_or(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
            );

        // 添加模型参数
        if let Some(model) = &request.model {
            cmd.arg("--model").arg(model);
        }

        if let Some(ref model_path) = self.model_path {
            cmd.env("TTS_MODEL_PATH", model_path.to_string_lossy().as_ref());
        }
        cmd.env(
            "TTS_PYTHON_PATH",
            self.python_path.to_string_lossy().as_ref(),
        );

        debug!("Execute TTS command: {:?}", cmd);

        // 执行命令
        let output = cmd
            .output()
            .map_err(|e| VoiceCliError::TtsError(format!("执行TTS命令失败: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            error!(
                "TTS synthesis failed - stderr: {}, stdout: {}",
                stderr, stdout
            );
            return Err(VoiceCliError::TtsError(format!("TTS合成失败: {}", stderr)));
        }

        // 验证输出文件
        if !output_path.exists() {
            return Err(VoiceCliError::TtsError(
                "TTS合成失败：输出文件未创建".to_string(),
            ));
        }

        let file_size = output_path.metadata().map(|m| m.len()).unwrap_or(0);

        if file_size == 0 {
            return Err(VoiceCliError::TtsError(
                "TTS合成失败：输出文件为空".to_string(),
            ));
        }

        let processing_time = start_time.elapsed();
        info!(
            "TTS synthesis completed - file size: {} bytes, time taken: {:?}",
            file_size, processing_time
        );

        // 将临时文件持久化到正式位置
        let final_output_path = self
            .persist_output_file(&output_path, output_format)
            .await?;

        Ok(final_output_path)
    }

    /// 创建异步TTS任务
    pub async fn create_async_task(
        &self,
        request: TtsAsyncRequest,
    ) -> Result<TtsTaskResponse, VoiceCliError> {
        // 验证输入
        if request.text.trim().is_empty() {
            return Err(VoiceCliError::InvalidInput("文本不能为空".to_string()));
        }

        // 预估处理时间（基于文本长度）
        let estimated_duration = self.estimate_processing_time(&request.text);

        info!(
            "Create a TTS asynchronous task - text length: {}, estimated time: {}s",
            request.text.len(),
            estimated_duration
        );

        // TODO: 将任务提交到TTS任务管理器
        // 这里暂时返回模拟的任务ID，实际实现需要集成TtsTaskManager
        let task_id = Uuid::new_v4().to_string();

        Ok(TtsTaskResponse {
            task_id: task_id.clone(),
            message: "TTS任务已提交".to_string(),
            estimated_duration: Some(estimated_duration),
        })
    }

    /// 预估处理时间
    fn estimate_processing_time(&self, text: &str) -> u32 {
        // 简单的预估：基于文本长度
        // 假设每秒处理10个字符
        let chars_per_second = 10;
        let estimated_seconds = (text.len() as f32 / chars_per_second as f32).ceil() as u32;

        // 最少3秒，最多300秒（5分钟）
        estimated_seconds.max(3).min(300)
    }

    /// 持久化输出文件
    async fn persist_output_file(
        &self,
        temp_path: &Path,
        format: &str,
    ) -> Result<PathBuf, VoiceCliError> {
        // 创建输出目录
        let output_dir = PathBuf::from("./data/tts");
        tokio::fs::create_dir_all(&output_dir)
            .await
            .map_err(|e| VoiceCliError::Io(format!("创建输出目录失败: {}", e)))?;

        // 生成唯一文件名
        let filename = format!("tts_{}.{}", Uuid::new_v4(), format);
        let final_path = output_dir.join(filename);

        // 复制文件
        tokio::fs::copy(temp_path, &final_path)
            .await
            .map_err(|e| VoiceCliError::Io(format!("复制文件失败: {}", e)))?;

        Ok(final_path)
    }

    /// 清理资源
    pub async fn cleanup(&self) -> Result<(), VoiceCliError> {
        // 清理临时文件等
        info!("TTS service cleanup completed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_processing_time() {
        let service = TtsService::new(None, None).unwrap();

        // 测试短文本
        let short_time = service.estimate_processing_time("Hello");
        assert!(short_time >= 3);

        // 测试长文本
        let long_text = "A".repeat(1000);
        let long_time = service.estimate_processing_time(&long_text);
        assert!(long_time > 50);

        // 测试最大限制
        let very_long_text = "A".repeat(10000);
        let max_time = service.estimate_processing_time(&very_long_text);
        assert_eq!(max_time, 300);
    }

    #[tokio::test]
    async fn test_create_async_task() {
        let service = TtsService::new(None, None).unwrap();

        let request = TtsAsyncRequest {
            text: "Hello, world!".to_string(),
            model: None,
            speed: Some(1.0),
            pitch: Some(0),
            volume: Some(1.0),
            format: Some("mp3".to_string()),
            priority: None,
        };

        let response = service.create_async_task(request).await.unwrap();

        assert!(!response.task_id.is_empty());
        assert_eq!(response.message, "TTS任务已提交");
        assert!(response.estimated_duration.unwrap() >= 3);
    }
}
