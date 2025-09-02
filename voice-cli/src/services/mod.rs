pub mod apalis_manager;
pub mod audio_file_manager;
pub mod audio_format_detector;
pub mod audio_processor;
pub mod metadata_extractor;
pub mod model_service;
pub mod transcription_engine;
pub mod tts_service;
pub mod tts_task_manager;

// 重新导出核心服务
pub use apalis_manager::{ApalisManager, LockFreeApalisManager, TranscriptionTask, StepContext, TaskStatusUpdate, init_global_apalis_manager, init_global_lock_free_apalis_manager, transcription_pipeline_worker};
pub use audio_file_manager::AudioFileManager;
pub use audio_format_detector::AudioFormatDetector;
pub use audio_processor::AudioProcessor;
pub use metadata_extractor::{MetadataExtractor, AudioVideoMetadata};
pub use model_service::ModelService;
pub use transcription_engine::TranscriptionEngine;
pub use tts_service::TtsService;
pub use tts_task_manager::{TtsTaskManager, TtsTaskStats};