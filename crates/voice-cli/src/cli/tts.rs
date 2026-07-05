use clap::Subcommand;
use std::path::PathBuf;

use crate::tts::{
    AudioFormat, TtsKey, TtsLoadParams, TtsModelService, TtsOptions, get_or_init_tts,
};

#[derive(Subcommand)]
pub enum TtsAction {
    /// 准备 TTS 运行环境（v1 = 检查 Kokoro 模型是否就绪；Python 已移除）
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

        /// Output file path（默认 ./data/tts/tts_test.wav）
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Speaker id（Kokoro voices.bin 索引）
        #[arg(short = 'S', long, default_value = "0")]
        sid: i32,

        /// Speech speed (1.0 = 原速)
        #[arg(short, long, default_value = "1.0")]
        speed: f32,

        /// Output format（wav / pcm_s16le）
        #[arg(short = 'f', long, default_value = "wav")]
        format: String,
    },
}

/// 准备 TTS 运行环境（v1：检查 Kokoro 模型就绪；不再有 Python 依赖）。
pub async fn handle_tts_init(_force: bool, config: &crate::Config) -> anyhow::Result<()> {
    println!("🎤 Checking TTS (sherpa-onnx Kokoro) environment...");

    let svc = TtsModelService::new(&config.tts.engine.models_dir);
    let model_id = &config.tts.engine.default_model;
    match svc.ensure_model(model_id) {
        Ok(()) => println!("✅ TTS model ready: {model_id}"),
        Err(e) => {
            println!("⚠️  TTS 模型未就绪：{e}");
            println!(
                "💡 提示：从 https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models 下载 Kokoro 模型，解压到 {}/{}",
                config.tts.engine.models_dir, model_id
            );
        }
    }
    Ok(())
}

/// TTS 测试参数
pub struct TtsTestParams {
    pub text: String,
    pub output: Option<PathBuf>,
    pub sid: i32,
    pub speed: f32,
    pub format: String,
}

/// 同步合成并落盘（CLI 路径，不经 HTTP）。
pub async fn handle_tts_test(config: &crate::Config, params: TtsTestParams) -> anyhow::Result<()> {
    let TtsTestParams {
        text,
        output,
        sid,
        speed,
        format,
    } = params;
    println!("🎤 Testing TTS (sherpa-onnx Kokoro)...");

    let model_id = config.tts.engine.default_model.clone();
    let svc = TtsModelService::new(&config.tts.engine.models_dir);
    svc.ensure_model(&model_id)
        .map_err(|e| anyhow::anyhow!("TTS 模型未就绪: {e}"))?;
    let paths = svc
        .resolve_paths(&model_id)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let fmt = AudioFormat::parse(&format);
    let opts = TtsOptions {
        sid,
        speed,
        ..Default::default()
    };
    let load_params = TtsLoadParams {
        paths: paths.clone(),
        num_threads: config.tts.engine.num_threads,
        length_scale: config.tts.engine.default_length_scale,
        provider: config.tts.engine.provider.clone(),
        pool_size: config.tts.engine.pool_size,
        debug: config.tts.engine.debug,
        lang: config.tts.engine.default_language.clone(),
    };

    // 同步合成走 spawn_blocking
    let text_owned = text.clone();
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<(Vec<u8>, AudioFormat)> {
        let pool = get_or_init_tts(TtsKey::new(&model_id), load_params)?;
        let inst = pool.pick();
        let guard = inst.lock().unwrap_or_else(|p| p.into_inner());
        let audio = crate::tts::synthesize(&guard, &text_owned, &opts)?;
        let bytes = crate::tts::encode(&audio.samples, audio.sample_rate, fmt)?;
        Ok((bytes, fmt))
    })
    .await
    .map_err(|e| anyhow::anyhow!("TTS join 失败: {e}"))??;

    let (bytes, fmt) = result;
    tokio::fs::create_dir_all("./data/tts")
        .await
        .map_err(|e| anyhow::anyhow!("创建输出目录失败: {e}"))?;
    let out = output.unwrap_or_else(|| PathBuf::from("./data/tts/tts_test.").join(fmt.ext()));
    tokio::fs::write(&out, &bytes)
        .await
        .map_err(|e| anyhow::anyhow!("写入输出文件失败: {e}"))?;

    println!("✅ TTS test successful! {} bytes", bytes.len());
    println!("📁 Output file: {}", out.display());
    Ok(())
}

/// Handle TTS-related commands
pub async fn handle_tts_command(action: TtsAction, config: &crate::Config) -> anyhow::Result<()> {
    match action {
        TtsAction::Init { force } => handle_tts_init(force, config).await,
        TtsAction::Test {
            text,
            output,
            sid,
            speed,
            format,
        } => {
            handle_tts_test(
                config,
                TtsTestParams {
                    text,
                    output,
                    sid,
                    speed,
                    format,
                },
            )
            .await
        }
    }
}
