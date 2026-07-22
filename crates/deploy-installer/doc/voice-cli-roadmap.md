# voice-cli 自动部署路线图

相对 document-parser（已有 npm + Mac Mini 一键路径），voice-cli 的对齐进度如下。

| 能力 | document-parser | voice-cli |
|------|-----------------|-------------|
| `deploy-installer …` 子命令 | 有 | **已有一期**（`setup` / `install` / `upgrade` / `service`） |
| 二进制进 npm `vendor/darwin-arm64/` | 有 | 有（含 macOS sherpa/onnx dylib） |
| 预编译大依赖 OSS | Python venv | **Whisper ggml**（默认 large-v3 包；可选全量包） |
| 服务注册 | deploy-installer → launchd/systemd | 同上；另保留 `./voice-cli service`（Linux CUDA drop-in） |
| Mac Mini 文档 | [mac-mini-quickstart.md](./mac-mini-quickstart.md) | 同上（合并入口） |

## 一期（已落地）

1. `deploy-installer voice-cli setup|install|upgrade|service …`
2. 默认安装目录 `~/voice-cli`，默认端口 `8077`
3. LaunchAgent：`voice-cli server run --config <dir>/config.yml`
4. **默认 OSS 拉取 `ggml-large-v3.bin`**，`whisper.default_model: large-v3`
5. assemble 脚本构建 binary + dylib + templates

## 二期（未做）

- TTS/Kokoro 预置 OSS 包
- Linux x86_64 npm vendor + sherpa CUDA 一键 OSS
- 与 document-parser 同机 GPU 共存调优文档

## 当前 Linux 生产路径

继续使用 [crates/voice-cli/deploy/README.md](../../voice-cli/deploy/README.md)：

```bash
./voice-cli service install --install-dir /opt/voice-cli \
  --cuda-lib-dir /usr/local/cuda/lib64 --cudnn-lib-dir <cudnn/lib>
```
