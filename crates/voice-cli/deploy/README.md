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

# 5. systemd 注册时传 CUDA 路径（生成 drop-in LD_LIBRARY_PATH）
#    voice-cli service install --install-dir $INSTALL_DIR \
#      --cuda-lib-dir /usr/local/cuda/lib64 --cudnn-lib-dir <cudnn/lib>
```

**复用现成 binary（免重编）**：`voice-cli` + 4 个 `.so` 打包成 `voice-cli-cuda-*.tar`（~940M）传阿里云 OSS，新机 `tar xf` 解到 `$INSTALL_DIR/`（.so 与 binary 同目录）+ `service install` 传 CUDA 路径即可。
> 模型另放：fireredasr2（`models/fireredasr2/...`）+ 标点（`models/punct/...`），拉取见 `scripts/dev/fetch-asr-models.sh`。

## 文件清单

| 文件 | 用途 |
|------|------|
| `config.example.yml` | 带注释的运维参考（**非**自动创建源；缺 `config.yml` 时由 `Config::default()` 生成） |
| `.env.example` | 环境变量说明 |
| `install-libssl1.1-ubuntu2404.sh` | libssl1.1 兜底检测脚本（新架构 rustls 通常不需要） |

## 快速部署（Ubuntu）

```bash
# 1. 编译
make build-voice-cli-x86_64                 # 或 cargo build --release -p voice-cli

# 2. 传到目标机（只需二进制 + 本目录参考文件，不必 scp unit 模板）
scp target/release/voice-cli deploy/ <目标机>:/opt/voice-cli/

# 3. 注册 systemd（缺 config.yml 时自动用代码默认值创建）
cd /opt/voice-cli
./voice-cli service install --install-dir /opt/voice-cli

# 4. STT 模型放 ./models/；TTS（启用时）放 ./models/tts/
```

## 系统服务（systemd）

部署统一走 **`voice-cli service`** 子命令（内置 unit 渲染，无需手动 `sed` / `tee`）。

### 安装（一次性）

```bash
cd /opt/voice-cli   # 须含 voice-cli 二进制；config.yml 可缺省（自动创建）

# 默认：注册 + enable + 立即 start
# 不要用 sudo 跑二进制（内部会对 systemctl 调 sudo；若必须 sudo，会读 SUDO_USER 填 User=）
./voice-cli service install --install-dir /opt/voice-cli

# sherpa CUDA：附加库路径（生成 cuda-sherpa drop-in；两路径按需传，不会臆造默认 CUDA 路径）
./voice-cli service install --install-dir /opt/voice-cli \
  --cuda-lib-dir /usr/local/cuda/lib64 \
  --cudnn-lib-dir /path/to/nvidia/cudnn/lib

# 仅注册、暂不启动（改完 config 再 restart）
./voice-cli service install --install-dir /opt/voice-cli --no-start

# 验证
./voice-cli service status
curl -s http://localhost:8077/health
```

### 常用命令

```bash
./voice-cli service restart      # 改完 config.yml 后
./voice-cli service status       # 状态 + unit + 最近 journal
./voice-cli service uninstall
```

### 日志

```bash
sudo journalctl -u voice-cli -f
# 应用文件日志仍在 ./logs/（按天轮转）
```

## Docker 构建补充（github 阻断环境）

```bash
bash docker/fetch-sherpa.sh amd64
make build-voice-cli-x86_64
```

## ⚠️ 关键坑（必看）

1. **`--install-dir`**：必须是**专用安装根**（含二进制）；勿指向含其他服务 `config.yml` 的目录。
2. **`server run --config` 位置**：`--config` 必须在 `server run` **后面**（unit 已正确配置）。
3. **端口**：由 `config.yml` 的 `server.port` 决定（默认 8077）。
4. **TTS 默认禁用**：启用见 `../docs/DEPLOYMENT.md`。
5. **WorkingDirectory**：`service install` 自动设为 `--install-dir`。
6. **libssl1.1**：通常不需要（rustls）；仅 `ldd voice-cli | grep libssl` 命中时再装。

## Mac 本地验证

```bash
cargo build -p voice-cli
cd crates/voice-cli
cargo run -p voice-cli -- service install --dry-run --install-dir .
# 只打印 unit，不写 /etc、不创建文件
```

## Mac Mini 一键部署（deploy-installer）

见 [mac-mini-quickstart.md](../../deploy-installer/doc/mac-mini-quickstart.md) 与 [voice-cli-roadmap.md](../../deploy-installer/doc/voice-cli-roadmap.md)。
