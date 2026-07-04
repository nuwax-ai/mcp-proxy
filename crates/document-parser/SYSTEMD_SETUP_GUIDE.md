# document-parser systemd 服务部署与运维指南

## 概述

本指南说明如何把 `document-parser` 配置成 **systemd 系统服务**，实现：

- 开机自启、崩溃自动重启
- 统一的日志管理（journald）
- 用独立的环境变量文件管理 OSS 密钥等敏感信息，不泄露在命令行或全局配置里
- 通过 `systemctl` 标准化地启停、查看状态

适用于 Ubuntu 20.04+ / Debian 12+ 等使用 systemd 的 Linux 发行版。

---

## 前置条件

部署目录下需要准备好以下内容（下文以 `INSTALL_DIR=/home/swufe/workspace/document-parser` 为例，按需修改）：

| 文件/目录 | 说明 |
|-----------|------|
| `document-parser` | 已编译好的二进制（`cargo build --release` 产物，或 `make build-document-parser-x86_64` 产出的 Linux 二进制） |
| `config.yml` | 服务配置文件，含端口、解析参数、OSS bucket 等 |
| `venv/` | Python 虚拟环境（MinerU / MarkItDown 依赖）。`config.yml` 里 `python_path: ./venv/bin/python` 是**相对路径** |
| `logs/`、`data/` | 运行时日志与本地数据目录（不存在会被自动创建） |

快速自检：

```bash
cd /home/swufe/workspace/document-parser

# 二进制存在且可执行
ls -l document-parser && file document-parser
# 验证可以运行(打印帮助)
./document-parser --help

# 配置文件存在
ls -l config.yml

# venv 存在(python_path 是相对路径, 所以 WorkingDirectory 必须设对)
ls venv/bin/python

# 端口(默认 8087)未被占用
ss -ltn | grep :8087 || echo "8087 空闲"
```

> ⚠️ **路径建议**：尽量把部署目录放在**稳定的挂载点**下（如根分区下的 `/opt/...` 或 `/home/<user>/...`）。避免放在可能有挂载问题的独立分区上 —— 详见后文「常见问题 → dependency failed」。

---

## ⚠️ mineru 3.4 适配注意（新部署必看）

document-parser 已适配 mineru 3.4.x。新部署注意以下几点（详细踩坑见 `deploy/PITFALLS.md`）：

1. **mineru 锁 3.4.2**：3.4.0 有 PageChars bug（任务失败 `TypeError: 'PageChars' object is not iterable`），必须 3.4.2。
   ```bash
   uv pip install "mineru[core]==3.4.2" --python ./venv/bin/python --index-url https://pypi.org/simple
   ```
2. **huggingface-hub<1.0**：mineru 3.4.2 要 huggingface-hub<1.0，但升级会拉进 1.22，要降级。
   ```bash
   uv pip install "huggingface-hub>=0.34,<1.0" --python ./venv/bin/python
   ```
   （推荐直接用 `deploy/scripts/setup-venv.sh`，已锁版本 + 处理冲突）
3. **venv 用系统 Python 3.12**（避开 anaconda 3.7 污染）：`uv venv --python /usr/bin/python3 ./venv`
4. **backend 改名**：旧 `vlm-transformers`/`vlm-sglang-engine`/`vlm-sglang-client` 已废，新值 `pipeline`/`vlm-engine`/`hybrid-engine`/`vlm-http-client`/`hybrid-http-client`。config.yml 默认 `hybrid-engine`。
5. **CLI 参数变更**：mineru 3.4 不再支持 `-d`/`--vram`/`--source`（document-parser 已改用环境变量 `MINERU_DEVICE_MODE`/`MINERU_VIRTUAL_VRAM_SIZE`/`MINERU_MODEL_SOURCE`，用户无需手动设）。
6. **GPU 共存 OOM**：与 voice-cli 等 GPU 进程共存时，config.yml 设 `gpu_memory_utilization: 0.3`（让 vllm 少占显存）。
7. **hybrid-engine + vllm 冲突**：vllm 要 huggingface-hub≥1.0、mineru 要 <1.0，互斥。如撞 `Please install vllm`，改用 `pipeline` backend（不用 vllm，仍 cuda 加速 OCR/公式）。

---

## 第一步：准备环境变量（密钥）文件

`config.yml` 里 OSS 密钥写的是占位符（`${OSS_ACCESS_KEY_ID}`），实际值通过环境变量注入。代码里读的是裸变量名（`document-parser/src/config.rs` 的 `load_oss_config_from_env`）：

```yaml
oss:
  access_key_id: "${OSS_ACCESS_KEY_ID}"      # 必须用环境变量覆盖
  access_key_secret: "${OSS_ACCESS_KEY_SECRET}"
```

如果这两个变量没注入，程序拿到的就是字面量 `${OSS_ACCESS_KEY_ID}`，OSS 调用必然失败。

### 1.1 创建密钥文件

在部署目录下创建 `.document-parser.env`：

```bash
cd /home/swufe/workspace/document-parser

# 用编辑器写入(或用下面的 heredoc)
cat > .document-parser.env <<'EOF'
OSS_ACCESS_KEY_ID=你的AccessKeyId
OSS_ACCESS_KEY_SECRET=你的AccessKeySecret
EOF
```

### 1.2 ⚠️ 格式要点：systemd 不支持 `export`

systemd 的 `EnvironmentFile` 解析的是 `.env` 格式（`KEY=value`，每行一个），**不执行 shell 语法**。下面两种写法的区别很重要：

```bash
# ❌ 错误：bash 风格, systemd 不会解析, 密钥注入失败
export OSS_ACCESS_KEY_ID='xxxx'
export OSS_ACCESS_KEY_SECRET='xxxx'

# ✅ 正确：纯 KEY=value, systemd 能解析
OSS_ACCESS_KEY_ID=xxxx
OSS_ACCESS_KEY_SECRET=xxxx
```

> 如果你希望同一个文件既能被 systemd 加载、又能用 `source` 手动调试，写成无 `export` 的纯 `KEY=value`，手动调试时用 `set -a; source .document-parser.env; set +a`（`set -a` 让赋值自动 export）。

值里有特殊字符（`$`、空格等）建议用**单引号**包裹：`KEY='value'`。systemd 会正确剥离单引号。

### 1.3 收紧权限

密钥文件必须只允许运行用户读取：

```bash
chmod 600 .document-parser.env
chown swufe:swufe .document-parser.env   # 换成实际运行用户
ls -l .document-parser.env
# -rw------- 1 swufe swufe ... .document-parser.env
```

### 1.4 把文件加入 .gitignore

```bash
# 部署目录如果在 git 仓库内, 务必忽略密钥文件
grep -qxF '.document-parser.env' .gitignore || echo '.document-parser.env' >> .gitignore
```

---

## 第二步：编写 systemd 服务单元

### 2.1 完整 unit 模板

把下面的内容存成 `document-parser.service`（先放本地，下一步安装到系统目录）：

```ini
[Unit]
Description=Document Parser Service (MCP document-parser)
Documentation=https://github.com/xxx/mcp-proxy
After=network.target

[Service]
Type=simple
User=swufe
Group=swufe
WorkingDirectory=/home/swufe/workspace/document-parser
EnvironmentFile=/home/swufe/workspace/document-parser/.document-parser.env
ExecStart=/home/swufe/workspace/document-parser/document-parser server
Restart=on-failure
RestartSec=5s
KillSignal=SIGINT
TimeoutStopSec=60
StandardOutput=journal
StandardError=journal
SyslogIdentifier=document-parser

[Install]
WantedBy=multi-user.target
```

### 2.2 关键字段说明

| 字段 | 说明 |
|------|------|
| `Type=simple` | 主进程就是服务进程，systemd 认为 `ExecStart` fork 出的进程即服务 |
| `User` / `Group` | 以非 root 用户运行（监听 8087 不需要特权，且能直接读写自己的工作目录） |
| `WorkingDirectory` | **必须设置**：`config.yml` 里 `python_path: ./venv/bin/python` 是相对路径，工作目录错了会找不到 venv |
| `EnvironmentFile` | 指向上一步的密钥文件。文件不存在会导致服务启动失败（systemd 会报错） |
| `ExecStart` | 启动命令。注意要带 `server` 子命令 |
| `Restart=on-failure` | 进程异常退出（非 0）时自动重启 |
| `RestartSec=5s` | 两次重启间隔 5 秒，避免疯狂重启 |
| `KillSignal=SIGINT` | 用 SIGINT 触发 tokio/axum 的优雅退出（Ctrl+C 同款信号） |
| `TimeoutStopSec=60` | 优雅退出最多等 60 秒，超时发 SIGKILL。若常有大文档在解析，可适当调大 |
| `StandardOutput/Error=journal` | 日志统一进 journald，用 `journalctl` 查看 |
| `WantedBy=multi-user.target` | `enable` 后会在多用户模式下自启 |

---

## 第三步：安装并启动服务

### 3.1 安装 unit 文件

需要 sudo。**注意写部署脚本时的一个坑**：`echo 密码 | sudo -S tee ... <<HEREDOC` 里管道和 heredoc 会抢同一个 stdin，导致 sudo 把 heredoc 内容当密码读 → 三次失败。正确做法是先写临时文件，再用 `sudo install` 装进去（密码走管道、文件内容不冲突）。

```bash
# 1) 先用普通用户把 unit 写到临时文件(无 sudo, 无 stdin 冲突)
cat > /tmp/document-parser.service <<'EOF'
[Unit]
Description=Document Parser Service (MCP document-parser)
After=network.target

[Service]
Type=simple
User=swufe
Group=swufe
WorkingDirectory=/home/swufe/workspace/document-parser
EnvironmentFile=/home/swufe/workspace/document-parser/.document-parser.env
ExecStart=/home/swufe/workspace/document-parser/document-parser server
Restart=on-failure
RestartSec=5s
KillSignal=SIGINT
TimeoutStopSec=60
StandardOutput=journal
StandardError=journal
SyslogIdentifier=document-parser

[Install]
WantedBy=multi-user.target
EOF

# 2) 安装到系统目录(密码走 stdin, 不冲突)
sudo install -m 644 -o root -g root /tmp/document-parser.service \
     /etc/systemd/system/document-parser.service
rm -f /tmp/document-parser.service

# 3) 让 systemd 重新加载 unit 定义
sudo systemctl daemon-reload
```

### 3.2 启用并启动

```bash
# 设置开机自启(建立 multi-user.target.wants 软链)
sudo systemctl enable document-parser

# 启动服务
sudo systemctl start document-parser
```

---

## 第四步：验证

```bash
# 1) 服务是否 active
systemctl is-active document-parser      # 期望: active
systemctl is-enabled document-parser     # 期望: enabled

# 2) 完整状态(含最近几行日志)
systemctl status document-parser

# 3) 端口是否监听(默认 8087)
ss -ltn | grep :8087

# 4) 确认密钥真的注入到进程环境(只判断存在性, 不打印值)
PID=$(systemctl show -p MainPID --value document-parser)
sudo cat /proc/$PID/environ | tr '\0' '\n' | grep -c '^OSS_ACCESS_KEY_ID=.'
# 输出 >= 1 即表示已注入且非空

# 5) 本机接口探测
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8087/
```

`systemctl status` 期望看到：

```
● document-parser.service - Document Parser Service (MCP document-parser)
     Loaded: loaded (/etc/systemd/system/document-parser.service; enabled; preset: enabled)
     Active: active (running)
   Main PID: 12345 (document-parser)
```

---

## 常用运维命令

### 服务管理

```bash
sudo systemctl start    document-parser   # 启动
sudo systemctl stop     document-parser   # 停止(发 SIGINT 优雅退出, 最多等 60s)
sudo systemctl restart  document-parser   # 重启
sudo systemctl reload   document-parser   # 重载(本服务未实现 reload, 会报错, 用 restart)
systemctl status        document-parser   # 查看状态(无需 sudo)
```

### 日志查看（journald）

```bash
# 实时跟踪日志(类似 tail -f)
sudo journalctl -u document-parser -f

# 最近 100 行
sudo journalctl -u document-parser -n 100 --no-pager

# 本次启动以来的日志
sudo journalctl -u document-parser -b

# 某时间段
sudo journalctl -u document-parser --since "10 min ago" --until "now"

# 只看错误
sudo journalctl -u document-parser -p err
```

### 开机自启管理

```bash
sudo systemctl enable  document-parser   # 开机自启
sudo systemctl disable document-parser   # 取消开机自启
systemctl is-enabled document-parser     # 查询是否自启
```

### 修改 unit 后生效

```bash
# 改了 /etc/systemd/system/document-parser.service 之后, 必须先 reload 再 restart
sudo systemctl daemon-reload
sudo systemctl restart document-parser
```

---

## 更新二进制后如何重启

发布新版本二进制后：

```bash
cd /home/swufe/workspace/document-parser

# 1) 替换二进制(建议先备份旧的)
cp document-parser document-parser.bak
# 把新二进制放进来(覆盖), 并保证可执行
chmod +x document-parser

# 2) 重启服务加载新二进制
sudo systemctl restart document-parser

# 3) 确认起来了
systemctl status document-parser
```

> `Restart=on-failure` 只在异常退出时重启。如果新二进制启动即退出（比如配置错误），systemd 会按 `RestartSec=5s` 反复重试，可在 `journalctl -u document-parser` 看到失败原因。

---

## 常见问题排查

### 1. `dependency failed`（连坐失败）

**现象**：`systemctl status` 显示 `Dependency failed for document-parser.service`，但 unit 文件里并没有写 `Requires=`。

**原因**：systemd 会根据 `WorkingDirectory` 路径自动注入 `RequiresMountsFor=<路径>` 的隐式依赖。如果该路径落在某个挂载失败的分区上（典型场景：`/home` 是独立分区，fstab 里挂载但 fsck/坏块导致 `home.mount` failed），document-parser 就会被「连坐」。

**诊断**：

```bash
# 看实际依赖(systemd 自动注入的)
systemctl show document-parser -p Requires,After,Wants
# 列出所有 failed 单元, 找连坐源头
systemctl --failed
```

**解决**：

- 如果那个分区确实不需要（数据已在别处），在 `/etc/fstab` 里注释掉对应挂载行（**先备份 fstab**），再 `sudo systemctl daemon-reload && sudo systemctl reset-failed`。
- 修改 fstab 示例（用 `|` 作为 sed 分隔符，避免路径里的 `/` 冲突）：

  ```bash
  sudo cp -a /etc/fstab /etc/fstab.bak.$(date +%s)
  sudo sed -i '\#^/dev/disk/by-uuid/<对应UUID>[[:space:]]/home#s|^|# DISABLED: |' /etc/fstab
  sudo systemctl daemon-reload
  sudo systemctl reset-failed
  sudo systemctl start document-parser
  ```

### 2. `status=203/EXEC`

**含义**：systemd **根本没能执行** `ExecStart` 指定的命令。

**常见原因 & 排查**：

| 原因 | 排查命令 |
|------|----------|
| 二进制路径不对 | `ls -l <ExecStart 路径>` |
| 二进制没有执行权限 | `chmod +x document-parser` |
| ELF 解释器缺失（交叉编译/动态库缺失） | `file document-parser`、`ldd document-parser` |
| `User=` 指定的用户对工作目录/二进制无权限 | 切到该用户 `sudo -u <user> <ExecStart>` 手动跑一遍 |

### 3. 密钥未注入进程

**现象**：服务起来了，但访问 OSS 报鉴权失败；`/proc/$PID/environ` 里没有 `OSS_ACCESS_KEY_ID`。

**排查清单**：

```bash
# 1) EnvironmentFile 是否存在且可读
ls -l /home/swufe/workspace/document-parser/.document-parser.env

# 2) 文件里是否带了 export(systemd 不识别) —— 应该没有
grep -E '^\s*export' .document-parser.env && echo "需去掉 export 前缀"

# 3) 幂等地去掉 export 前缀
sed -i -E 's/^[[:space:]]*export[[:space:]]+//' .document-parser.env

# 4) 改完重启
sudo systemctl restart document-parser
```

### 4. 部署脚本里 sudo + heredoc 导致密码认证失败

**现象**：脚本里 `echo "$PW" | sudo -S tee FILE >/dev/null <<'EOF' ... EOF` 报 `3 incorrect password attempts`。

**原因**：管道（`echo |`）和 heredoc（`<<EOF`）争抢同一个 stdin，sudo 把 heredoc 的第一行当成密码读取。

**解决**：改成「先写临时文件（普通用户，无 sudo），再用 `sudo install` 安装」：

```bash
cat > /tmp/x.service <<'EOF'
... unit 内容 ...
EOF
echo "$PW" | sudo -S -p "" install -m 644 /tmp/x.service /etc/systemd/system/x.service
rm -f /tmp/x.service
```

---

## 卸载服务

```bash
sudo systemctl stop    document-parser
sudo systemctl disable document-parser
sudo rm -f /etc/systemd/system/document-parser.service
sudo systemctl daemon-reload
sudo systemctl reset-failed
```

---

## 附录：一键部署脚本参考

把上面几步整合成一个幂等脚本（已规避 stdin 冲突坑）。把 `INSTALL_DIR`、运行用户、sudo 密码按实际情况调整：

```bash
#!/usr/bin/env bash
set -euo pipefail

INSTALL_DIR=/home/swufe/workspace/document-parser
RUN_USER=swufe
RUN_GROUP=swufe
ENV_FILE="$INSTALL_DIR/.document-parser.env"
UNIT_FILE=/etc/systemd/system/document-parser.service

# sudo 封装: 密码走 stdin, 命令本身不读 stdin (避免 heredoc 冲突)
SUDO() { sudo -S -p "" "$@"; }

echo "==> 1) 收紧密钥文件权限"
[ -f "$ENV_FILE" ] || { echo "缺少 $ENV_FILE"; exit 1; }
chmod 600 "$ENV_FILE"
chown "$RUN_USER:$RUN_GROUP" "$ENV_FILE"
# 幂等去除 export 前缀(systemd EnvironmentFile 不识别)
sed -i -E 's/^[[:space:]]*export[[:space:]]+//' "$ENV_FILE"

echo "==> 2) 生成 unit 到临时文件"
TMP=$(mktemp)
cat > "$TMP" <<EOF
[Unit]
Description=Document Parser Service (MCP document-parser)
After=network.target

[Service]
Type=simple
User=$RUN_USER
Group=$RUN_GROUP
WorkingDirectory=$INSTALL_DIR
EnvironmentFile=$ENV_FILE
ExecStart=$INSTALL_DIR/document-parser server
Restart=on-failure
RestartSec=5s
KillSignal=SIGINT
TimeoutStopSec=60
StandardOutput=journal
StandardError=journal
SyslogIdentifier=document-parser

[Install]
WantedBy=multi-user.target
EOF

echo "==> 3) 安装 unit"
SUDO install -m 644 -o root -g root "$TMP" "$UNIT_FILE"
rm -f "$TMP"

echo "==> 4) 启用并启动"
SUDO systemctl daemon-reload
SUDO systemctl enable document-parser
SUDO systemctl restart document-parser

echo "==> 5) 验证"
sleep 3
systemctl is-active document-parser
systemctl is-enabled document-parser
ss -ltn | grep :8087 && echo "8087 监听中"
```

> 脚本里 `SUDO` 函数依赖终端输入密码（`sudo -S` 从 stdin 读）。如需免交互，配置 sudoers NOPASSWD 或在受控环境用 SSH key + root。**不要把密码硬编码进脚本文件。**
