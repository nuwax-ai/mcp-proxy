# document-parser 部署踩坑笔记

实战中踩过的坑，按出现频率排序。

---

## 1. mineru 3.4.0 PageChars bug
**现象**: 任务失败 `TypeError: 'PageChars' object is not iterable`（`pipeline_magic_model.py:94`）。
**原因**: mineru 3.4.0 自身 bug（pipeline 和 hybrid-engine 都走 pipeline 部分，都会撞）。
**解决**: 升级 3.4.2（已修）。`setup-venv.sh` 已锁 `mineru[core]==3.4.2`。
```bash
uv pip install -U "mineru[core]==3.4.2" --python ./venv/bin/python
```
> 阿里云源最新可能只到 3.4.0，用 pypi 官方源：`--index-url https://pypi.org/simple`

---

## 2. huggingface-hub 依赖冲突
**现象**: `huggingface-hub>=0.34.0,<1.0 is required ... but found huggingface-hub==1.22.0`。
**原因**: 升级 mineru 3.4.2 时 huggingface-hub 被升到 1.22，但 mineru/transformers 要 <1.0。
**解决**:
```bash
uv pip install "huggingface-hub>=0.34,<1.0" --python ./venv/bin/python
```
`setup-venv.sh` 已包含。

---

## 3. hybrid-engine 的 vllm 冲突 + OOM
**现象 A**: hybrid-engine 报 `Please install vllm to use the vllm-async-engine backend`。
**原因**: vllm 要 huggingface-hub≥1.0，mineru 要 <1.0，降级 huggingface-hub 会卸 vllm —— 二者互斥。
**现象 B**: `CUDA out of memory`（vllm 默认占 50% 显存，与 voice-cli 共存 OOM）。
**解决**:
- **用 `pipeline` backend**（不用 vllm，无冲突，仍走 cuda 加速 OCR/公式/表格）。改 config.yml `backend: "pipeline"`。质量略低于 hybrid-engine，但稳定。
- 或保持 hybrid-engine + 调 `gpu_memory_utilization: 0.3` 让 vllm 少占显存（config.yml 默认 0.3）。但前提是 vllm 还在（没被降级 huggingface-hub 卸掉）。

---

## 4. mineru 3.4 CLI 参数变更
3.4 起不再支持 `-d` / `--vram` / `--source` CLI 参数。它们会被 `ignore_unknown_options=True` 静默吞掉，转发给 vllm 的 `AsyncEngineArgs(**kwargs)`，触发 `TypeError: unexpected keyword argument 'vram'`。

document-parser 已改用环境变量（代码自动注入，用户无需手动设）：
- device → `MINERU_DEVICE_MODE`
- vram → `MINERU_VIRTUAL_VRAM_SIZE`
- model-source → `MINERU_MODEL_SOURCE`

> ⚠️ `MINERU_BACKEND` 不是有效环境变量（document-parser 不读它），backend 只能通过 config.yml 的 `mineru.backend` 配置。

---

## 5. backend 命名变更（3.4）
旧名 `vlm-transformers` / `vlm-sglang-engine` / `vlm-sglang-client` 已废弃（**无别名**）。
新合法值: `pipeline` / `vlm-engine` / `hybrid-engine` / `vlm-http-client` / `hybrid-http-client`。

---

## 6. venv 被建成了 anaconda 的 Python 3.7
**现象**: `uv pip install` 报 `the current Python version (3.7) does not satisfy Python>=3.10,<3.14`。
**原因**: shell 激活了 anaconda base（Python 3.7），uv 默认拿了它。
**解决**: 创建 venv 时用系统 Python 绝对路径 `/usr/bin/python3`（3.12），避开 anaconda。`setup-venv.sh` 已处理。
```bash
uv venv --python /usr/bin/python3 ./venv
```

---

## 7. 部署脚本 sudo + heredoc stdin 冲突
**坑**: `echo 密码 | sudo -S tee FILE <<EOF ... EOF` 里管道和 heredoc 抢同一 stdin，sudo 把 heredoc 内容当密码读 → 3 次失败。
**正解**: 先写临时文件（普通用户），再 `sudo install`（密码走管道、文件内容不冲突）。`install.sh` 已采用。

---

## 8. /home 独立分区导致 dependency failed
**现象**: `systemctl status` 报 `Dependency failed for document-parser.service`，但 unit 里没写 `Requires=`。
**原因**: systemd 根据 WorkingDirectory 路径自动注入 `RequiresMountsFor=/home/...`；若 /home 是独立分区且挂载失败（如坏块/fsck 失败），document-parser 被连坐。
**排查**: `systemctl show document-parser -p Requires`、`systemctl --failed`、`dmesg | grep -i "medium error"`。
**解决**: 修复分区，或部署到根分区下的 `/opt/`。

---

## 9. 验证密钥真的注入了进程
```bash
PID=$(systemctl show -p MainPID --value document-parser)
sudo cat /proc/$PID/environ | tr '\0' '\n' | grep OSS
```
输出 `OSS_ACCESS_KEY_ID=...` 即注入成功。环境变量只在进程启动时注入，改完 `.env` 要 `systemctl restart`。

---

## 10. 服务器本地编译缺系统 -dev 包
**现象**: `cargo build` 报 `openssl-sys` / `could not find OpenSSL` / `cmake not installed` / `stdbool.h not found`。
**原因**: 服务器没装编译依赖。document-parser 依赖 `openssl-sys`（native-tls），不像 voice-cli 用 rustls；还需 cmake（whisper-cuda 也用，但 doc-parser 部分依赖）、libc6-dev（含 stdbool.h）。
**解决**:
```bash
sudo apt-get install -y build-essential cmake pkg-config libssl-dev
```
> ⚠️ apt **批量装可能假成功**（部分包没装但 `exit 0`）。装完逐个验证：`command -v cmake && pkg-config --exists openssl && echo OK`。

---

## 11. utoipa-swagger-ui build.rs 在线下载失败
**现象**: `failed to download Swagger UI: curl download file exited with error status: exit status: 56`（cargo 编译期）。
**原因**: `utoipa-swagger-ui` 的 build.rs 在线下载 Swagger UI 前端资源；网络受限环境（github/CDN 被墙）curl 56（收数据失败）。
**解决**: **重试**。同机器先编过其它 crate（如 voice-cli）会命中 `target/release/build/utoipa-swagger-ui-*/out/` 缓存，document-parser 直接复用跳过下载。彻底断网环境需预置 swagger-ui 资源（见 utoipa-swagger-ui 文档的 vendored 方案）。

---

## 12. 国内网络镜像（构建 + venv 加速）
github / huggingface / static.rust-lang.org / pypi 在国内常被墙或龟速。对应镜像：
```bash
# rust 工具链 + crate（~/.cargo/config.toml）
[source.crates-io]
replace-with = "rsproxy-sparse"
[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"
# 安装 rust: export RUSTUP_DIST_SERVER=https://rsproxy.cn

# Python venv（setup-venv.sh 前设）
export UV_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple   # torch CUDA ~2.5GB 快很多

# github release（sherpa C 库 / kokoro）/ HF 模型
#   fetch-sherpa.sh / fetch-voice-models.sh 已内置多镜像自动探活（ghproxy.net→gh-proxy.com→mirror.ghproxy.com）
#   mineru 模型：document-parser 自动注入 MINERU_MODEL_SOURCE=modelscope（绕过 HF）
```
> rust 首装若 bin/ 为空（manifest 有 cargo 组件但无二进制），是下载损坏：`rustup toolchain uninstall stable && rustup toolchain install stable --profile minimal` 重装。
