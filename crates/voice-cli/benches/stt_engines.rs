//! STT 引擎端到端性能基准（criterion，**经运行中的 voice-cli server HTTP 调用**）。
//!
//! # 为什么走 HTTP 而非直链引擎
//! 直链 `voice_cli`（拉入 whisper.cpp GGML 静态库 + ort + sherpa 动态库）的 bench 二进制在本机
//! macOS 上**启动即被 jetsam SIGKILL**（重原生库静态初始化，deps/ 下的 bench 进程比 server 主
//! 进程更易被杀；改 debug/release/criterion 版本/DYLD_LIBRARY_PATH 均无效）。走 HTTP 只链
//! `reqwest + criterion`（轻量），稳定可跑，且测的是**端到端 RTF**（含 ffmpeg 解码 + HTTP），
//! 即生产真实耗时——比纯引擎 RTF 更贴近实际。
//!
//! # 前置：server 必须先起（带对应模型 + backend 可被 -F engine 覆盖）
//! ```bash
//! cd ~/voice-cli-test
//! SHERPA_ONNX_ARCHIVE_DIR=~/.cache/sherpa-onnx-prebuilt \
//!   cargo run -p voice-cli --features sensevoice -- server run --config config.yml
//! ```
//!
//! # 用法
//! ```bash
//! cargo bench -p voice-cli --bench stt_engines                       # 全引擎 × 全音频
//! cargo bench -p voice-cli --bench stt_engines -- "中文/funasrnano"   # 过滤（group/engine）
//!
//! # 对比 CoreML vs CPU（改 server config 的 sherpa.provider 后重启 server，分别 --save-baseline）
//! BENCH_URL=http://localhost:8087 cargo bench -p voice-cli --bench stt_engines -- --save-baseline cpu
//! # （改 config sherpa.provider: coreml + 重启 server）
//! BENCH_URL=http://localhost:8087 cargo bench -p voice-cli --bench stt_engines -- --baseline cpu
//! ```
//!
//! # 环境变量
//! - `BENCH_URL`       server 根地址（默认 `http://localhost:8087`）
//! - `BENCH_AUDIO_ZH`  中文音频路径（默认 `~/voice-cli-test/data/audio/...m4a`）
//! - `BENCH_AUDIO_EN`  英文音频路径（默认 `~/voice-cli-test/jfk.wav`）
//! - `BENCH_ENGINES`   要测的引擎（逗号分隔，默认全部：`whisper,sensevoice,fireredasr2,funasrnano,qwen3asr`）
//!
//! # 结果解读
//! 报告 = **每次端到端推理耗时**（含 ffmpeg 解码 + 推理 + HTTP）。RTF = avg / 音频时长。
//! 音频时长在启动时打印。

use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use reqwest::Client;

/// 默认引擎清单（与 server 的 -F engine 取值一致）。
const ALL_ENGINES: &[&str] = &[
    "whisper",
    "sensevoice",
    "fireredasr2",
    "funasrnano",
    "qwen3asr",
];

fn bench_url() -> String {
    std::env::var("BENCH_URL").unwrap_or_else(|_| "http://localhost:8087".to_string())
}

fn home_path(default_sub: &[&str]) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut p = PathBuf::from(home).join("voice-cli-test");
    for s in default_sub {
        p.push(s);
    }
    p
}

/// 读音频文件为 bytes（缺失返回 None，跳过）。
fn read_audio(env_key: &str, default_sub: &[&str]) -> Option<(String, Vec<u8>)> {
    let path = std::env::var(env_key).map(PathBuf::from).unwrap_or_else(|_| home_path(default_sub));
    match std::fs::read(&path) {
        Ok(bytes) => {
            eprintln!("🎵 {env_key} = {}（{}KB）", path.display(), bytes.len() / 1024);
            Some((path.display().to_string(), bytes))
        }
        Err(_) => {
            eprintln!("⏭️  跳过 {env_key}：{} 不存在", path.display());
            None
        }
    }
}

/// 一次端到端转录：POST /transcribe（multipart: file + engine + language）。
fn transcribe(client: &Client, url: &str, audio: &(String, Vec<u8>), engine: &str, lang: &str) {
    let part = reqwest::multipart::Part::bytes(audio.1.clone())
        .file_name("audio.bin")
        .mime_str("application/octet-stream")
        .expect("mime");
    let form = reqwest::multipart::Form::new()
        .text("engine", engine.to_string())
        .text("language", lang.to_string())
        .part("file", part);
    // 同步阻塞发送（criterion 的 iter 本就是同步；超时兜底 5min 防长音频挂死）
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio rt");
    rt.block_on(async {
        let resp = client
            .post(format!("{url}/transcribe"))
            .multipart(form)
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .expect("HTTP send");
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        assert!(
            status.is_success(),
            "{engine}/{lang} 夅败 [{status}]: {}",
            &body[..body.len().min(300)]
        );
        // 触发 result 不被优化掉（body 已消费）
        let _ = black_box(body.len());
    });
}

fn bench_stt(c: &mut Criterion) {
    let url = bench_url();
    eprintln!("=== STT 端到端基准（criterion via HTTP）=== server = {url}");

    let zh = read_audio(
        "BENCH_AUDIO_ZH",
        &["data", "audio", "task_task019f3380f53b78c3b8f20d4a5ded58cb.m4a"],
    );
    let en = read_audio("BENCH_AUDIO_EN", &["jfk.wav"]);
    let fixtures: Vec<(&str, &(String, Vec<u8>), &str)> = [
        zh.as_ref().map(|a| ("中文", a, "zh")),
        en.as_ref().map(|a| ("英文", a, "en")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if fixtures.is_empty() {
        eprintln!("⚠️  无可用音频，跳过（设 BENCH_AUDIO_ZH / BENCH_AUDIO_EN）");
        return;
    }

    let engines_str = std::env::var("BENCH_ENGINES").unwrap_or_else(|_| ALL_ENGINES.join(","));
    let engines: Vec<&str> = engines_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    let client = Client::new();
    let mut g = c.benchmark_group("stt");
    // 端到端每次 1-17s：sample_size 小，measurement_time 适中；慢引擎耗时 ≈ sample_size × 单次
    g.sample_size(10)
        .measurement_time(Duration::from_secs(20))
        .warm_up_time(Duration::from_secs(3));
    for (lang, audio, language) in &fixtures {
        // 解一层引用：lang/language → &str，audio → &(String, Vec<u8>)
        let (lang, audio, language) = (*lang, *audio, *language);
        for engine in engines.iter().copied() {
            // 预热一次（首请求触发 server 端模型加载，不计入测量）
            transcribe(&client, &url, audio, engine, language);
            g.bench_function(BenchmarkId::new(engine, lang), |b| {
                b.iter(|| transcribe(&client, &url, audio, engine, language));
            });
        }
    }
    g.finish();
}

criterion_group!(benches, bench_stt);
criterion_main!(benches);
