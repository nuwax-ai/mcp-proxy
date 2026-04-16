use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum TtsAction {
    /// Initialize TTS environment
    Init {
        /// Force overwrite existing environment
        #[arg(long)]
        force: bool,
    },
    /// Test TTS functionality
    Test {
        /// Text to synthesize
        #[arg(short, long, default_value = "Hello, world!")]
        text: String,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Model to use
        #[arg(short, long)]
        model: Option<String>,

        /// Speech speed (0.5-2.0)
        #[arg(short, long, default_value = "1.0")]
        speed: f32,

        /// Pitch adjustment (-20 to 20)
        #[arg(short, long, default_value = "0")]
        pitch: i32,

        /// Volume adjustment (0.5-2.0)
        #[arg(short, long, default_value = "1.0")]
        volume: f32,

        /// Output format
        #[arg(short, long, default_value = "mp3")]
        format: String,
    },
}

/// Initialize TTS environment
pub async fn handle_tts_init(_force: bool) -> anyhow::Result<()> {
    println!("🎤 Initializing TTS environment...");

    // Reuse the server init logic for Python environment
    crate::server::init_python_tts_environment()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to initialize TTS environment: {}", e))?;

    println!("✅ TTS environment initialized successfully");
    Ok(())
}

/// Test TTS functionality
pub async fn handle_tts_test(
    config: &crate::Config,
    text: String,
    output: Option<std::path::PathBuf>,
    model: Option<String>,
    speed: f32,
    pitch: i32,
    volume: f32,
    format: String,
) -> anyhow::Result<()> {
    println!("🎤 Testing TTS functionality...");

    // Create TTS service
    let tts_service = crate::services::TtsService::new(
        config.tts.python_path.clone(),
        config.tts.model_path.clone(),
    )
    .map_err(|e| anyhow::anyhow!("Failed to create TTS service: {}", e))?;

    // Create request
    let request = crate::models::TtsSyncRequest {
        text,
        model,
        speed: Some(speed),
        pitch: Some(pitch),
        volume: Some(volume),
        format: Some(format),
    };

    // Test synthesis
    let result_path = tts_service
        .synthesize_sync(request)
        .await
        .map_err(|e| anyhow::anyhow!("TTS synthesis failed: {}", e))?;

    println!("✅ TTS test successful!");
    println!("📁 Output file: {}", result_path.display());

    // Copy to specified output path if provided
    if let Some(output_path) = output {
        tokio::fs::copy(&result_path, &output_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to copy output file: {}", e))?;
        println!("📁 Copied to: {}", output_path.display());
    }

    Ok(())
}

/// Handle TTS-related commands
pub async fn handle_tts_command(action: TtsAction, config: &crate::Config) -> anyhow::Result<()> {
    match action {
        TtsAction::Init { force } => handle_tts_init(force).await,

        TtsAction::Test {
            text,
            output,
            model,
            speed,
            pitch,
            volume,
            format,
        } => handle_tts_test(config, text, output, model, speed, pitch, volume, format).await,
    }
}
