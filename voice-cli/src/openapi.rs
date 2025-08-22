use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;
use crate::models::{
    TranscriptionResponse, Segment, HealthResponse, ModelsResponse, ModelInfo
};
use crate::server::handlers;

/// OpenAPI specification for Voice CLI API
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Voice CLI API",
        version = "0.1.0",
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
        (url = "http://localhost:8080", description = "Local development server"),
        (url = "https://api.voice-cli.dev", description = "Production server")
    ),
    paths(
        handlers::health_handler,
        handlers::models_list_handler,
        handlers::transcribe_handler
    ),
    components(
        schemas(
            TranscriptionResponse,
            Segment,
            HealthResponse,
            ModelsResponse,
            ModelInfo
        )
    ),
    tags(
        (name = "Health", description = "Service health and status endpoints"),
        (name = "Models", description = "Whisper model management endpoints"),
        (name = "Transcription", description = "Speech-to-text transcription endpoints")
    ),
    external_docs(
        url = "https://github.com/your-org/voice-cli",
        description = "Voice CLI GitHub Repository"
    )
)]
pub struct ApiDoc;

/// Create Swagger UI service
pub fn create_swagger_ui() -> SwaggerUi {
    SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi())
        .config(utoipa_swagger_ui::Config::new(["/api-docs/openapi.json"]))
}

/// Get OpenAPI JSON specification
pub fn get_openapi_json() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}
