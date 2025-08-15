// 服务模块
// TODO: 实现具体的服务
pub mod document_service;
pub mod task_service;
pub mod oss_service;
pub mod image_processor;
pub mod task_queue_service;
pub mod storage_service;

pub use document_service::{DocumentService, DocumentServiceConfig};
pub use task_service::{TaskService, TaskStats};
pub use oss_service::OssService;
pub use image_processor::{ImageProcessor, ImageProcessResult, ImageProcessConfig, ImageProcessingStats};
pub use task_queue_service::*;
pub use storage_service::*;
