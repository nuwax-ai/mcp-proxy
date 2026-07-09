# document-parser 部署目录

本目录包含 document-parser 在新机器上快速部署所需的全部文件（**不含二进制**，二进制走 `make build` 或 `cargo build` 单独产出）。

## 文件清单

| 文件 | 用途 |
|------|------|
| `systemd/document-parser.service.example` | systemd unit 模板（含占位符） |
| `systemd/.document-parser.env.example` | OSS 密钥环境变量模板 |
| `config/config.example.yml` | 配置文件模板（mineru 段带注释） |
| `scripts/setup-venv.sh` | 初始化 Python venv（装 mineru[core]==3.4.2 + markitdown + huggingface-hub<1.0） |
| `scripts/install.sh` | 一键部署（建密钥模板 + 装 unit + enable） |
| `PITFALLS.md` | 踩坑笔记（**必看**） |

## 快速部署（Linux）

```bash
# 1. 开发机编译二进制
make build-document-parser-x86_64

# 2. 传到目标机（连本目录一起）
scp -r dist/document-parser-x86_64/document-parser deploy/ <目标机>:/opt/document-parser/

# 3. 目标机：初始化 Python 环境 + 一键部署
cd /opt/document-parser
bash deploy/scripts/setup-venv.sh          # 装 mineru 3.4.2 等
bash deploy/scripts/install.sh             # 装 systemd unit + enable

# 4. 填 OSS 密钥
vim .document-parser.env

# 5. 启动
sudo systemctl start document-parser
sudo journalctl -u document-parser -f
```

### 服务器本地编译（Linux CUDA，可不走 Docker buildx）

目标机已装 rust/CUDA 时，直接在服务器上 `cargo build`（CPU 二进制，CUDA 由 venv 里 torch 提供），省去开发机交叉编译+上传：

```bash
# 1. 系统编译依赖（关键，缺了会卡 openssl-sys/cmake/stdbool.h，见 PITFALLS #10）
sudo apt-get install -y build-essential cmake pkg-config libssl-dev
# CUDA（mineru pipeline + device cuda 走 torch GPU，不需系统 CUDA toolkit；
#       但若同机编 voice-cli --features cuda，需装 cuda-toolkit-12-6）

# 2. rust（国内用 rsproxy 镜像，否则 static.rust-lang.org 龟速，见 PITFALLS #12）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
  RUSTUP_DIST_SERVER=https://rsproxy.cn sh -s -- -y --profile minimal

# 3. 编译 + 部署
cd /path/to/mcp-proxy && cargo build -p document-parser --release
cp target/release/document-parser /opt/document-parser/
cd /opt/document-parser
UV_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple bash deploy/scripts/setup-venv.sh
bash deploy/scripts/install.sh
```

> mineru 后端选 `pipeline`（无 vllm，仍走 cuda OCR/公式/表格，稳定）；与 voice-cli 共用 GPU 时设 `gpu_memory_utilization: 0.3` 防 OOM（详见 PITFALLS #3）。

## mineru 配置要点（config.yml）

| 字段 | 说明 |
|------|------|
| `backend` | `pipeline`（CPU/兼容）/ `hybrid-engine`（GPU+vllm，默认）/ `vlm-engine`（纯 VLM） |
| `vram` | `0`=不限（通过 `MINERU_VIRTUAL_VRAM_SIZE` 注入 mineru） |
| `gpu_memory_utilization` | 与 voice-cli 等 GPU 进程共存时设 `0.3` 避免 OOM；`0`=用 mineru 默认（约 0.5） |
| `device` | Linux+NVIDIA 自动 `cpu→cuda`；**macOS 必须显式 `device: mps`**（MPS 不自动检测，否则跑 CPU）；多 GPU 用 `cuda:N` |

> ⚠️ mineru 3.4.0 有 PageChars bug，**必须 3.4.2**（`setup-venv.sh` 已锁版本）。详见 `PITFALLS.md`。

## 更多

- 原理与排查：`../SYSTEMD_SETUP_GUIDE.md`
- 踩坑：`PITFALLS.md`
