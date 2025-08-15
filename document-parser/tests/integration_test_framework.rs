//! 集成测试框架
//! 
//! 提供完整的集成测试环境，包括：
//! - 测试环境设置和清理
//! - 模拟外部服务
//! - 端到端测试工具
//! - 性能基准测试

use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::net::TcpListener;
use axum::Router;
use tower::ServiceExt;
use tower_http::trace::TraceLayer;
use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path, header};
use serde_json::json;
use uuid::Uuid;

use document_parser::{
    config::{AppConfig, ServerConfig, LogConfig, DocumentParserConfig, FileSize, MinerUConfig, MarkItDownConfig, MarkItDownFeatures, GlobalFileSizeConfig},
    config::StorageConfig as ConfigStorageConfig,
    models::*,
    services::*,
    handlers::*,
    app_state::AppState,
    error::*,
};
use document_parser::config::{SledConfig, OssConfig};

/// 集成测试环境
pub struct IntegrationTestEnvironment {
    pub app_state: AppState,
    pub config: AppConfig,
    pub temp_dir: TempDir,
    pub mock_server: MockServer,
    pub test_server_addr: String,
}

impl IntegrationTestEnvironment {
    /// 创建新的测试环境
    pub async fn new() -> anyhow::Result<Self> {
        // 创建临时目录
        let temp_dir = TempDir::new()?;
        
        // 启动模拟服务器
        let mock_server = MockServer::start().await;
        
        // 创建测试配置
        let config = Self::create_test_config(&temp_dir, &mock_server).await?;
        
        // 创建应用状态
        let app_state = Self::create_app_state(&config).await?;
        
        // 获取可用端口
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let test_server_addr = listener.local_addr()?.to_string();
        drop(listener);
        
        Ok(Self {
            app_state,
            config,
            temp_dir,
            mock_server,
            test_server_addr,
        })
    }
    
    /// 创建测试配置
    async fn create_test_config(temp_dir: &TempDir, mock_server: &MockServer) -> anyhow::Result<AppConfig> {
        Ok(AppConfig {
            environment: "test".to_string(),
            server: ServerConfig {
                port: 0, // 将在运行时分配
                host: "127.0.0.1".to_string(),
            },
            log: LogConfig {
                level: "debug".to_string(),
                path: temp_dir.path().join("test.log").to_string_lossy().to_string(),
            },
            document_parser: DocumentParserConfig {
                max_concurrent: 5,
                queue_size: 50,
                download_timeout: 30,
                processing_timeout: 300,
            },
            file_size_config: GlobalFileSizeConfig::new(),
            storage: ConfigStorageConfig {
                sled: SledConfig {
                    path: temp_dir.path().join("test_sled.db").to_string_lossy().to_string(),
                    cache_capacity: 1024 * 1024,
                },
                oss: OssConfig {
                    endpoint: mock_server.uri(),
                    bucket: "test-bucket".to_string(),
                    access_key_id: "test-key".to_string(),
                    access_key_secret: "test-secret".to_string(),
                },
            },
            mineru: MinerUConfig {
                backend: "pipeline".to_string(),

                python_path: "python3".to_string(),
                max_concurrent: 2,
                queue_size: 10,
                timeout: 300,
                enable_gpu: false,
                batch_size: 1,
                quality_level: document_parser::config::QualityLevel::Balanced,
            },
            markitdown: MarkItDownConfig {
                python_path: "python3".to_string(),

                timeout: 180,
                enable_plugins: false,
                features: MarkItDownFeatures {
                    ocr: true,
                    audio_transcription: true,
                    azure_doc_intel: false,
                    youtube_transcription: false,
                },
            },
            external_integration: document_parser::config::ExternalIntegrationConfig {
                webhook_url: mock_server.uri(),
                api_key: "test-api-key".to_string(),
                timeout: 30,
            },
        })
    }
    
    /// 创建应用状态
    async fn create_app_state(config: &AppConfig) -> anyhow::Result<AppState> {
        AppState::new(config.clone()).await
            .map_err(|e| anyhow::anyhow!("Failed to create app state: {}", e))
    }
    
    /// 创建测试应用路由
    pub fn create_test_app(&self) -> Router {
        document_parser::routes::create_routes(self.app_state.clone())
            .layer(TraceLayer::new_for_http())
    }
    
    /// 设置OSS模拟响应
    pub async fn setup_oss_mocks(&self) {
        // 模拟文件上传成功
        Mock::given(method("PUT"))
            .and(path("/test-bucket/test-documents"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_string("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<PutObjectResult></PutObjectResult>"))
            .mount(&self.mock_server)
            .await;
        
        // 模拟文件下载成功
        Mock::given(method("GET"))
            .and(path("/test-bucket/test-documents"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_bytes(b"Mock file content".to_vec()))
            .mount(&self.mock_server)
            .await;
        
        // 模拟文件删除成功
        Mock::given(method("DELETE"))
            .and(path("/test-bucket/test-documents"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&self.mock_server)
            .await;
    }
    
    /// 设置Python解析器模拟脚本
    pub async fn setup_parser_mocks(&self) -> anyhow::Result<()> {
        // 创建模拟脚本目录
        let scripts_dir = self.temp_dir.path().join("scripts");
        tokio::fs::create_dir_all(&scripts_dir).await?;
        
        // 创建模拟的MinerU解析器脚本
        let mineru_script = r#"#!/usr/bin/env python3
import sys
import json

def main():
    if len(sys.argv) < 2:
        print(json.dumps({"error": "No input file provided"}), file=sys.stderr)
        sys.exit(1)
    
    input_file = sys.argv[1]
    result = {
        "markdown_content": "Mock Document Content",
        "extracted_images": [],
        "metadata": {
            "parser": "mineru_mock",
            "file_path": input_file
        }
    }
    print(json.dumps(result))

if __name__ == "__main__":
    main()
"#;
        
        let mineru_script_path = scripts_dir.join("mineru_mock.py");
        tokio::fs::write(&mineru_script_path, mineru_script).await?;
        
        // 创建模拟的MarkItDown解析器脚本
        let markitdown_script = r#"#!/usr/bin/env python3
import sys
import json

def main():
    if len(sys.argv) < 2:
        print(json.dumps({"error": "No input file provided"}), file=sys.stderr)
        sys.exit(1)
    
    input_file = sys.argv[1]
    result = {
        "markdown_content": "Mock Document Content",
        "extracted_images": [],
        "metadata": {
            "parser": "markitdown_mock",
            "file_path": input_file
        }
    }
    print(json.dumps(result))

if __name__ == "__main__":
    main()
"#;
        
        let markitdown_script_path = scripts_dir.join("markitdown_mock.py");
        tokio::fs::write(&markitdown_script_path, markitdown_script).await?;
        
        // 设置执行权限
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = tokio::fs::metadata(&mineru_script_path).await?.permissions();
            perms.set_mode(0o755);
            tokio::fs::set_permissions(&mineru_script_path, perms).await?;
            
            let mut perms = tokio::fs::metadata(&markitdown_script_path).await?.permissions();
            perms.set_mode(0o755);
            tokio::fs::set_permissions(&markitdown_script_path, perms).await?;
        }
        
        Ok(())
    }
    
    /// 创建测试文件
    pub async fn create_test_file(&self, filename: &str, content: &[u8]) -> anyhow::Result<String> {
        let file_path = self.temp_dir.path().join(filename);
        tokio::fs::write(&file_path, content).await?;
        Ok(file_path.to_string_lossy().to_string())
    }
    
    /// 清理测试环境
    pub async fn cleanup(&self) {
        // 清理数据库
        if let Err(e) = self.app_state.task_service.cleanup_expired_tasks().await {
            eprintln!("Failed to cleanup expired tasks: {}", e);
        }
        
        // 清理临时文件将在Drop时自动处理
    }
}

/// 集成测试工具
pub struct IntegrationTestTools;

impl IntegrationTestTools {
    /// 等待任务完成
    pub async fn wait_for_task_completion(
        task_service: &TaskService,
        task_id: &str,
        timeout: Duration,
    ) -> anyhow::Result<DocumentTask> {
        let start = std::time::Instant::now();
        
        loop {
            if start.elapsed() > timeout {
                return Err(anyhow::anyhow!("Task completion timeout"));
            }
            
            if let Some(task) = task_service.get_task(task_id).await? {
                match &task.status {
                    TaskStatus::Completed { .. } => return Ok(task),
                    TaskStatus::Failed { .. } => {
                        return Err(anyhow::anyhow!("Task failed: {:?}", task.status));
                    }
                    _ => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                }
            } else {
                return Err(anyhow::anyhow!("Task not found"));
            }
        }
    }
    
    /// 验证任务状态转换
    pub fn validate_task_status_transition(
        from: &TaskStatus,
        to: &TaskStatus,
    ) -> bool {
        use TaskStatus::*;
        
        use TaskStatus::*;
        match (from, to) {
            (_Pending, Processing { .. }) => true,
            (Processing { .. }, Completed { .. }) => true,
            (Processing { .. }, Failed { .. }) => true,
            (_Pending, Failed { .. }) => true,
            _ => false,
        }
    }
    
    /// 创建测试HTTP客户端
    pub fn create_test_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client")
    }
    
    /// 验证API响应格式
    pub fn validate_api_response<T>(
        response: &reqwest::Response,
        expected_status: reqwest::StatusCode,
    ) -> bool {
        response.status() == expected_status
    }
    
    /// 生成测试数据
    pub fn generate_test_pdf_content() -> Vec<u8> {
        // 简单的PDF文件头（用于测试）
        b"%PDF-1.4\n1 0 obj\n<<\n/Type /Catalog\n/Pages 2 0 R\n>>\nendobj\n2 0 obj\n<<\n/Type /Pages\n/Kids [3 0 R]\n/Count 1\n>>\nendobj\n3 0 obj\n<<\n/Type /Page\n/Parent 2 0 R\n/MediaBox [0 0 612 792]\n>>\nendobj\nxref\n0 4\n0000000000 65535 f \n0000000009 00000 n \n0000000074 00000 n \n0000000120 00000 n \ntrailer\n<<\n/Size 4\n/Root 1 0 R\n>>\nstartxref\n179\n%%EOF".to_vec()
    }
    
    pub fn generate_test_docx_content() -> Vec<u8> {
        // 简单的DOCX文件头（用于测试）
        b"PK\x03\x04\x14\x00\x00\x00\x08\x00".to_vec()
    }
    
    pub fn generate_test_image_content() -> Vec<u8> {
        // 简单的PNG文件头（用于测试）
        b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90wS\xde\x00\x00\x00\tpHYs\x00\x00\x0b\x13\x00\x00\x0b\x13\x01\x00\x9a\x9c\x18\x00\x00\x00\x0cIDATx\x9cc```\x00\x00\x00\x04\x00\x01\xdd\x8d\xb4\x1c\x00\x00\x00\x00IEND\xaeB`\x82".to_vec()
    }
}

/// 性能基准测试工具
pub struct PerformanceBenchmark {
    start_time: std::time::Instant,
    checkpoints: Vec<(String, std::time::Duration)>,
}

impl PerformanceBenchmark {
    pub fn new() -> Self {
        Self {
            start_time: std::time::Instant::now(),
            checkpoints: Vec::new(),
        }
    }
    
    pub fn checkpoint(&mut self, name: &str) {
        let elapsed = self.start_time.elapsed();
        self.checkpoints.push((name.to_string(), elapsed));
    }
    
    pub fn report(&self) -> String {
        let mut report = String::new();
        report.push_str("Performance Benchmark Report:\n");
        
        for (i, (name, duration)) in self.checkpoints.iter().enumerate() {
            if i == 0 {
                report.push_str(&format!("  {}: {:?}\n", name, duration));
            } else {
                let prev_duration = self.checkpoints[i - 1].1;
                let diff = *duration - prev_duration;
                report.push_str(&format!("  {}: {:?} (+{:?})\n", name, duration, diff));
            }
        }
        
        report
    }
}

/// 并发测试工具
pub struct ConcurrencyTestTools;

impl ConcurrencyTestTools {
    /// 并发执行任务并收集结果
    pub async fn run_concurrent_tasks<F, Fut, T>(
        tasks: Vec<F>,
        max_concurrent: usize,
    ) -> Vec<anyhow::Result<T>>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = anyhow::Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        use futures::stream::{FuturesUnordered, StreamExt};
        
        let mut results = Vec::new();
        let mut futures = FuturesUnordered::new();
        let mut task_iter = tasks.into_iter();
        
        // 启动初始任务
        for _ in 0..max_concurrent.min(task_iter.len()) {
            if let Some(task) = task_iter.next() {
                futures.push(tokio::spawn(task()));
            }
        }
        
        // 处理完成的任务并启动新任务
        while let Some(result) = futures.next().await {
            match result {
                Ok(task_result) => results.push(task_result),
                Err(e) => results.push(Err(anyhow::anyhow!("Task panicked: {}", e))),
            }
            
            // 启动下一个任务
            if let Some(task) = task_iter.next() {
                futures.push(tokio::spawn(task()));
            }
        }
        
        results
    }
    
    /// 测试竞态条件
    pub async fn test_race_condition<F, Fut>(
        task_factory: F,
        num_tasks: usize,
    ) -> Vec<anyhow::Result<()>>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let task_factory = Arc::new(task_factory);
        let mut handles = Vec::new();
        
        for _ in 0..num_tasks {
            let factory = task_factory.clone();
            let handle = tokio::spawn(async move {
                factory().await
            });
            handles.push(handle);
        }
        
        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => results.push(Err(anyhow::anyhow!("Task panicked: {}", e))),
            }
        }
        
        results
    }
}

#[cfg(test)]
mod framework_tests {
    use super::*;
    
    #[tokio::test]
    async fn test_integration_environment_setup() {
        let env = IntegrationTestEnvironment::new().await
            .expect("Failed to create test environment");
        
        // 验证环境设置
        assert!(env.temp_dir.path().exists());
        assert!(!env.test_server_addr.is_empty());
        assert!(!env.mock_server.uri().is_empty());
        
        // 验证配置
        assert_eq!(env.config.server.host, "127.0.0.1");
        assert!(env.config.document_parser.max_concurrent > 0);
        
        env.cleanup().await;
    }
    
    #[tokio::test]
    async fn test_performance_benchmark() {
        let mut benchmark = PerformanceBenchmark::new();
        
        tokio::time::sleep(Duration::from_millis(10)).await;
        benchmark.checkpoint("First checkpoint");
        
        tokio::time::sleep(Duration::from_millis(20)).await;
        benchmark.checkpoint("Second checkpoint");
        
        let report = benchmark.report();
        assert!(report.contains("Performance Benchmark Report"));
        assert!(report.contains("First checkpoint"));
        assert!(report.contains("Second checkpoint"));
    }
    
    #[tokio::test]
    async fn test_concurrency_tools() {
        let tasks: Vec<Box<dyn FnOnce() -> _ + Send>> = (0..5)
            .map(|i| {
                Box::new(move || async move {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    Ok::<i32, anyhow::Error>(i)
                }) as Box<dyn FnOnce() -> _ + Send>
            })
            .collect();
        
        let results = ConcurrencyTestTools::run_concurrent_tasks(tasks, 3).await;
        
        assert_eq!(results.len(), 5);
        for result in results {
            assert!(result.is_ok());
        }
    }
}