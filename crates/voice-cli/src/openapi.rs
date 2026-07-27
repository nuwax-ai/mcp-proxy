use crate::models::{
    AsyncTaskResponse, CancelResponse, DeleteResponse, HealthResponse, ModelInfo, ModelsResponse,
    RetryResponse, Segment, TaskPriority, TaskStatsResponse, TaskStatus, TaskStatusResponse,
    TranscriptionResponse, TtsAsyncRequest, TtsSyncRequest, TtsTaskError, TtsTaskResponse,
    TtsTaskStatus,
};
use crate::server::handlers;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

/// OpenAPI specification for Voice CLI API
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Voice CLI API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Speech-to-text HTTP service with Whisper model support",
        license(
            name = "MIT",
        ),
        contact(
            name = "Voice CLI Support",
            email = "support@voice-cli.dev"
        )
    ),
    servers(
        (url = "http://localhost:8077", description = "Local development server"),
        (url = "https://api.voice-cli.dev", description = "Production server")
    ),
    paths(
        handlers::health_handler,
        handlers::models_list_handler,
        handlers::transcribe_handler,
        handlers::transcribe_from_url_handler,
        handlers::async_transcribe_handler,
        handlers::get_task_handler,
        handlers::cancel_task_handler,
        handlers::get_task_result_handler,
        handlers::delete_task_handler,
        handlers::retry_task_handler,
        handlers::get_tasks_stats_handler,
        handlers::tts_tasks_stats_handler,
        handlers::tts_sync_handler,
        handlers::tts_voices_handler,
        handlers::tts_async_handler,
        handlers::tts_task_status_handler,
        handlers::tts_task_audio_handler,
        crate::server::stt_stream::ws_transcribe_handler,
        crate::server::tts_stream::ws_tts_handler,
    ),
    components(
        schemas(
            TranscriptionResponse,
            Segment,
            HealthResponse,
            ModelsResponse,
            ModelInfo,
            AsyncTaskResponse,
            TaskStatusResponse,
            TaskStatus,
            TaskPriority,
            CancelResponse,
            DeleteResponse,
            RetryResponse,
            TaskStatsResponse,
            TtsSyncRequest,
            TtsAsyncRequest,
            TtsTaskResponse,
            TtsTaskStatus,
            TtsTaskError
        )
    ),
    tags(
        (name = "健康检查", description = "服务健康与状态"),
        (name = "模型管理", description = "Whisper 模型管理"),
        (name = "转录", description = "语音转文本（同步 /transcribe）"),
        (name = "异步转录", description = "异步转录任务管理"),
        (name = "TTS", description = "文本转语音（sherpa-onnx Kokoro）"),
        (name = "任务管理", description = "任务生命周期与监控"),
        (name = "流式转录", description = "STT 流式 WebSocket（LocalAgreement 2）"),
        (name = "流式 TTS", description = "TTS 流式 WebSocket（增量 PCM）")
    ),
    external_docs(
        url = "https://github.com/your-org/voice-cli",
        description = "Voice CLI GitHub Repository"
    )
)]
pub struct ApiDoc;

/// Create Swagger UI service
pub fn create_swagger_ui() -> SwaggerUi {
    SwaggerUi::new("/api/docs")
        .url("/api/docs/openapi.json", ApiDoc::openapi())
        .config(utoipa_swagger_ui::Config::new(["/api/docs/openapi.json"]))
}

/// Get OpenAPI JSON specification
pub fn get_openapi_json() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}
