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

- 安装用户未在本机 **图形界面登录**：LaunchAgent 需要 `gui/<uid>`。控制台是别人、你只 SSH 进来时，`service install` 常失败（exit 134 / `Domain does not support specified action`）。请用安装用户登录桌面后再装；或先手工跑二进制验证（见 [mac-mini-quickstart.md](./mac-mini-quickstart.md)「SSH 临时验证」）
- 安装目录在 `~/Documents` / Desktop / iCloud：会报 `Operation not permitted`，请改用 `~/document-parser`
- `.document-parser.env` 密钥未填或格式错误（不要用 `export` 前缀）；改密钥后需 `service restart`
- 端口 8087 被占用：修改 `config.yml` 的 `server.port`；或先 `pkill` 掉手工启动的 `document-parser`
- `venv` 未就绪 / OSS 404：确认已上传 `venv-macos-arm64-{X.Y.Z}.tar.gz`（beta 用同系列稳定版文件名），再 `setup --use-prebuilt-venv`
- 首次启动卡在 MinerU 检查：等 1–2 分钟再 `curl http://127.0.0.1:8087/health`；旧版若 plist 含 `ProcessType=Background` 会导致 MPS 卡住，请升级 CLI 后重新 `service install`
- 确认 LaunchAgent 直接启动二进制（`ProgramArguments` 应为 `document-parser --config … server`，不应再有 `run-server.sh`）：
  `plutil -p ~/Library/LaunchAgents/com.nuwax.document-parser.plist`

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
