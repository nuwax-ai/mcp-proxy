# 启用 TTS（默认禁用）

voice-cli **默认 `tts.enabled: false`**。这样设计的好处：
- 缺 `tts_service.py` 不再阻塞 STT 服务启动
- `/tts/*` 请求返回 503，STT（`/api/v1/tasks/transcribeFromUrl` 等）正常工作

## 不启用 TTS（默认）
什么都不用做。服务正常起，STT 可用，`/tts/*` 返回 503。

## 启用 TTS 步骤

> TTS 用 IndexTTS，需要 Python 3.10 + torch + IndexTTS 模型，是套**重量级**环境（下载几 G）。按需启用。

```bash
# 1. 放 tts_service.py 到工作目录（脚本在 voice-cli/ 源码目录）
cp /path/to/voice-cli/tts_service.py /opt/voice-cli/

# 2. 初始化 TTS Python 环境（装 IndexTTS + 模型）
#    用源码自带的 install_indextts.sh（在 voice-cli/ 源码目录）
cd /path/to/voice-cli
bash install_indextts.sh

# 3. config.yml 启用 TTS
cat >> /opt/voice-cli/config.yml <<'EOF'
# （把 tts 段的 enabled 改成 true，或在 tts 段加:）
EOF
sed -i 's/enabled: false/enabled: true/' /opt/voice-cli/config.yml
# 或手动编辑: vim /opt/voice-cli/config.yml → tts.enabled: true

# 4. 重启
/opt/voice-cli/deploy/server-manager.sh restart

# 5. 验证
/opt/voice-cli/voice-cli tts test                          # CLI 测试
curl -X POST http://127.0.0.1:8087/tts/sync \
  -H 'Content-Type: application/json' \
  -d '{"text":"测试语音合成"}'                              # HTTP 测试
#   200 = TTS 可用；503 = 未启用或脚本缺失
```

## 详细 TTS 配置
见源码目录的 `TTS_README.md` 和 `INDEXTTS_SETUP.md`。
