# Mac 故障排除

## deploy-installer: missing binary for darwin-arm64

npm 包未包含当前平台二进制。第一期仅支持 **Apple Silicon**。请确认：

```bash
uname -m   # 应为 arm64
npm ls -g nuwax-deploy-installer
```

## 解析很慢 / CPU 占用高

确认 `config.yml` 中：

```yaml
mineru:
  device: "mps"
```

macOS 不会自动启用 MPS，必须显式设置。

## launchd 服务未启动

```bash
deploy-installer document-parser service status --install-dir ~/document-parser
tail -f ~/document-parser/logs/launchd.stderr.log
tail -f ~/document-parser/logs/launchd.stdout.log
```

常见原因：

- 安装目录在 `~/Documents` / Desktop / iCloud：会报 `Operation not permitted`，请改用 `~/document-parser`
- `.document-parser.env` 密钥未填或格式错误（不要用 `export` 前缀）
- 端口 8087 被占用：修改 `config.yml` 的 `server.port`
- `venv` 未就绪：重新 `deploy-installer document-parser setup --use-prebuilt-venv`
- 首次启动卡在 MinerU 检查：等 1–2 分钟再 `curl http://127.0.0.1:8087/health`；旧版若 plist 含 `ProcessType=Background` 会导致 MPS 卡住，请升级 CLI 后重新 `service install`

## health 不通但 status 显示 running

环境检查（MinerU/MarkItDown）未完成前，HTTP 尚未监听。看 stdout 日志是否已出现 `Service started successfully`。
## uv-init 失败

```bash
brew install uv python@3.12
cd ~/document-parser && ./document-parser uv-init
./document-parser check
```

或使用 OSS 预编译 venv，见 [oss-optional-assets.md](./oss-optional-assets.md)。

## 卸载

```bash
deploy-installer document-parser service uninstall --install-dir ~/document-parser
rm -rf ~/document-parser   # 可选：删除数据目录
```
