pub mod apalis_manager;
pub mod audio_file_manager;
pub mod audio_format_detector;
pub mod metadata_extractor;
pub mod model_service;
pub mod tts_apalis_manager;

// 重新导出核心服务
pub use apalis_manager::{
    LockFreeApalisManager, StepContext, TaskStatusUpdate, TranscriptionTask,
    transcription_pipeline_worker,
};
pub use audio_file_manager::AudioFileManager;
pub use audio_format_detector::AudioFormatDetector;
pub use metadata_extractor::{AudioVideoMetadata, MetadataExtractor};
pub use model_service::ModelService;
pub use tts_apalis_manager::{TtsApalisManager, TtsStepContext, tts_pipeline_worker};
