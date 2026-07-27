// 服务模块
// TODO: 实现具体的服务
pub mod document_service;
pub mod document_task_processor;
pub mod image_processor;
pub mod oss_service;
pub mod storage_service;
pub mod task_queue_service;
pub mod task_service;

pub use document_service::{DocumentService, DocumentServiceConfig};
pub use document_task_processor::DocumentTaskProcessor;
pub use image_processor::{ImageProcessor, ImageProcessorConfig};
pub use oss_service::OssService;
pub use storage_service::*;
pub use task_queue_service::*;
pub use task_service::{TaskService, TaskStats};
