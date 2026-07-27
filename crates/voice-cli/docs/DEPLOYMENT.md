# voice-cli 部署手册

> 本地一体化语音服务：**STT**（transcribe-rs + Whisper，Metal/CoreML GPU 加速）+ **TTS**（sherpa-onnx；Kokoro 标准多音色 / ZipVoice 零样本克隆，RTF<0.3）。
> 异步任务 apalis + SQLite 持久化；HTTP axum + utoipa（Swagger `/api/docs`）。

---

## 1. 架构概览

| 子系统 | 引擎 | GPU | HTTP 接口 |
|---|---|---|---|
| STT | transcribe-rs 0.3.11（Whisper.cpp 绑定） | Metal（mac）/ CUDA / Vulkan（linux，cargo feature） | **兼容旧接口**（`/transcribe` 等，加可选字段） |
| TTS | sherpa-onnx 1.13.3（Kokoro v1_1 103 音色 / ZipVoice 克隆） | CPU；GPU（coreml/cuda） | **重新设计**（`/api/v1/tts*`） |
| 任务队列 | apalis + SQLite | — | 异步 STT/TTS 持久化、可恢复 |
| HTTP | axum 0.8 + tower + utoipa | — | REST + WebSocket 流式 |

引擎栈统一 ONNX 生态：transcribe-rs 用 `ort=2.0.0-rc.12`（与 fastembed 一致），sherpa-onnx 自带 onnxruntime C 库（隔离无冲突）。

---

## 2. 系统要求

- **OS**：macOS 12+（M1/M2/M3）/ Linux x86_64+ARM64 / Windows
- **Rust**：stable toolchain（rustup）
- **FFmpeg**：系统 PATH（音频转码 16k/mono/s16le）。无 ffmpeg 时启动报错，需手动装：`brew install ffmpeg` / `apt install ffmpeg`
- **磁盘**：模型 ~550MB 起（STT base 141MB + TTS Kokoro v1_1 408MB；可选 ZipVoice 156MB + vocos 52MB）+ 编译产物 ~1.5GB

### 平台 GPU 矩阵

| 平台 | STT EP | TTS | 编译 |
|---|---|---|---|
| macOS（开发） | Metal / CoreML | CPU | 裸 `cargo build`（`whisper-metal` 平台默认开） |
| Linux + NVIDIA | CUDA | CPU（v1） | `cargo build --features cuda` |
| Linux 通用（AMD/Intel/NVIDIA） | Vulkan | CPU | `cargo build --features vulkan`（非默认，需显式开；详见 §3.3） |

---

## 3. 编译

### 3.1 一次性：缓存 sherpa-onnx C 库（关键）

`sherpa-onnx-sys` 默认联网下载预编译 C 库；github 阻断时会卡死。用 **gh-proxy** 预下载到本地：

```bash
mkdir -p ~/.cache/sherpa-onnx-prebuilt && cd ~/.cache/sherpa-onnx-prebuilt

# macOS arm64
curl -SL -o sherpa-onnx-v1.13.3-osx-arm64-static-lib.tar.bz2 \
  https://gh-proxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.3/sherpa-onnx-v1.13.3-osx-arm64-static-lib.tar.bz2
# Linux x86_64 → sherpa-onnx-v1.13.3-linux-x64-static-lib.tar.bz2
# Linux arm64 → sherpa-onnx-v1.13.3-linux-aarch64-static-lib.tar.bz2
```

> gh-proxy 是 github 阻断时的通用解法：`curl -L https://gh-proxy.com/<github-url>`。源可达时直接用原 URL 即可。

### 3.2 编译命令

```bash
# 每次编译都带这个环境变量（指向缓存目录，-sys 跳过联网）
export SHERPA_ONNX_ARCHIVE_DIR="$HOME/.cache/sherpa-onnx-prebuilt"

cd /path/to/mcp-proxy
cargo build -p voice-cli                 # debug
cargo build -p voice-cli --release       # 生产（推荐，CPU 推理快 2-3 倍）

# Linux NVIDIA GPU
cargo build -p voice-cli --features cuda --release

# Linux AMD / Intel GPU（Vulkan，仅加速 STT whisper；详见 §3.3）
cargo build -p voice-cli --features vulkan --release
```

二进制产物：`target/debug/voice-cli` 或 `target/release/voice-cli`。

### 3.3 AMD / Intel GPU（Vulkan）部署 —— 仅加速 STT

> **适用范围**：仅 STT 的 **whisper 引擎**（批量 + 流式 LA2）。**TTS（Kokoro/ZipVoice）和 sherpa ASR
> （FireRedASR2/Fun/Qwen3）仍走 CPU** —— sherpa-onnx 上游 EP 只有 `cpu`/`cuda`/`coreml`，
> 不支持 vulkan（onnxruntime 的 Vulkan EP 至今仍是 [feature request #21917](https://github.com/microsoft/onnxruntime)，未发布）。

whisper.cpp 的 **ggml Vulkan backend** 跨厂商支持 AMD/Intel/NVIDIA，无需 CUDA。
whisper.cpp 1.8.3 在 AMD/Intel 核显上实测有显著加速（参考 [Phoronix](https://www.phoronix.com/news/Whisper-cpp-1.8.3-12x-Perf)）。

**1）装 Vulkan 驱动 + 编译依赖**

```bash
# Debian / Ubuntu（AMD 用 mesa RADV；Intel 用 anv）
sudo apt install -y libvulkan-dev glslang-tools mesa-vulkan-drivers vulkan-tools
# 验证 GPU 被 Vulkan 识别（应列出 AMD / Intel 设备名）
vulkaninfo --summary
```

> macOS 不适用（Mac 走 Metal）。Windows 需 [LunarG Vulkan SDK](https://vulkan.lunarg.com/)，但 AMD 上 Linux（RADV）体验更稳，推荐 Linux。

**2）编译（显式开 vulkan feature）**

```bash
export SHERPA_ONNX_ARCHIVE_DIR="$HOME/.cache/sherpa-onnx-prebuilt"
cargo build -p voice-cli --features vulkan --release
```

**3）配置 `whisper.engine.device: gpu`**

accel.rs → `WhisperAccelerator::Gpu` → whisper.cpp 用编译进来的 vulkan backend。

```yaml
whisper:
  default_model: "base"          # AMD/Intel 显存有限建议 base / small
  engine:
    device: gpu                  # 非 cpu 即启用 GPU；实际后端由编译期 feature 决定
```

**4）启动日志确认（关键 —— 没装好会静默回退 CPU，不报错）**

```
ggml_vulkan: Found 1 Vulkan devices: ...
STT accelerator configured: device=gpu, use_gpu=true (backend by compile feature: ... vulkan)
```

看到 `Found N Vulkan devices` 才算真吃到 GPU；否则已 fallback CPU（查驱动 / SDK）。

**限制与注意**
- TTS / sherpa ASR 不受 vulkan 加速（见上适用范围），AMD 机器跑 TTS 仍 CPU。
- 缺 Vulkan SDK / 驱动时 whisper.cpp 自动回退 CPU，**不报错**，必须看日志确认。
- Linux（RADV）AMD 体验优于 Windows；建议 `base`/`small` 实测 RTF 后再决定模型档位。

---

## 4. 模型准备

工作目录以 `~/voice-cli-test/` 为例（可任意，服务读取 CWD 下 `config.yml`）。

### 4.1 STT — Whisper ggml（ModelScope 镜像，HF 阻断）

```bash
mkdir -p ~/voice-cli-test/models && cd ~/voice-cli-test/models
# tiny 74MB（CPU 也能实时）/ base 141MB（推荐，Metal ~0.8s/30s 音频）
curl -L -o ggml-tiny.bin \
  https://modelscope.cn/models/cjc1887415157/whisper.cpp/resolve/master/ggml-tiny.bin
curl -L -o ggml-base.bin \
  https://modelscope.cn/models/cjc1887415157/whisper.cpp/resolve/master/ggml-base.bin
```

可选 `small`/`medium`/`large-v3`（精度↑速度↓）。

### 4.2 TTS — Kokoro multi-lang v1.1（默认，103 音色）

```bash
mkdir -p ~/voice-cli-test/models/tts && cd ~/voice-cli-test/models/tts
curl -SL -o kokoro.tar.bz2 \
  https://gh-proxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_1.tar.bz2
tar xjf kokoro.tar.bz2 && rm kokoro.tar.bz2
```

校验 `kokoro-multi-lang-v1_1/` 内容：`model.onnx` / `voices.bin`（103 音色）/ `tokens.txt` / `espeak-ng-data/` / `dict` / `lexicon-us-en.txt` + `lexicon-zh.txt` + `lexicon-gb-en.txt`。

> ⚠️ **Kokoro multi-lang 必需** `lexicon-us-en.txt + lexicon-zh.txt`（不含 gb-en）。代码 `resolve_paths` 已自动选对；否则 sherpa-onnx C 端抛 foreign exception 或 `std::exit`。

### 4.2.1 TTS — ZipVoice（可选，零样本克隆）

ZipVoice 是零样本**克隆**引擎（中英）：每次合成需 reference 音频 + 文本决定音色。双轨：预置 profile（`config.tts.engine.zipvoice.voices`，请求 `voice:<name>`）或动态上传 `reference_audio`（base64 WAV）。

```bash
cd ~/voice-cli-test/models/tts
# 主模型（含 encoder/decoder/tokens/espeak-ng-data/lexicon.txt/test_wavs）
curl -SL -o zipvoice.tar.bz2 \
  https://gh-proxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/sherpa-onnx-zipvoice-distill-int8-zh-en-emilia.tar.bz2
tar xjf zipvoice.tar.bz2 && rm zipvoice.tar.bz2
# vocoder（vocos_24khz.onnx）独立下载，放 ZipVoice 目录
curl -SL -o sherpa-onnx-zipvoice-distill-int8-zh-en-emilia/vocos_24khz.onnx \
  https://gh-proxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download/vocoder-models/vocos_24khz.onnx
```

启用：`config.tts.engine.backend: zipvoice` + `default_model: sherpa-onnx-zipvoice-distill-int8-zh-en-emilia` + `zipvoice.voices: [{name, reference_wav, reference_text}]`（示例见 `deploy/config.example.yml`；或 `FETCH_ZIPVOICE=1 bash scripts/dev/fetch-voice-models.sh`）。

### 4.3 测试音频

```bash
cd ~/voice-cli-test
# macOS say + ffmpeg 生成 16kHz mono wav
say -o jfk.aiff "and so my fellow Americans, ask not what your country can do for you"
ffmpeg -y -i jfk.aiff -ar 16000 -ac 1 jfk.wav && rm jfk.aiff
```

### 4.4 STT（可选）— SenseVoice ONNX（中文更准，原生简体）

SenseVoice（阿里 FunASR）经 transcribe-rs `onnx` feature 接入，与 whisper 并存。中文 CER 优于 whisper、推理更快、原生输出简体。**仅批量**（非流式模型，流式端点仍走 whisper）。

```bash
# 1) 编译带 sensevoice feature（Mac CPU ONNX；Linux NVIDIA GPU 用 sensevoice-cuda）
cd /path/to/mcp-proxy
cargo build -p voice-cli --features sensevoice           # Mac dev
# cargo build -p voice-cli --features "cuda sensevoice-cuda"  # Linux server（需 CUDA toolkit）

# 2) 拉模型（sherpa-onnx 布局：model.int8.onnx + tokens.txt，int8 ~900MB）
cd ~/voice-cli-test
bash /path/to/mcp-proxy/scripts/dev/fetch-sensevoice-model.sh
# → ./models/sensevoice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/

# 3) config.yml 切后端（whisper.engine 下）
# whisper:
#   engine:
#     backend: sensevoice
#     sensevoice:
#       model_dir: "./models/sensevoice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"
#       quantization: "int8"
```

A/B 对比（同音频，免重启）：`/api/v1/transcribe` 传 `-F engine=sensevoice` 或 `-F engine=whisper` 覆盖后端。默认 `backend: whisper`，未拉 SenseVoice 模型不影响现有 whisper 链路。

> SenseVoice 不支持流式：`backend: sensevoice` 时 `/api/v1/stream/transcribe` 返回明确错误（不静默回退）。需要流式请保持 `backend: whisper`。

### 4.5 STT（可选）— sherpa-onnx 新模型（FireRedASR2 / Fun-ASR-Nano / Qwen3-ASR，Mac CoreML GPU）

三个 2025-2026 新中文 ASR 模型经 **sherpa-onnx** `OfflineRecognizer` 接入（与 TTS 同库），**不走 transcribe-rs**（transcribe-rs 的 `ort` 库在 Mac 上 CoreML EP 不可用，且不支持这三模型）。关键：**sherpa-onnx 预编译 macOS lib 含 CoreMLExecutionProvider** → 这三模型在 Mac 可走 CoreML GPU/ANE（与 SenseVoice 的 CPU-only 路径不同）。三者均**仅批量**，流式端点仍 whisper。

| 模型 | 架构 | 大小(int8) | 语种 | 备注 |
|---|---|---|---|---|
| `fireredasr2` | AED（encoder-decoder，无自回归 LLM） | ~1.2G | zh/en/20+ 方言 | **Mac CoreML 加速最显著**；AISHELL CER ~2.89% |
| `funasrnano` | audio-encoder + Qwen3-0.6B LLM decoder | ~950M | 31 语+7 方言+热词 | LLM-decoder，CoreML 加速有限；含热词 |
| `qwen3asr` | conv frontend + encoder + LLM decoder | ~940M | 52 语+22 方言+热词 | LLM-decoder；长音频需调 `max_new_tokens` |

```bash
# 1) 拉模型（任选，sherpa-onnx asr-models release，gh-proxy 镜像自动探活）
cd ~/voice-cli-test
bash /path/to/mcp-proxy/scripts/dev/fetch-asr-models.sh fireredasr2
# bash /path/to/mcp-proxy/scripts/dev/fetch-asr-models.sh funasrnano
# bash /path/to/mcp-proxy/scripts/dev/fetch-asr-models.sh qwen3asr
# → ./models/<kind>/sherpa-onnx-...-int8-.../

# 2) 编译（无需新 feature：sherpa-onnx C 库本就为 TTS 无条件链接）
cd /path/to/mcp-proxy
SHERPA_ONNX_ARCHIVE_DIR="$HOME/.cache/sherpa-onnx-prebuilt" cargo build -p voice-cli

# 3) config.yml 切后端 + 开 CoreML（whisper.engine 下）
# whisper:
#   engine:
#     backend: fireredasr2          # 或 funasrnano / qwen3asr
#     sherpa:
#       provider: coreml            # Mac GPU/ANE；留空=CPU；Linux 用 cuda
#       fireredasr2:
#         model_dir: "./models/fireredasr2/sherpa-onnx-fire-red-asr2-zh_en-int8-2026-02-26"

# 4) 跑（务必带 --config）
SHERPA_ONNX_ARCHIVE_DIR="$HOME/.cache/sherpa-onnx-prebuilt" \
  cargo run -p voice-cli -- server run --config config.yml
```

A/B 对比（同音频，免重启）：`/api/v1/transcribe` 传 `-F engine=fireredasr2|funasrnano|qwen3asr` 覆盖后端；切 `sherpa.provider` 在 coreml/空之间对比 Mac GPU vs CPU。默认 `backend: whisper`，未拉模型不影响现有链路。

> ⚠️ **Fun-ASR-Nano 默认值陷阱**（sherpa-onnx issue #3066）：Rust `OfflineFunASRNanoModelConfig::default()` 的生成参数是错的（`max_new_tokens:0`/`temperature:1.0`/无 prompt → 乱码重复）。代码已硬编码 C++ 工作默认（`build_recognizer`），用户无需配置；若换库版本需复核。
>
> 三模型均不支持流式：`backend` 为任一 sherpa 模型时 `/api/v1/stream/transcribe` 返回明确错误（不静默回退）。

---

## 5. 配置

工作目录放 `config.yml`（`voice-cli server run` 默认读取 CWD 下 `config.yml`；也可 `--config <path>` 指定）。

> 用 `voice-cli server init` 可生成 `server-config.yml` 模板，再重命名/编辑为 `config.yml`。

完整最小可用配置（已实测）：

```yaml
# Voice CLI Server Configuration

server:
  host: "0.0.0.0"
  port: 8080
  max_file_size: 209715200      # 200MB 上传上限
  cors_enabled: true

whisper:
  default_model: "base"          # 启动默认 STT 模型（对应 models/ggml-base.bin）
  models_dir: "./models"
  auto_download: false           # 建议 false（联网下载受限），模型手动放好
  supported_models: ["tiny", "base", "small", "medium", "large-v3"]
  audio_processing:
    supported_formats: ["mp3", "wav", "flac", "m4a", "ogg", "aac", "opus", "mp4"]
    auto_convert: true           # ffmpeg-sidecar 自动转 16k/mono/s16le
    conversion_timeout: 60
    temp_file_cleanup: true
    temp_file_retention: 300
  workers:
    transcription_workers: 3
    channel_buffer_size: 100
    worker_timeout: 3600
  # engine:                        # STT 引擎配置（可选，全部有默认值）
  #   output_script: "simplified"  # 输出脚本：simplified(默认,繁→简;英文透传) / original(原样)
  # streaming:                     # STT 流式（可选，全部有默认值）
  #   decode_interval_sec: 0.5     # 解码触发间隔
  #   tail_trim_sec: 0.3           # B 解码尾部裁剪
  #   min_agree_count: 2           # LA2 前缀稳定阈值
  #   buffer_max_sec: 30           # ★ 长会话 utterance 切分阈值（超时 flush+reset，封顶 O(n²)；详见 API.md §4）
  #   idle_timeout_sec: 30
  #   decode_timeout_sec: 30
  #   compare_granularity: "auto"  # auto（按 language 推断）/ char / word

tts:
  enabled: true                  # ⚠️ 必须 true，否则 /api/v1/tts* 返回 403
  max_text_length: 5000
  engine:
    pool_size: 1                 # 引擎实例数（CPU 最优 1；多实例并发但内存×N）
    default_model: "kokoro-multi-lang-v1_1"
    default_sid: 0               # 默认音色 id（0-52）
    default_speed: 1.0           # 语速（1.0 原速）
    default_length_scale: 1.0    # 时长缩放（model-level，仅首次加载生效）
    num_threads: 4
    models_dir: "./models/tts"
  # streaming:                   # 可选，全部有默认值
  #   idle_timeout_sec: 30
  #   synth_timeout_sec: 120
  #   default_format: "pcm_s16le"

logging:
  level: "info"                  # trace/debug/info/warn/error
  log_dir: "./logs"
  max_file_size: "100MB"
  max_files: 30

daemon:
  pid_file: "./voice-cli-server.pid"
```

配置层级（后者覆盖前者）：代码默认值 → `config.yml` → 环境变量（前缀 `VOICE_CLI_`，如 `VOICE_CLI_TTS_ENGINE_POOL_SIZE=2`）→ CLI 参数。

---

## 6. 启动

```bash
cd ~/voice-cli-test
/path/to/mcp-proxy/target/release/voice-cli server run
# 或 debug：/path/to/mcp-proxy/target/debug/voice-cli server run
```

### 启动日志确认（关键）

```
STT accelerator configured: device=auto, use_gpu=true
  (backend by compile feature: mac=metal/CoreML; linux=cpu unless --features cuda/vulkan)
...
Server listening on 0.0.0.0:8080
```

- `use_gpu=true` → STT 走 Metal/CUDA（mac 默认）。若 `false`，检查平台 feature。
- 监听后 `curl http://localhost:8080/health` 应返回 200。

### 健康检查 + Swagger

```bash
curl -s http://localhost:8080/health | python3 -m json.tool
# 浏览器打开 http://localhost:8080/api/docs  → Swagger UI 在线调试
```

---

## 7. 故障排查

| 现象 | 原因 / 解决 |
|---|---|
| 编译报 `Failed to download sherpa-onnx archive` / 卡住 | 没设 `SHERPA_ONNX_ARCHIVE_DIR`，或缓存目录里缺对应平台的 tar.bz2 |
| 启动报 `Failed to load server configuration` | CWD 下没有 `config.yml`，或 YAML 语法错。先 `voice-cli server init` 生成模板 |
| `/api/v1/tts*` 返回 `TTS service is disabled`（403） | `config.yml` 缺 `tts.enabled: true` |
| TTS 调用进程崩溃、日志含 `Rust cannot catch foreign exceptions` | Kokoro lexicon 组合错。确认用 `lexicon-us-en.txt + lexicon-zh.txt`（不含 gb-en） |
| TTS 报 `text contains NUL` 或 panic | 输入文本含控制字符，需预清洗（代码 `sanitize_text` 已处理；自建客户端避免传 NUL） |
| STT 很慢（30s 音频 >5s） | Metal 未启用：启动日志应有 `use_gpu=true`；mac 检查 `[target.'cfg(target_os="macos")']` 的 `whisper-metal` feature |
| STT 报 `No such file ggml-base.bin` | 模型未下载，或 `whisper.models_dir` 路径不对 |
| `ffmpeg not found` | 系统装 ffmpeg：`brew install ffmpeg` / `apt install ffmpeg` |
| 端口 8080 占用 | `lsof -i :8080` 找进程 kill，或改 `config.yml` 的 `server.port` |
| WS 客户端连不上 / SOCKS proxy 报错 | python `websockets` 自动探测系统代理，客户端需传 `proxy=None`（见 API.md 客户端） |
| 模型下载慢/失败 | HF 阻断：STT 用 modelscope.cn，TTS 用 gh-proxy.com（见 §4） |

---

## 8. 性能基线（M2 Max，base 模型）

| 场景 | 耗时 |
|---|---|
| STT 同步（jfk.wav ~8s，CPU） | ~1.5s |
| STT 同步（jfk.wav ~8s，Metal） | ~0.3s |
| TTS 同步（"你好世界"，wav） | ~0.4s（RTF≈0.3） |
| TTS 异步 Pending→Completed | ~1.9s（含排队） |
| TTS 流式首字节 | ~0.2s（callback 增量） |

---

## 9. GPU 升级路径（TTS v2，可选）

v1 TTS 走 CPU（sherpa-onnx 默认预编译库 CPU-only，无 cargo GPU feature）。要 TTS GPU：

1. 从 sherpa-onnx 源码自编 C++ 库（Mac CoreML / Linux CUDA）：
   ```bash
   git clone https://github.com/k2-fsa/sherpa-onnx
   cd sherpa-onnx && mkdir build && cd build
   cmake .. -DSHERPA_ONNX_ENABLE_COREML=ON   # mac
   # 或 -DSHERPA_ONNX_ENABLE_CUDA=ON（linux，需 CUDA toolkit）
   make -j
   ```
2. 设 `SHERPA_ONNX_LIB_DIR=/path/to/sherpa-onnx/build/lib` 重新编译 voice-cli
3. `config.yml` 设 `tts.engine.provider: "coreml"`（Mac）/ `"cuda"`（Linux NVIDIA）。**不含 vulkan**
   （sherpa-onnx 无 Vulkan EP；AMD/Intel GPU 跑 TTS 只能 CPU，见 §3.3）

Kokoro CPU RTF<0.3 已快于实时，多数场景无需 GPU。GPU（coreml/cuda）仅对高并发或长文本批量合成有收益；ZipVoice 同走 sherpa（GPU 同 Kokoro）。

> 💡 **引擎懒加载 + 启动预热**：引擎首次加载 ~10-20s（`OfflineTts::create` + EP 初始化）。`config.tts.engine.warmup: true`（默认）启动期预热默认引擎（listen 前，完成后接请求），首个用户即命中缓存；关 warmup 则首个用户触发懒加载（慢 ~15s）。STT 仍懒加载（warmup 暂只 TTS）。

---

## 10. 生产部署建议

- 用 `--release` 编译（CPU 推理快 2-3×）
- `tts.engine.pool_size` 按并发调（每实例 ~1.5GB 内存，CPU 核数 ×0.5 上限）
- STT `transcription_workers` ≤ CPU 核数
- 异步任务 DB（`./data/`）放 SSD；定期清 `cleanup_expired_tasks`
- 前置 nginx 反代 + 限流；WS 加 `proxy_read_timeout` 长连接超时
- 日志 `log_dir` 挂独立卷，`tracing-appender` 按天轮转
- 健康检查接负载均衡：`GET /health`

---

参见 [API.md](./API.md) 获取全部 HTTP/WebSocket 接口的集成测试用例。
