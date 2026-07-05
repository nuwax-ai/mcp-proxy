pub mod apalis_manager;
pub mod audio_file_manager;
pub mod audio_format_detector;
pub mod audio_processor;
pub mod metadata_extractor;
pub mod model_service;

// 重新导出核心服务
pub use apalis_manager::{
    ApalisManager, LockFreeApalisManager, StepContext, TaskStatusUpdate, TranscriptionTask,
    init_global_apalis_manager, init_global_lock_free_apalis_manager,
    transcription_pipeline_worker,
};
pub use audio_file_manager::AudioFileManager;
pub use audio_format_detector::AudioFormatDetector;
pub use audio_processor::AudioProcessor;
pub use metadata_extractor::{AudioVideoMetadata, MetadataExtractor};
pub use model_service::ModelService;
