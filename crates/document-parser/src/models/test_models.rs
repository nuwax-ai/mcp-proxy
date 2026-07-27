use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 测试 MinerU 后续处理的请求模型
#[derive(Debug, Deserialize, ToSchema)]
pub struct TestPostMineruRequest {
    /// 任务ID
    pub task_id: String,
}

/// 测试 MinerU 后续处理的响应模型
#[derive(Debug, Serialize, ToSchema)]
pub struct TestPostMineruResponse {
    /// 任务ID
    pub task_id: String,
    /// 响应消息
    pub message: String,
    /// MinerU 输出路径
    pub mineru_output_path: String,
    /// Markdown 文件名
    pub markdown_file: String,
    /// 图片数量
    pub images_count: usize,
    /// 是否开始后续处理
    pub processing_started: bool,
}

impl TestPostMineruResponse {
    /// 创建成功响应
    pub fn success(
        task_id: String,
        mineru_output_path: String,
        markdown_file: String,
        images_count: usize,
    ) -> Self {
        Self {
            task_id,
            message: "模拟 MinerU 解析完成，开始后续处理".to_string(),
            mineru_output_path,
            markdown_file,
            images_count,
            processing_started: true,
        }
    }
}
