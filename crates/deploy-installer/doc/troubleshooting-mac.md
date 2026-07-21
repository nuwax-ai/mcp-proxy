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
```

常见原因：

- `.document-parser.env` 密钥未填或格式错误（不要用 `export` 前缀）
- 端口 8087 被占用：修改 `config.yml` 的 `server.port`
- `venv` 未就绪：重新 `deploy-installer document-parser setup`

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
