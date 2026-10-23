# voice-cli 部署指南

语音转文字（STT）+ 文字转语音（TTS）服务。通过统一安装器 **deploy-installer**（npm 包 `@nuwax-ai/deploy-installer`）一条命令完成安装：下载程序和模型 → 写好配置 → 注册成开机自启服务 → 等服务就绪，全程自动。

> document-parser（文档解析）的部署见姊妹篇 [deploy-document-parser.md](./deploy-document-parser.md)。

## 1. 平台与加速方式总览

voice-cli 的转写功能在不同机器上用不同的方式加速，**安装器自动检测、无需手工选择**：

| 平台 | 加速方式 | 说明 |
|------|---------|------|
| macOS（Apple 芯片） | Metal | GPU 加速已编译进 Mac 版程序，装好即用 |
| Linux + NVIDIA 显卡 | **CUDA** | 安装器自动下载 GPU 加速包（约 370MB，覆盖 GTX 10 系至 RTX 40 系）；语音转写用 GPU 加速，语音合成始终用 CPU |
| Linux + AMD/Intel 显卡 | **Vulkan** | 安装器自动下载 GPU 加速包（约 31MB）；只有语音转写（Whisper）用 GPU，合成等仍用 CPU |
| 无显卡 / Windows | CPU | 安装包自带程序，装好即用 |

以上**自动检测、无需选择**：按 NVIDIA 显卡 → AMD/Intel 显卡 → 无显卡（CPU）的顺序判断。没有独显的机器会装 CPU 版并给出提示，**不影响正常使用**。

| 项目 | 要求 |
|------|------|
| Node.js | 18 或更新版本（必需，用于运行安装器） |
| **ffmpeg** | 转写时要解码音频。**默认无需手动安装**：安装器会自动下载一份 ffmpeg 到安装目录（系统里已装过就优先用系统的）。只有内网机器且系统也没有时才需手动装：macOS `brew install ffmpeg`；Debian/Ubuntu `sudo apt install -y ffmpeg`；Windows `winget install Gyan.FFmpeg`（doctor 命令会自动检查并给出提示） |
| 磁盘 | Whisper large-v3 模型约 3GB（自动下载）+ Linux 显卡加速包（NVIDIA 约 370MB / AMD·Intel 约 31MB） |
| Linux sudo 权限 | 按 [document-parser 部署指南 §3](./deploy-document-parser.md) 配置一次免密 sudo（有现成命令，复制即可） |

## 2. 安装 deploy-installer

```bash
npm install -g @nuwax-ai/deploy-installer
```

国内网络先配 npm 镜像（注意：加 sudo 安装时要把镜像地址直接写在命令里），具体见 [document-parser 部署指南 §2](./deploy-document-parser.md)。装完自检：

```bash
deploy-installer doctor
```

Linux 上 doctor 还会检查显卡，并显示一行 `voice-cli tier (auto)`，提前告诉你这台机器会走哪种加速。

## 3. 一键部署

```bash
deploy-installer voice-cli install
```

默认安装目录 `~/voice-cli`、端口 **8077**；装到其它目录加 `--install-dir`：

```bash
deploy-installer voice-cli install --install-dir ~/apps/voice-cli
```

配置、模型、ffmpeg、日志等所有文件都放在安装目录里。**非默认目录时，后续的 `upgrade` / `verify` / `service` 命令都要带同样的 `--install-dir`**（不带就默认找 `~/voice-cli`）。各平台差异：

- **macOS**：自动下载 Whisper large-v3 模型（约 3GB）；GPU 加速（Metal）装好即用。
- **Linux**：根据显卡自动选加速包（见 §4）；Whisper large-v3 模型自动下载。
- **Windows**：CPU 版程序，注册成计划任务实现开机自启。

安装成功会打印 `✅ voice-cli → http://127.0.0.1:8077`；如果超时，会如实报错并告诉你怎么查日志（显卡加速包装错是常见原因之一，见 §6.2）。

> **只用 SSH 远程连接 Mac 的情况**：Mac 的开机自启要求你本人在这台 Mac 上登录过桌面。远程安装会正常完成（配置都已写好），但要等你坐到这台 Mac 前登录一次桌面，服务才会启动——这是正常行为，不是安装失败；之后在纯 SSH 下执行 `service start` 也会报同样的提示。

## 4. 显卡加速控制（Linux）

自动检测不准时可以手动指定：

```bash
deploy-installer voice-cli install --use-oss-cuda      # 强制用 NVIDIA 显卡（跳过检测；环境不匹配时后果自负）
deploy-installer voice-cli install --use-oss-vulkan    # 强制用 AMD/Intel 显卡
deploy-installer voice-cli install --skip-oss-cuda --skip-oss-vulkan   # 两个 skip 一起用 = 强制 CPU
```

行为约定：

- 单独用 `--skip-oss-cuda` 只是"不用 NVIDIA"，机器还可能自动选 AMD/Intel；**两个 skip 一起用**才是"强制 CPU"。
- `--use-oss-cuda` 和 `--skip-oss-cuda` 同时给会直接报错（自相矛盾）。
- **换加速方式会自动清理**：从一种换成另一种时，安装器会自动删掉旧方式的残留文件；强制降级到 CPU 也是持久的——之后的升级不会悄悄跳回显卡版。
- **升级保持原方式**：已经用 NVIDIA/AMD·Intel 加速的机器，`upgrade` 后还是原方式（即使显卡驱动临时坏了也不会被 CPU 版覆盖）；升级完会自动重启在跑的服务。
- AMD/Intel 方式需要系统装有 `libvulkan.so.1` 和显卡驱动（Ubuntu：`sudo apt install -y libvulkan1 mesa-vulkan-drivers`）；驱动有问题时自动降级为 CPU，不会卡住安装。

## 5. 验证

```bash
# 健康检查（能看到已加载的模型）
curl http://localhost:8077/health

# 引擎与模型清单
curl http://localhost:8077/models

# 快速试一次语音转写（模型和 ffmpeg 都已自动就位，无需手动准备）
curl -X POST http://localhost:8077/transcribe -F "file=@test.wav"

# 语音合成（TTS）音色清单，详见接口文档
curl http://localhost:8077/api/v1/tts/voices
open http://localhost:8077/api/docs

# 另一种风格的接口文档（与上面并存；页面组件由浏览器从公网加载，纯内网打不开）
open http://localhost:8077/api/docs/scalar

# 一键自验（健康 + 接口文档 + 转写试跑）
deploy-installer voice-cli verify
```

Whisper 模型默认自动下载（约 3GB，Mac/Linux/Windows 通用）；
想一次装全五档模型（tiny/base/small/medium/large-v3，约 5GB）加 `--models all`；
装的时候跳过了模型（`--skip-models`）之后重跑一次 install 就会补上。
模型和 ffmpeg 互不影响——跳过模型不影响 ffmpeg 的自动下载。

想确认 GPU 加速真的生效了：转写时看服务日志（`journalctl -u voice-cli -f`），
出现 `ggml_cuda: using CUDA` 或 `ggml_vulkan: Found ... Vulkan devices` 字样就说明在用显卡。

## 6. 服务管理与升级

```bash
deploy-installer voice-cli service status
deploy-installer voice-cli service stop       # 停止（重复执行也安全）
deploy-installer voice-cli service start      # 启动（会等服务真正就绪才返回）
deploy-installer voice-cli service restart    # 重启（Mac/Linux/Windows 通用）
deploy-installer voice-cli service uninstall  # 卸载
deploy-installer voice-cli upgrade            # 升级到新版（自动重启在跑的服务；Linux 保持原加速方式）
deploy-installer voice-cli upgrade --install-dir ~/apps/voice-cli   # 装在非默认目录时要带；verify / service 命令同理
```

### 6.1 启动 / 停止

stop / start 在三个平台用法完全一样，不用记各系统自己的命令：

```bash
deploy-installer voice-cli service stop    # 停止（Mac 上会把开机自启也一并注销，start 会自动恢复）
deploy-installer voice-cli service start   # 启动并等待服务就绪；纯 SSH 连 Mac 的限制见 §3 说明
```

两个命令**重复执行也安全**：已停止的服务再 stop、已运行的服务再 start，都只是提示一下并正常结束。停止时会等端口真正释放完（Windows 上超时会自动结束残留进程）；启动时只认健康检查真正通过（起不来会明确报错，并告诉你怎么查日志）。

各平台等价的系统原生命令（**仅供排障时使用**，平时用上面的命令即可）：

| 平台 | 停止 | 启动 |
|------|------|------|
| Linux | `sudo systemctl stop voice-cli` | `sudo systemctl start voice-cli` |
| macOS | `launchctl bootout gui/$(id -u)/com.nuwax.voice-cli` | `launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.nuwax.voice-cli.plist` |
| Windows | `schtasks /end /tn com.nuwax.voice-cli` | `schtasks /run /tn com.nuwax.voice-cli` |

- Windows：手动 `/end` 之后要等几秒再 `/run`（旧进程释放端口需要一点时间，立刻重跑容易报端口被占）——上面的子命令已自动处理这个等待。
- macOS：表里的"停止"会把开机自启一起注销，之后必须用表里的"启动"重新注册——这正是推荐用子命令的原因，这些细节它都会自动处理。

### 6.2 常见问题

| 现象 | 原因与处理 |
|------|-----------|
| 安装时提示"未检测到 NVIDIA GPU / Vulkan 运行时"后装了 CPU 版 | 机器没检测到可用的显卡环境，自动退回 CPU 版——**不影响使用**；想用显卡加速就按提示装好驱动后重新 install |
| NVIDIA 加速装了但服务起不来，日志报 `libcublas.so.12 not found` | 加速包不含 NVIDIA 系统库，需要先装 CUDA toolkit 再重装（doctor 命令能提前发现这个问题） |
| AMD/Intel 加速起不来，提示 `libvulkan.so.1` 缺失 | `sudo apt install -y libvulkan1 mesa-vulkan-drivers` 后重装 |
| 转写报音频处理错误（ffmpeg 缺失或损坏） | 说明 ffmpeg 没装上或坏了——看看 `~/voice-cli/ffmpeg(.exe)` 在不在，不在就重新跑一次 install；内网环境也可以自己放一个（用 ffmpeg 官方静态包） |
| 转写报无模型 / 模型缺失 | 重新跑一次 install 会自动补下载；也可以自己下载模型放进 `models/` 目录（模型源见 [whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp)） |
| 偶尔返回"解码/合成引擎忙"（HTTP 4xx） | 正常的并发保护：这个引擎实例正被别的请求占用（长音频的解码不能中断，通常几十秒内结束）——客户端稍等重试即可；出现频繁可在 `config.yml` 里调大 `whisper.engine.pool_size` / `tts.engine.pool_size`（增加并行实例数） |
| 想换一种加速方式 | 直接带对应参数重跑 install（旧方式的文件会自动清理），例如降到 CPU：`install --skip-oss-cuda --skip-oss-vulkan` |

## 7. 配置

配置文件 `~/voice-cli/config.yml`（安装时从模板生成），常用项：

```yaml
server:
  host: "0.0.0.0"
  port: 8077            # 改端口后 service restart 生效

whisper:
  default_model: "large-v3"   # models/ 下已有的 ggml-*.bin 名（不含前缀）
```

STT/TTS 引擎选择（whisper | sensevoice、Kokoro | ZipVoice）、批转写并发、模型目录等完整配置见 [crates/voice-cli/docs/DEPLOYMENT.md](../../../crates/voice-cli/docs/DEPLOYMENT.md)。
