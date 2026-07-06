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
