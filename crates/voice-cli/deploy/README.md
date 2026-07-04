# voice-cli 部署目录

不含二进制。二进制按场景单独编译：

| 场景 | 编译命令 | 说明 |
|------|---------|------|
| CPU | `cargo build --release -p voice-cli` | 默认，Docker `make build-voice-cli-x86_64` 也走这条 |
| **CUDA(GPU)** | `cargo build --release -p voice-cli --features cuda` | **需 CUDA Toolkit**，建议在 NVIDIA 服务器上编 |
| macOS Metal | `cargo build --release -p voice-cli --features metal` | Mac 本地 |

## 文件清单

| 文件 | 用途 |
|------|------|
| `server-manager.sh` | 进程管理（改良版：启动前 `cd` + 传 `--config`，修复相对路径落错的 bug） |
| `voice-cli.service` | systemd unit 模板（可选，比 server-manager.sh 更标准） |
| `config.example.yml` | 配置模板（端口 8087、`tts.enabled: false`、删了死字段 `script_path`） |
| `.env.example` | 环境变量模板 |
| `install-libssl1.1-ubuntu2404.sh` | Ubuntu 24.04 装 libssl1.1（cuda 编译版链接 OpenSSL 1.1） |
| `enable-tts.md` | TTS 启用步骤（默认禁用） |

## 快速部署（Ubuntu）

```bash
# 1. 编译（在开发机或目标机）
make build-voice-cli-x86_64                 # CPU 版
# 或目标机本地: cargo build --release -p voice-cli --features cuda   # GPU 版

# 2. 传到目标机（连本目录）
scp -r voice-cli deploy/ <目标机>:/opt/voice-cli/

# 3. Ubuntu 24.04 + cuda 版: 装 libssl1.1
bash deploy/install-libssl1.1-ubuntu2404.sh

# 4. 放配置 + 启动
cd /opt/voice-cli
cp deploy/config.example.yml config.yml     # 按需改端口/模型
./deploy/server-manager.sh start
./deploy/server-manager.sh status
```

## ⚠️ 关键坑（必看）

1. **Ubuntu 24.04 + cuda 编译版必须装 libssl1.1**：24.04 只带 OpenSSL 3，而 cuda 编译版链接了 OpenSSL 1.1，运行报 `error while loading shared libraries: libssl.so.1.1`。跑 `install-libssl1.1-ubuntu2404.sh`。
2. **`server run --config` 位置坑**：`--config` 必须放在 `server run` **后面**（`voice-cli server run --config config.yml`）；全局的 `-c config.yml server run` 在 `server run` 子命令下**会被代码忽略**（见 `src/main.rs:get_config_path_for_server_action`）。`server-manager.sh` 已正确处理。
3. **端口**：由 config.yml 的 `server.port` 决定（默认 8087）。改端口改配置，别在命令行传。
4. **TTS 默认禁用**：缺 `tts_service.py` 不再崩，`/tts/*` 请求返回 503，STT 正常。要 TTS 见 `enable-tts.md`。
5. **WorkingDirectory 必须设对**：`./models` `./logs` `./data/tasks.db` 都是相对路径。`server-manager.sh` 启动前会 `cd $PROJECT_ROOT`；systemd unit 的 `WorkingDirectory=` 也要设（否则落到 `/`）。

## Mac 本地验证

```bash
cd voice-cli
cargo build --release -p voice-cli --features metal
./target/release/voice-cli server run --config config.yml
# 端口 8087，whisper 走 Metal(mps)
```
