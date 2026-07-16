# voice-cli 部署目录

二进制按场景单独编译（本目录不含二进制）：

| 场景 | 编译命令 | 说明 |
|------|---------|------|
| Linux CPU | `make build-voice-cli-x86_64`（Docker）或 `cargo build --release -p voice-cli` | 默认，Docker 走 `docker/Dockerfile.voice-cli` |
| **Linux CUDA(GPU)** | `cargo build --release -p voice-cli --features cuda` | **需 CUDA Toolkit**，建议在 NVIDIA 服务器上编 |
| macOS Metal | `cargo build --release -p voice-cli` | Mac 默认带 `whisper-metal`，**无需 flag** |

> `--features` 仅 `cuda` / `vulkan`（见 `Cargo.toml [features]`）。macOS Metal 由 `[target.'cfg(target_os="macos")']` 默认启用，**不存在 metal feature**，写 `--features metal` 会报错。

### Linux CUDA 服务器编译前置依赖

`--features cuda` 编译 whisper.cpp 的 CUDA kernel，编译期需 nvcc（CUDA Toolkit），**Docker buildx 的 `rust:1.92` 基础镜像无 nvcc**，故 CUDA 版必须在 NVIDIA 服务器本地编。前置：

```bash
# 1. 系统编译依赖（whisper-cuda 经 cmake 编译，缺了报 cmake not found / stdbool.h not found）
sudo apt-get install -y build-essential cmake pkg-config ffmpeg
# 2. CUDA Toolkit 12.x（驱动 535+ 即可；nvcc 给 build.rs 编 CUDA kernel 用）
wget https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2404/x86_64/cuda-keyring_1.1-1_all.deb
sudo dpkg -i cuda-keyring_1.1-1_all.deb && sudo apt-get update
sudo apt-get install -y cuda-toolkit-12-6
export PATH=/usr/local/cuda-12.6/bin:$PATH CUDA_HOME=/usr/local/cuda-12.6

# 3. rust（国内用 rsproxy 镜像，否则 static.rust-lang.org 龟速）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
  RUSTUP_DIST_SERVER=https://rsproxy.cn sh -s -- -y --profile minimal

# 4. 预缓存 sherpa-onnx C 库（github 阻断时 build.rs 联网下载会卡死）
bash docker/fetch-sherpa.sh amd64          # → ~/.cache/sherpa-onnx-prebuilt/（自动探活镜像）

# 5. 编译
export SHERPA_ONNX_ARCHIVE_DIR=$HOME/.cache/sherpa-onnx-prebuilt
cargo build --release -p voice-cli --features cuda
```
> 编译期 `utoipa-swagger-ui` build.rs 会在线下载 Swagger UI（网络受限可能 curl 56 失败，重试即可命中缓存）。

### sherpa CUDA 后端（fireredasr2 / Fun-ASR-Nano / Qwen3-ASR）部署

上面 `--features cuda` + `SHERPA_ONNX_ARCHIVE_DIR` 编的是 **whisper CUDA + sherpa 静态 CPU**（sherpa 预编译 static 包无 CUDA EP，同 Mac static 无 CoreML）。要 sherpa 三引擎也走 GPU（A6000），用 sherpa 官方 **CUDA 预编译包**（shared .so，含 `libonnxruntime_providers_cuda.so`）：

```bash
# 1. 拉 sherpa CUDA 预编译包（CUDA 12.x + cuDNN 9.x；钉 v1.13.3，1.13.4 的 ort 1.27.0 启动 SIGKILL）
curl -fL -o /tmp/sherpa-cuda.tar.bz2 \
  https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.3/sherpa-onnx-v1.13.3-cuda-12.x-cudnn-9.x-linux-x64-gpu.tar.bz2
mkdir -p ~/sherpa-cuda && tar xjf /tmp/sherpa-cuda.tar.bz2 -C ~/sherpa-cuda
CUDA_LIB=~/sherpa-cuda/sherpa-onnx-v1.13.3-cuda-12.x-cudnn-9.x-linux-x64-gpu/lib

# 2. 编译（SHERPA_ONNX_LIB_DIR 指向 CUDA 包，而非静态缓存；必须设 CUDACXX 否则 whisper.cpp cmake 报 No CMAKE_CUDA_COMPILER）
export PATH=/usr/local/cuda/bin:$PATH CUDACXX=/usr/local/cuda/bin/nvcc
SHERPA_ONNX_LIB_DIR=$CUDA_LIB cargo build --release -p voice-cli --features cuda

# 3. 部署：binary + 4 个 .so 放同目录（binary rpath=$ORIGIN 找同目录 .so）
INSTALL_DIR=/home/$USER/workspace/voice-server
cp target/release/voice-cli $CUDA_LIB/libsherpa-onnx-c-api.so $CUDA_LIB/libonnxruntime.so \
   $CUDA_LIB/libonnxruntime_providers_cuda.so $CUDA_LIB/libonnxruntime_providers_shared.so $INSTALL_DIR/

# 4. cuDNN 9.x（libonnxruntime_providers_cuda.so 依赖；二选一）
pip install nvidia-cudnn-cu12    # → .../site-packages/nvidia/cudnn/lib/libcudnn.so.9
# 或借用同机 document-parser venv 的 cuDNN（LD_LIBRARY_PATH 指过去）

# 5. systemd voice-cli.service 取消注释 LD_LIBRARY_PATH 行，填 cuDNN 路径 + daemon-reload + enable
#    Environment=LD_LIBRARY_PATH=<INSTALL_DIR>:<cudnn/lib>:/usr/local/cuda/lib64
```

**复用现成 binary（免重编）**：`voice-cli` + 4 个 `.so` 打包成 `voice-cli-cuda-*.tar`（~940M）传阿里云 OSS，新机 `tar xf` 解到 `$INSTALL_DIR/`（.so 与 binary 同目录）+ 配 `LD_LIBRARY_PATH`（cuDNN + cuda）+ cuDNN 9.x / CUDA toolkit 12.x 即可。
> 模型另放：fireredasr2（`models/fireredasr2/...`）+ 标点（`models/punct/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12/model.onnx`），拉取见 `scripts/dev/fetch-asr-models.sh`。

## 文件清单

| 文件 | 用途 |
|------|------|
| `voice-cli.service` | systemd unit 模板（开机自启 + 崩溃重启 + journald，部署统一走 systemd） |
| `config.example.yml` | 配置模板（端口 8077；STT transcribe-rs + TTS Kokoro/ZipVoice 双引擎字段） |
| `.env.example` | 环境变量模板 |
| `install-libssl1.1-ubuntu2404.sh` | libssl1.1 兜底检测脚本（新架构 rustls 通常不需要，见下「关键坑 #5」） |

## 快速部署（Ubuntu）

```bash
# 1. 编译（Docker 跨平台，推荐）
make build-voice-cli-x86_64                 # 产出 dist/voice-cli-x86_64/voice-cli
# 或目标机本地: cargo build --release -p voice-cli --features cuda   # GPU 版

# 2. 传到目标机（连本目录）
scp -r voice-cli deploy/ <目标机>:/opt/voice-cli/

# 3. 放配置 + 模型
cd /opt/voice-cli
cp deploy/config.example.yml config.yml     # 按需改端口/模型
# STT 模型放 ./models/；TTS（启用时）放 ./models/tts/（见 ../docs/DEPLOYMENT.md §4）

# 4. 注册 systemd 服务（开机自启 + 崩溃重启）—— 命令见下方「系统服务」章节
```

## 系统服务（systemd）

部署统一走 systemd：开机自启 + 崩溃自动重启 + journald 统一日志。`voice-cli.service` 是 unit 模板。

### 安装（一次性）

```bash
# 1. 替换占位符 __USER__ / __GROUP__ / __INSTALL_DIR__ 并安装到系统目录
sudo sed -e "s|__USER__|$USER|g" \
         -e "s|__GROUP__|$USER|g" \
         -e "s|__INSTALL_DIR__|/opt/voice-cli|g" \
         deploy/voice-cli.service \
         | sudo tee /etc/systemd/system/voice-cli.service > /dev/null

# 2. 重载 systemd + 开机自启 + 立即启动
sudo systemctl daemon-reload
sudo systemctl enable --now voice-cli

# 3. 验证
sudo systemctl status voice-cli
curl -s http://localhost:8077/health
```

> `__INSTALL_DIR__`（默认 `/opt/voice-cli`）须含 `voice-cli` 二进制 + `config.yml` + `models/`（先完成上方"快速部署"步骤 2-3）。`User=` 决定运行用户，确保该用户对 `__INSTALL_DIR__` 有读写权限（运行时要写 `./logs/`、`./data/`）。

### 常用命令

```bash
sudo systemctl start voice-cli       # 启动
sudo systemctl stop voice-cli        # 停止
sudo systemctl restart voice-cli     # 重启（改完 config.yml 后用它生效）
sudo systemctl status voice-cli      # 状态 + 最近日志
sudo systemctl disable voice-cli     # 取消开机自启
```

### 日志查询（journalctl）

systemd 走 journald 统一收集；应用内 `tracing-appender` 文件日志仍在 `./logs/`（按天轮转），两者并存。

```bash
sudo journalctl -u voice-cli -f                       # 实时跟踪
sudo journalctl -u voice-cli --since "10 min ago"     # 最近 10 分钟
sudo journalctl -u voice-cli -n 200                   # 最近 200 行
```

---

## Docker 构建补充（github 阻断环境）

`docker/Dockerfile.voice-cli` 编译期需 sherpa-onnx 预编译 C 库（无 `SHERPA_ONNX_ARCHIVE_DIR` 缓存时默认联网下载，github 阻断会卡死）。**先预下载**：

```bash
bash docker/fetch-sherpa.sh amd64          # gh-proxy 拉 linux x64 tar 到 docker/sherpa-cache/
make build-voice-cli-x86_64                # build.rs 命中本地 tar，跳过联网
```

有网环境（CI / 公网服务器）可跳过预下载，容器内自动联网。

## ⚠️ 关键坑（必看）

1. **`server run --config` 位置坑**：`--config` 必须放在 `server run` **后面**（`voice-cli server run --config config.yml`）；全局的 `-c config.yml server run` 在 `server run` 子命令下**会被代码忽略**（见 `src/main.rs:get_config_path_for_server_action`）。下方 systemd unit 的 `ExecStart` 已正确放置（`--config` 在 `server run` 后）。
2. **端口**：由 config.yml 的 `server.port` 决定（本模板默认 8077；`../docs/DEPLOYMENT.md` 示例用 8080，按需统一）。改端口改配置，别在命令行传。
3. **TTS 默认禁用**：sherpa-onnx（Kokoro v1_1 标准 / ZipVoice 克隆）。启用见 `../docs/DEPLOYMENT.md` §4.2（置 `tts.enabled: true` + 放模型到 `./models/tts/`；Kokoro 默认，ZipVoice 改 `backend: zipvoice`，参考 `config.example.yml`）。
4. **WorkingDirectory 必须设对**：`./models` `./logs` `./data/tasks.db` 都是相对路径。systemd unit 的 `WorkingDirectory=` 必须指向安装根（否则相对路径落到 `/`）。
5. **libssl1.1（可选兜底）**：新架构 reqwest 已用 rustls（`Cargo.toml` reqwest 段注释明确），产物**不依赖任何 libssl.so**，通常无需 `install-libssl1.1-ubuntu2404.sh`。仅当 `ldd voice-cli | grep libssl` 命中时（cuda 编译 + 特定链接场景）才跑该脚本——它是检测性的，命中才装。

## Mac 本地验证

```bash
cargo build --release -p voice-cli          # 默认带 whisper-metal
./target/release/voice-cli server run --config config.yml
# 端口 8077，whisper 走 Metal(mps)
```
