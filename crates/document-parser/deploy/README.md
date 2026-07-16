# document-parser 部署目录

本目录包含 document-parser 在新机器上快速部署所需的参考文件（**不含二进制**）。

## 文件清单

| 文件 | 用途 |
|------|------|
| `systemd/.document-parser.env.example` | OSS 密钥环境变量模板（`service install` 缺省时自动复制） |
| `config/config.example.yml` | 带注释的运维参考（**非**自动创建源；缺 `config.yml` 时由 `AppConfig::default()` 生成） |
| `scripts/setup-venv.sh` | 初始化 Python venv（mineru[core]==3.4.2 + markitdown） |
| `PITFALLS.md` | 踩坑笔记（**必看**） |

> 已废弃：`scripts/install.sh`、外置 `document-parser.service.example` —— 请用 `document-parser service install`。

## 快速部署（Linux）

```bash
# 1. 编译
make build-document-parser-x86_64
# 或 cargo build --release -p document-parser

# 2. 传到目标机
scp target/release/document-parser deploy/ <目标机>:/opt/document-parser/

# 3. Python 环境
cd /opt/document-parser
bash deploy/scripts/setup-venv.sh

# 4. 注册 systemd（缺 config.yml / .env 时自动创建模板）
# 不要用 sudo 跑二进制（内部会对 systemctl 调 sudo；若必须 sudo，会读 SUDO_USER 填 User=）
./document-parser service install --install-dir /opt/document-parser

# 5. 填 OSS 密钥
vim .document-parser.env
./document-parser service restart

# 6. 日志
sudo journalctl -u document-parser -f
```

### 服务器本地编译

```bash
sudo apt-get install -y build-essential cmake pkg-config libssl-dev
cargo build -p document-parser --release
cp target/release/document-parser /opt/document-parser/
cd /opt/document-parser && bash deploy/scripts/setup-venv.sh
./document-parser service install --install-dir /opt/document-parser
```

## systemd 子命令

```bash
document-parser service install  --install-dir <dir> [--user <u>] [--no-start] [--dry-run]
document-parser service uninstall
document-parser service status
document-parser service restart
```

- **默认**：`install` = 写 unit + `enable` + `start`（幂等用 `restart`）
- **`--dry-run`**：只打印 unit（Mac 验证用，不写文件、不调 systemctl）
- **缺 `config.yml`**：从 `AppConfig::default()` 序列化生成
- **缺 `.document-parser.env`**：从 example 复制 + `chmod 600`（密钥未填仅警告）

## mineru 配置要点（config.yml）

| 字段 | 说明 |
|------|------|
| `backend` | `pipeline`（推荐）/ `hybrid-engine` / `vlm-engine` |
| `gpu_memory_utilization` | 与 voice-cli 共存时设 `0.3` |
| `device` | macOS 须显式 `mps` |

> mineru **必须 3.4.2**，见 `PITFALLS.md`。

## 更多

- `../SYSTEMD_SETUP_GUIDE.md` — systemd 原理
- `PITFALLS.md` — 踩坑

## Mac 本地验证

```bash
cargo build -p document-parser
cd crates/document-parser
cargo run -p document-parser -- service install --dry-run --install-dir .
```
