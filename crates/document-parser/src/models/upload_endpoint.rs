use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 已解析的自定义文件上传端点（nuwax 风格 REST API）
///
/// 持久化到 [`crate::models::DocumentTask::upload_config`]：异步链路的上传动作
/// 发生在后台 worker（队列只传 task_id），配置必须随任务自包含，重试/重启后
/// 仍然指向同一后端，不受全局配置后续变化影响。
///
/// 任务产物以 [`crate::models::OssData`] 存储，`storage_type = Some("custom")`
/// 判别（数据自描述），消费方按该字段分支。
///
/// 安全说明：`api_key` 以明文存入 sled 任务记录（内网部署已接受的取舍），
/// 并会随 `GET /api/v1/tasks` 的任务序列化原样回显。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UploadEndpoint {
    /// 服务基地址（如 `https://agent.example.com`，已去尾 `/`）
    pub base_url: String,
    /// 上传接口路径（如 `/api/v1/file/upload`）
    pub path: String,
    /// API Key（Bearer）
    pub api_key: String,
    /// 上传存储类型："store"（默认，永久）或 "tmp"（临时，时效由服务端自管）
    #[schema(value_type = String, example = "store")]
    pub upload_type: oss_client::CustomUploadType,
}

impl UploadEndpoint {
    /// 转换为 oss-client 的上传配置
    pub fn to_api_config(&self) -> oss_client::ApiUploadConfig {
        oss_client::ApiUploadConfig {
            base_url: self.base_url.clone(),
            path: self.path.clone(),
            api_key: self.api_key.clone(),
            upload_type: self.upload_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serde_round_trip() {
        let endpoint = UploadEndpoint {
            base_url: "https://agent.example.com".to_string(),
            path: "/api/v1/file/upload".to_string(),
            api_key: "ak-xxx".to_string(),
            upload_type: oss_client::CustomUploadType::Store,
        };
        let json = serde_json::to_string(&endpoint).unwrap();
        assert!(json.contains("\"upload_type\":\"store\""));
        let parsed: UploadEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.base_url, endpoint.base_url);
        assert_eq!(parsed.upload_type, oss_client::CustomUploadType::Store);
    }

    #[test]
    fn test_serde_tmp_variant() {
        let json = r#"{"base_url":"https://x.com","path":"/p","api_key":"k","upload_type":"tmp"}"#;
        let parsed: UploadEndpoint = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.upload_type, oss_client::CustomUploadType::Tmp);
    }
}
