//! STT 全局 GPU 加速配置（启动时一次性、幂等）。
//!
//! transcribe-rs 的 WhisperEngine::load 会读取全局 accelerator，
//! 故只需在启动早期调一次 [`init_global_accel`]，后续所有 load 自动生效。
//!
//! 编译期 feature 决定可用后端（见 voice-cli Cargo.toml 的 `[target]` 段）：
//! - macOS：默认 `whisper-metal`（Metal / CoreML）
//! - Linux/其他：默认 `whisper-cpp`（CPU）；GPU 需 `--features cuda`/`vulkan`

use std::sync::OnceLock;

use transcribe_rs::{
    GPU_DEVICE_AUTO, WhisperAccelerator, set_whisper_accelerator, set_whisper_gpu_device,
};

static INIT: OnceLock<()> = OnceLock::new();

/// 配置 STT 全局 GPU 加速（幂等：重复调用只生效第一次）。
///
/// - `device = "cpu"`：强制 CPU
/// - 其它（`"auto"` / `"gpu"` / `"metal"` / `"cuda"` / `"vulkan"`）：启用 GPU；
///   实际后端由编译期 feature 决定（无对应 feature 时 whisper.cpp 自动 fallback CPU，不报错）
pub fn init_global_accel(device: &str) {
    INIT.get_or_init(|| {
        let use_gpu = !matches!(device.to_ascii_lowercase().as_str(), "cpu");
        let accel = if use_gpu {
            WhisperAccelerator::Gpu
        } else {
            WhisperAccelerator::CpuOnly
        };
        set_whisper_accelerator(accel);
        set_whisper_gpu_device(GPU_DEVICE_AUTO);
        tracing::info!(
            "STT accelerator configured: device={device}, use_gpu={use_gpu} \
             (backend by compile feature: mac=metal/CoreML; linux=cpu unless --features cuda/vulkan)"
        );

        // SenseVoice ONNX（ort）加速：独立全局，须在任何 SenseVoiceModel::load 之前设置。
        // feature 未启用时本块编译期移除；启用时 ort 按 device 选 EP（缺对应 ort-* feature 则 ort 自降级 CPU 并告警）。
        #[cfg(feature = "sensevoice")]
        {
            use transcribe_rs::{OrtAccelerator, set_ort_accelerator};
            let ort = match device.to_ascii_lowercase().as_str() {
                "cpu" => OrtAccelerator::CpuOnly,
                "cuda" | "gpu" => OrtAccelerator::Cuda,
                #[cfg(target_os = "macos")]
                "metal" | "coreml" => OrtAccelerator::CoreMl,
                // 含 "auto"：ort 自选已编译的 EP（mac→CoreML，linux→CUDA，否则 CPU）
                _ => OrtAccelerator::Auto,
            };
            set_ort_accelerator(ort);
            tracing::info!("ORT(SenseVoice) accelerator configured: device={device}, ort={ort}");
        }
    });
}
