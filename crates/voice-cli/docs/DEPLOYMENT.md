# voice-cli 部署手册

> 本地一体化语音服务：**STT**（transcribe-rs + Whisper，Metal/CoreML GPU 加速）+ **TTS**（sherpa-onnx + Kokoro，CPU，RTF<0.3）。
> 异步任务 apalis + SQLite 持久化；HTTP axum + utoipa（Swagger `/api/docs`）。

---

## 1. 架构概览

| 子系统 | 引擎 | GPU | HTTP 接口 |
|---|---|---|---|
| STT | transcribe-rs 0.3.11（Whisper.cpp 绑定） | Metal（mac）/ CUDA / Vulkan（linux，cargo feature） | **兼容旧接口**（`/transcribe` 等，加可选字段） |
| TTS | sherpa-onnx 1.13.3（Kokoro multi-lang v1.0） | CPU（v1）；GPU 留 v2 | **重新设计**（`/api/v1/tts*`） |
| 任务队列 | apalis + SQLite | — | 异步 STT/TTS 持久化、可恢复 |
| HTTP | axum 0.8 + tower + utoipa | — | REST + WebSocket 流式 |

引擎栈统一 ONNX 生态：transcribe-rs 用 `ort=2.0.0-rc.12`（与 fastembed 一致），sherpa-onnx 自带 onnxruntime C 库（隔离无冲突）。

---

## 2. 系统要求

- **OS**：macOS 12+（M1/M2/M3）/ Linux x86_64+ARM64 / Windows
- **Rust**：stable toolchain（rustup）
- **FFmpeg**：系统 PATH（音频转码 16k/mono/s16le）。无 ffmpeg 时启动报错，需手动装：`brew install ffmpeg` / `apt install ffmpeg`
- **磁盘**：模型 ~500MB（STT base 141MB + TTS Kokoro 349MB）+ 编译产物 ~1.5GB

### 平台 GPU 矩阵

| 平台 | STT EP | TTS | 编译 |
|---|---|---|---|
| macOS（开发） | Metal / CoreML | CPU | 裸 `cargo build`（`whisper-metal` 平台默认开） |
| Linux + NVIDIA | CUDA | CPU（v1） | `cargo build --features cuda` |
| Linux 通用 | Vulkan | CPU | `cargo build`（`whisper-vulkan` 默认） |

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
```

二进制产物：`target/debug/voice-cli` 或 `target/release/voice-cli`。

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

### 4.2 TTS — Kokoro multi-lang v1.0（gh-proxy 拉 github release）

```bash
mkdir -p ~/voice-cli-test/models/tts && cd ~/voice-cli-test/models/tts
curl -SL -o kokoro.tar.bz2 \
  https://gh-proxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_0.tar.bz2
tar xjf kokoro.tar.bz2 && rm kokoro.tar.bz2
```

校验 `kokoro-multi-lang-v1_0/` 内容：

| 文件 | 大小 | 用途 |
|---|---|---|
| `model.onnx` | 311M | 主模型 |
| `voices.bin` | 26M | 53 个音色 |
| `tokens.txt` | 687B | BPE 词表 |
| `espeak-ng-data/` | — | 音素化 |
| `dict` | 320B | 中文字典 |
| `lexicon-us-en.txt` | 5.7M | 美式英文词表 |
| `lexicon-zh.txt` | 2.3M | 中文词表 |
| `lexicon-gb-en.txt` | 6.1M | 英式英文（**不加载**，与 us-en 词表重叠会触发 C++ 异常） |

> ⚠️ **Kokoro multi-lang 必需** `lexicon-us-en.txt + lexicon-zh.txt`（不含 gb-en）。代码 `resolve_paths` 已自动选对；若换模型布局需检查 lexicon 组合，否则 sherpa-onnx C 端抛 foreign exception（Rust 捕获不到）或 `std::exit`。

### 4.3 测试音频

```bash
cd ~/voice-cli-test
# macOS say + ffmpeg 生成 16kHz mono wav
say -o jfk.aiff "and so my fellow Americans, ask not what your country can do for you"
ffmpeg -y -i jfk.aiff -ar 16000 -ac 1 jfk.wav && rm jfk.aiff
```

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

tts:
  enabled: true                  # ⚠️ 必须 true，否则 /api/v1/tts* 返回 403
  max_text_length: 5000
  engine:
    pool_size: 1                 # 引擎实例数（CPU 最优 1；多实例并发但内存×N）
    device: "cpu"                # v1 CPU；v2 GPU 走 "coreml"/"cuda"/"vulkan"
    default_model: "kokoro-multi-lang-v1_0"
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
3. `config.yml` 设 `tts.engine.device: "coreml"`（或 `cuda`/`vulkan`），`provider` 对应

kokoro CPU RTF<0.3 已快于实时，多数场景无需 GPU。GPU 仅对高并发或长文本批量合成有收益。

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
