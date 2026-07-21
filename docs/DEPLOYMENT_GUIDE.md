# 部署指南 (Deployment Guide)

> voice-cli + document-parser 生产部署操作手册 —— 基于 deploy-installer（原 systemd-installer）`service install` 方式
> 配套设计文档:[SYSTEMD_SERVICE_INSTALLER_DESIGN.md](./SYSTEMD_SERVICE_INSTALLER_DESIGN.md)
> 适用:Ubuntu / systemd / NVIDIA CUDA 服务器

---

## 0. 服务概览

| 服务 | 默认端口 | 编译方式 | 部署目录(示例) |
|---|---|---|---|
| **voice-cli** | 8077 | `--features cuda`(CUDA + sherpa) | `/home/<user>/workspace/voice-server` |
| **document-parser** | 8087 | CPU | `/home/<user>/workspace/document-server` |

两服务统一 **systemd 管理**(开机自启 + 崩溃重启 + journald),部署用各自二进制的 `service install` 子命令(**unit 模板内嵌进二进制**,无需手动 sed/tee)。

> **端口由来**:voice-cli 8077 / document-parser 8087,两服务同机部署避免冲突。改端口改各自 `config.yml` 的 `server.port`。

---

## 1. 前置条件

### 编译环境
```bash
# 1. rust(国内用 rsproxy 镜像)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | RUSTUP_DIST_SERVER=https://rsproxy.cn sh -s -- -y --profile minimal
# 2. 系统依赖
sudo apt-get install -y build-essential cmake pkg-config ffmpeg
# 3. voice-cli CUDA:CUDA Toolkit 12.x(nvcc)
sudo apt-get install -y cuda-toolkit-12-6
export PATH=/usr/local/cuda/bin:$PATH
```

### 源码
```bash
git clone <repo-url> mcp-proxy && cd mcp-proxy   # test 分支(含 deploy-installer)
```

> 详细编译依赖(sherpa 缓存、Swagger UI 等)见 [`crates/voice-cli/deploy/README.md`](../crates/voice-cli/deploy/README.md)。

---

## 2. voice-cli 部署(CUDA)

### 2.1 编译(whisper CUDA + sherpa CUDA EP)
```bash
# sherpa CUDA shared 包(钉 v1.13.3;含 libonnxruntime_providers_cuda.so)
curl -fL -o /tmp/sherpa-cuda.tar.bz2 \
  https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.3/sherpa-onnx-v1.13.3-cuda-12.x-cudnn-9.x-linux-x64-gpu.tar.bz2
mkdir -p ~/sherpa-cuda && tar xjf /tmp/sherpa-cuda.tar.bz2 -C ~/sherpa-cuda
CUDA_LIB=~/sherpa-cuda/sherpa-onnx-v1.13.3-cuda-12.x-cudnn-9.x-linux-x64-gpu/lib

export PATH=/usr/local/cuda/bin:$PATH CUDACXX=/usr/local/cuda/bin/nvcc
SHERPA_ONNX_LIB_DIR=$CUDA_LIB cargo build --release -p voice-cli --features cuda
```

### 2.2 放置文件
```bash
INSTALL_DIR=/home/$USER/workspace/voice-server
mkdir -p $INSTALL_DIR/models/tts

# binary + 4 个 sherpa .so 放同目录(binary rpath=$ORIGIN 找同目录 .so)
cp target/release/voice-cli \
   $CUDA_LIB/libsherpa-onnx-c-api.so $CUDA_LIB/libonnxruntime.so \
   $CUDA_LIB/libonnxruntime_providers_cuda.so $CUDA_LIB/libonnxruntime_providers_shared.so \
   $INSTALL_DIR/

# config.yml(从代码默认生成,或从 deploy/config.example.yml 复制后改)
cp crates/voice-cli/deploy/config.example.yml $INSTALL_DIR/config.yml

# 模型(见 crates/voice-cli/docs/DEPLOYMENT.md):
#   STT: models/large-v3 等;TTS: models/tts/kokoro-multi-lang-v1_1/、zipvoice-.../
```

### 2.3 注册 systemd(新方式)
```bash
cd $INSTALL_DIR

# cuDNN 路径:二选一
#   (a) 借用同机 document-parser venv:../document-server/venv/.../nvidia/cudnn/lib
#       ⚠️ 需先部署 document-parser(§3)建好 venv;否则先做 §3,或用方案 (b)
#   (b) pip install nvidia-cudnn-cu12 → .../site-packages/nvidia/cudnn/lib(独立,不依赖 document-parser)
# ⚠️ sudo 非 NOPASSWD 时:用 `echo <pass> | sudo -S` 包裹下行命令(见 §6 #2)
./voice-cli service install --install-dir $INSTALL_DIR \
  --cuda-lib-dir /usr/local/cuda/lib64 \
  --cudnn-lib-dir /home/$USER/workspace/document-server/venv/lib/python3.12/site-packages/nvidia/cudnn/lib
# → 预检(binary/config/端口/sudo)→ 写 unit + cuda-sherpa drop-in → enable + start
```

### 2.4 验证
```bash
./voice-cli service status                                   # is-enabled/is-active + unit + 日志
curl -s http://localhost:8077/health                         # 200 healthy
curl -s http://localhost:8077/api/v1/tts/voices | grep -o num_speakers\":[0-9]*   # 103
```

---

## 3. document-parser 部署(CPU)

### 3.1 编译 + Python venv
```bash
cargo build --release -p document-parser

cd crates/document-parser
cargo run --bin document-parser -- uv-init    # 建 ./venv,装 mineru + markitdown
cargo run --bin document-parser -- check      # 验证 venv
# backend 选择(默认 pipeline 快 ~30s;hybrid-engine 是 VLM 后端,复杂版式/扫描件质量更好但慢 ~60s):
#   要用 hybrid-engine → 先 sudo apt-get install -y python3.12-dev(triton 编译 GPU kernel 要 Python.h)
#   再 config.yml 改 backend: hybrid-engine + service restart
```

### 3.2 放置文件
```bash
INSTALL_DIR=/home/$USER/workspace/document-server
mkdir -p $INSTALL_DIR

cp target/release/document-parser $INSTALL_DIR/
cp -r crates/document-parser/venv $INSTALL_DIR/        # venv(markitdown/mineru)
cp crates/document-parser/deploy/config/config.example.yml $INSTALL_DIR/config.yml
# 编辑 config.yml:storage.oss 的 public_bucket/private_bucket(必填,空则启动 validate 失败)

# OSS 密钥(不放 config.yml,放 .env):
cp crates/document-parser/deploy/systemd/.document-parser.env.example $INSTALL_DIR/.document-parser.env
chmod 600 $INSTALL_DIR/.document-parser.env
vim $INSTALL_DIR/.document-parser.env                  # 填 OSS_ACCESS_KEY_ID / OSS_ACCESS_KEY_SECRET
```

### 3.3 注册 systemd(新方式)
```bash
cd $INSTALL_DIR
# ⚠️ sudo 非 NOPASSWD 时:用 `echo <pass> | sudo -S` 包裹下行命令(见 §6 #2)
./document-parser service install --install-dir $INSTALL_DIR
# → 预检(含 .document-parser.env 必须存在)→ 写 unit(ExecStart 含 --config 让 OSS 密钥 env 生效)→ enable + start
```

### 3.4 验证
```bash
./document-parser service status
curl -s http://localhost:8087/health
curl -s http://localhost:8087/api/v1/documents/parser/health   # markitdown/mineru available
```
> **首次解析 PDF**:mineru 会自动从 modelscope 下载模型(PDF-Extract-Kit ~1GB;hybrid-engine 再加 MinerU2.5-Pro),`MINERU_MODEL_SOURCE=modelscope` 已注入。首次慢属正常,后续走缓存。

---

## 4. 服务管理(日常)

```bash
<bin> service status       # 状态 + unit 内容 + 最近 journal
<bin> service restart      # 重启(改完 config.yml 后用)
<bin> service uninstall    # 卸载(stop + disable + 删 unit/drop-in + daemon-reload)

sudo journalctl -u voice-cli -f                # 实时日志(journald)
sudo journalctl -u document-parser --since "10 min ago"
```

> 默认 `service install` = 注册 + enable(开机自启) + start(立即启动)。`--no-start` 仅注册不自启;`--dry-run` 只渲染 unit 不写(Mac/预览用)。

---

## 5. 升级 / 回滚

### 升级(替换运行中二进制)
```bash
INSTALL_DIR=<部署目录>; BIN=<voice-cli|document-parser>
# 1. 备份(可回滚)
TS=$(date +%Y%m%d_%H%M%S)
cp $INSTALL_DIR/$BIN $INSTALL_DIR/$BIN.bak.$TS
# 2. 替换运行中 binary 必须先 stop(否则 cp 报 "Text file busy")
echo <sudo_pass> | sudo -S systemctl stop $BIN
cp target/release/$BIN $INSTALL_DIR/$BIN
# 3. 重新注册 unit(幂等;voice-cli 带 cuda 参数)
echo <sudo_pass> | sudo -S $INSTALL_DIR/$BIN service install --install-dir $INSTALL_DIR [voice-cli: --cuda-lib-dir ... --cudnn-lib-dir ...]
```

### 回滚(新版有问题)
```bash
echo <sudo_pass> | sudo -S systemctl stop $BIN
cp $INSTALL_DIR/$BIN.bak.<ts> $INSTALL_DIR/$BIN
echo <sudo_pass> | sudo -S systemctl start $BIN
# (若连旧 unit 也要恢复:从 ~/处的 *.service.bak 拷回 /etc/systemd/system/ + daemon-reload)
```

---

## 6. 踩坑速查

| # | 问题 | 解决 |
|---|---|---|
| 1 | `cp: Text file busy` | 替换运行中 binary 前先 `systemctl stop <name>` |
| 2 | sudo 非 NOPASSWD,`service install` 卡密码 | `echo <pass> \| sudo -S <bin> service install ...` 以 root 跑(`SUDO_USER` → `User=` 当前用户) |
| 3 | voice-cli CUDA 启动找不到 cuDNN | `--cudnn-lib-dir` 指向 cuDNN 9.x lib(可借 document-parser venv 的 `nvidia/cudnn/lib`) |
| 4 | 端口冲突硬阻断 | 改 `config.yml` 的 `server.port` 重试(预检会提示占用 pid) |
| 5 | document-parser OSS 密钥不生效 | 确认走 `--config` 分支(新 unit 已带);密钥在 `.document-parser.env`(EnvironmentFile),非 config.yml |
| 6 | 不确定 unit 是否正确 | `<bin> service install --dry-run --install-dir <dir>` 先看渲染结果 |
| 7 | 跨服务依赖 cuDNN | document-parser venv 重建若改了 cudnn 路径,voice-cli 的 `--cudnn-lib-dir` 要同步更新 |
| 8 | `backend: hybrid-engine` 报 `fatal error: Python.h: No such file` | triton 运行时编译 GPU kernel 缺 Python.h → `sudo apt-get install -y python3.12-dev`(默认 pipeline 不需要;hybrid-engine VLM 质量好但慢 ~60s/文档,VLM 模型每次加载) |

---

## 7. 相关文档

- voice-cli 编译/模型/CUDA/Vulkan 细则:[`crates/voice-cli/deploy/README.md`](../crates/voice-cli/deploy/README.md)、[`crates/voice-cli/docs/DEPLOYMENT.md`](../crates/voice-cli/docs/DEPLOYMENT.md)
- document-parser venv/PITFALLS:[`crates/document-parser/deploy/README.md`](../crates/document-parser/deploy/README.md)、[`PITFALLS.md`](../crates/document-parser/deploy/PITFALLS.md)
- deploy-installer 设计(架构/ServiceSpec/决策):[`SYSTEMD_SERVICE_INSTALLER_DESIGN.md`](./SYSTEMD_SERVICE_INSTALLER_DESIGN.md)（历史文档名）
- Mac Mini npm 部署:[`crates/deploy-installer/doc/mac-mini-quickstart.md`](../crates/deploy-installer/doc/mac-mini-quickstart.md)
