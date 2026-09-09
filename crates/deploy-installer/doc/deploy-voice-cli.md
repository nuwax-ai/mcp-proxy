# voice-cli 部署指南

语音转写（STT）+ 语音合成（TTS）服务：Whisper / SenseVoice 引擎转写，sherpa-onnx（Kokoro / ZipVoice）合成。通过统一部署 CLI **deploy-installer**（npm 包 `nuwax-deploy-installer`）一键安装：复制二进制与伴生库 → 下载 Whisper 模型（macOS）→ 写配置 → 注册系统服务 → 等待健康检查通过。

> document-parser（文档解析）的部署见姊妹篇 [deploy-document-parser.md](./deploy-document-parser.md)。

## 1. 平台与 GPU 档位总览

voice-cli 的 STT 在不同平台走不同加速档位，**安装器自动检测、无需手工选择**：

| 平台 | 档位 | 加速说明 |
|------|------|---------|
| macOS（Apple Silicon） | Metal | whisper-metal 编译进 darwin 二进制，开箱即用 |
| Linux x86_64 + NVIDIA | **CUDA** | 安装器下载 CUDA 预编译 bundle（~235MB，含 GPU 版 onnxruntime）；STT 与 TTS/SenseVoice 都吃 GPU |
| Linux x86_64 + AMD/Intel GPU | **Vulkan** | 安装器下载 Vulkan 预编译 bundle（~30MB）；**仅 STT-whisper 加速**（TTS/SenseVoice 仍 CPU） |
| Linux 无 GPU / Windows | CPU | vendor 内置二进制，开箱即用 |

三档自动检测优先级：NVIDIA（nvidia-smi + libcublas）→ Vulkan（GPU 探针）→ CPU。无 GPU 机器会打印提示与修复建议但**不阻塞部署**（装 CPU 版照常可用）。

| 项目 | 要求 |
|------|------|
| Node.js | 18+（deploy-installer 方式必需） |
| **ffmpeg** | **必需**（音频元数据/格式处理）：macOS `brew install ffmpeg`；Debian/Ubuntu `sudo apt install -y ffmpeg`；Windows `winget install Gyan.FFmpeg`（doctor 会检出并给对应命令） |
| 磁盘 | whisper large-v3 模型约 3GB（仅 macOS 自动下载时需要） |
| Linux sudoers | 同 document-parser 的五命令 NOPASSWD allowlist（见 [deploy-document-parser.md §3](./deploy-document-parser.md)） |

## 2. 安装 deploy-installer

```bash
npm install -g nuwax-deploy-installer
```

国内网络镜像建议与 sudo 内联传参等注意事项同 [document-parser 部署指南 §2](./deploy-document-parser.md)（完全一致）。装完自检：

```bash
deploy-installer doctor
```

Linux x86_64 上 doctor 会额外输出 GPU 预检（nvidia-smi / libcublas / Vulkan loader+GPU 探针）与 `voice-cli tier (auto)` 档位汇总，部署前即可确认机器会走哪档。

## 3. 一键部署

```bash
deploy-installer voice-cli install
```

默认安装目录 `~/voice-cli`、端口 **8077**。各平台差异：

- **macOS**：自动从 OSS 下载 Whisper large-v3 模型（约 2.8GB，写入 `models/ggml-large-v3.bin`）；二进制 Metal 加速开箱即用。
- **Linux**：按 GPU 档位自动选 CUDA / Vulkan / CPU 包（见 §4）。**Whisper 模型 OSS 只提供 macOS 下载源**——Linux 需自备：从 [whisper.cpp 模型源](https://huggingface.co/ggerganov/whisper.cpp) 下载 `ggml-*.bin` 放入 `~/voice-cli/models/`（国内网络不通 HF 时可经代理机器下载后 scp 过去）。
- **Windows**：CPU 版二进制 + 伴生 DLL，以当前用户计划任务（`com.nuwax.voice-cli`）注册服务。

健康检查通过打印 `✅ voice-cli → http://127.0.0.1:8077`；超时如实报错（GPU 档位装错是常见原因之一，见 §6）。

## 4. GPU 档位控制（Linux x86_64）

自动检测之外可显式控制档位：

```bash
deploy-installer voice-cli install --use-oss-cuda      # 强制 CUDA 档（跳过预检，环境不对自负）
deploy-installer voice-cli install --use-oss-vulkan    # 强制 Vulkan 档
deploy-installer voice-cli install --skip-oss-cuda --skip-oss-vulkan   # 双 skip = 强制 CPU
```

行为约定：

- **`--skip-oss-cuda` 单用不再是"强制 CPU"**（历史语义已变）——只跳过 CUDA 档，仍可自动选 Vulkan；强制 CPU 用双 skip。
- 同档 use+skip 组合（如 `--use-oss-cuda --skip-oss-cuda`）会被直接拒绝。
- **档位切换自动互斥清理**：CUDA ↔ Vulkan ↔ CPU 互切时安装器清理另一档的残留文件（marker / CUDA 专属 .so），双 skip 强制降级也是持久的——下次自动升级不会悄悄跳回 GPU 档。
- **升级保档**：已装 CUDA/Vulkan 档的机器 `upgrade` 保持原档位（驱动临时不可用也不会被 CPU 版覆盖）；`upgrade` 完成后自动重启在跑的服务。
- Vulkan 档依赖系统 `libvulkan.so.1` 与 GPU 驱动（AMD/Intel：`sudo apt install -y libvulkan1 mesa-vulkan-drivers`）；探针发现坏驱动时按"无 Vulkan"降级，不会卡住安装。

## 5. 验证

```bash
# 健康检查（含 models_loaded 状态）
curl http://localhost:8077/health

# 引擎与模型清单
curl http://localhost:8077/models

# STT 冒烟（需已放好 whisper 模型 + 系统 ffmpeg）
curl -X POST http://localhost:8077/transcribe -F "file=@test.wav"

# TTS 音色清单 / 合成（详见 Swagger UI）
curl http://localhost:8077/api/v1/tts/voices
open http://localhost:8077/api/docs
```

Linux GPU 档验证加速是否生效：服务日志（`journalctl -u voice-cli -f`）转写时出现 `ggml_cuda: using CUDA` 或 `ggml_vulkan: Found ... Vulkan devices` 即在走 GPU。

## 6. 服务管理与升级

```bash
deploy-installer voice-cli service status
deploy-installer voice-cli service restart
deploy-installer voice-cli service uninstall
deploy-installer voice-cli upgrade        # Linux 保档升级 + 自动重启；mac/Windows 升级 vendor 二进制
```

### 6.1 常见问题

| 现象 | 原因与处理 |
|------|-----------|
| 安装时打印“未检测到 NVIDIA GPU / Vulkan 运行时”后继续装了 CPU 版 | 预检未过 + 未显式强制——属正常回退；要上 GPU 按提示补驱动/工具包后重装，或用 `--use-oss-*` 强制 |
| CUDA 档服务起不来，日志 `libcublas.so.12 not found` | bundle 不含 CUDA 库，依赖系统 toolkit：装 `cuda-toolkit` 后重装（doctor 的 libcublas 预检可提前发现） |
| Vulkan 档起不来，`libvulkan.so.1` 缺失 | `sudo apt install -y libvulkan1 mesa-vulkan-drivers` |
| 转写报音频处理错误 | 系统 ffmpeg 未装（§1 的安装命令；doctor 检查项） |
| Linux 上模型 404 / 转写无模型 | Linux 不走 OSS 模型下载——手工放 `models/ggml-*.bin`（§3） |
| 想换档位 | 直接带目标旗标重跑 install（自动互斥清理），如 CUDA 机器降级：`install --skip-oss-cuda --skip-oss-vulkan` |

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
