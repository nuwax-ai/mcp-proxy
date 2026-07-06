# voice-cli 部署目录

二进制按场景单独编译（本目录不含二进制）：

| 场景 | 编译命令 | 说明 |
|------|---------|------|
| Linux CPU | `make build-voice-cli-x86_64`（Docker）或 `cargo build --release -p voice-cli` | 默认，Docker 走 `docker/Dockerfile.voice-cli` |
| **Linux CUDA(GPU)** | `cargo build --release -p voice-cli --features cuda` | **需 CUDA Toolkit**，建议在 NVIDIA 服务器上编 |
| macOS Metal | `cargo build --release -p voice-cli` | Mac 默认带 `whisper-metal`，**无需 flag** |

> `--features` 仅 `cuda` / `vulkan`（见 `Cargo.toml [features]`）。macOS Metal 由 `[target.'cfg(target_os="macos")']` 默认启用，**不存在 metal feature**，写 `--features metal` 会报错。

## 文件清单

| 文件 | 用途 |
|------|------|
| `server-manager.sh` | 进程管理（启动前 `cd` + 传 `--config`，修复相对路径落错的 bug） |
| `voice-cli.service` | systemd unit 模板（可选，比 server-manager.sh 更标准） |
| `config.example.yml` | 配置模板（端口 8087；STT transcribe-rs + TTS sherpa-onnx Kokoro 字段） |
| `.env.example` | 环境变量模板 |
| `install-libssl1.1-ubuntu2404.sh` | libssl1.1 兜底检测脚本（新架构 rustls 通常不需要，见下「关键坑 #5」） |

## 快速部署（Ubuntu）

```bash
# 1. 编译（Docker 跨平台，推荐）
make build-voice-cli-x86_64                 # 产出 dist/voice-cli-x86_64/voice-cli
# 或目标机本地: cargo build --release -p voice-cli --features cuda   # GPU 版

# 2. 传到目标机（连本目录）
scp -r voice-cli deploy/ <目标机>:/opt/voice-cli/

# 3. 放配置 + 模型 + 启动
cd /opt/voice-cli
cp deploy/config.example.yml config.yml     # 按需改端口/模型
# STT 模型放 ./models/；TTS（启用时）放 ./models/tts/（见 ../docs/DEPLOYMENT.md §4）
./deploy/server-manager.sh start
./deploy/server-manager.sh status
```

## 系统服务（systemd，生产推荐）

`voice-cli.service` 是 systemd unit 模板，比 `server-manager.sh` 更标准：开机自启 + 崩溃自动重启 + journald 统一日志。适合生产服务器。

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
curl -s http://localhost:8087/health
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

### 与 server-manager.sh 怎么选

| 场景 | 推荐 |
|---|---|
| 个人机 / 临时测试 | `server-manager.sh`（无需 sudo，nohup 后台） |
| 生产服务器 | `voice-cli.service`（开机自启 + 崩溃重启 + journald，`Type=simple` 不依赖 PID 文件） |

---

## Docker 构建补充（github 阻断环境）

`docker/Dockerfile.voice-cli` 编译期需 sherpa-onnx 预编译 C 库（无 `SHERPA_ONNX_ARCHIVE_DIR` 缓存时默认联网下载，github 阻断会卡死）。**先预下载**：

```bash
bash docker/fetch-sherpa.sh amd64          # gh-proxy 拉 linux x64 tar 到 docker/sherpa-cache/
make build-voice-cli-x86_64                # build.rs 命中本地 tar，跳过联网
```

有网环境（CI / 公网服务器）可跳过预下载，容器内自动联网。

## ⚠️ 关键坑（必看）

1. **`server run --config` 位置坑**：`--config` 必须放在 `server run` **后面**（`voice-cli server run --config config.yml`）；全局的 `-c config.yml server run` 在 `server run` 子命令下**会被代码忽略**（见 `src/main.rs:get_config_path_for_server_action`）。`server-manager.sh` 已正确处理。
2. **端口**：由 config.yml 的 `server.port` 决定（本模板默认 8087；`../docs/DEPLOYMENT.md` 示例用 8080，按需统一）。改端口改配置，别在命令行传。
3. **TTS 默认禁用**：sherpa-onnx Kokoro（CPU v1）。启用见 `../docs/DEPLOYMENT.md`（置 `tts.enabled: true` + 放 Kokoro 模型到 `./models/tts/kokoro-multi-lang-v1_0/`，注意 lexicon 组合：us-en + zh，不含 gb-en）。
4. **WorkingDirectory 必须设对**：`./models` `./logs` `./data/tasks.db` 都是相对路径。`server-manager.sh` 启动前会 `cd $PROJECT_ROOT`；systemd unit 的 `WorkingDirectory=` 也要设（否则落到 `/`）。
5. **libssl1.1（可选兜底）**：新架构 reqwest 已用 rustls（`Cargo.toml` reqwest 段注释明确），产物**不依赖任何 libssl.so**，通常无需 `install-libssl1.1-ubuntu2404.sh`。仅当 `ldd voice-cli | grep libssl` 命中时（cuda 编译 + 特定链接场景）才跑该脚本——它是检测性的，命中才装。

## Mac 本地验证

```bash
cargo build --release -p voice-cli          # 默认带 whisper-metal
./target/release/voice-cli server run --config config.yml
# 端口 8087，whisper 走 Metal(mps)
```
