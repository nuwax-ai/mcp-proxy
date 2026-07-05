# voice-cli API 集成测试手册

> 全部 HTTP/WebSocket 接口的请求/响应示例 + curl/python 测试用例。
> 部署见 [DEPLOYMENT.md](./DEPLOYMENT.md)。Swagger UI：`http://localhost:8080/api/docs`。

- **Base URL**：`http://localhost:8080`
- **服务须已启动**：`cd ~/voice-cli-test && voice-cli server run`（见 DEPLOYMENT.md §6）
- 测试音频：`~/voice-cli-test/jfk.wav`（16kHz mono，~8s 英文）

---

## 接口总览

| 分类 | 方法 | 路径 | 说明 |
|---|---|---|---|
| 健康 | GET | `/health` | 服务状态 |
| 模型 | GET | `/models` | 已就绪 STT 模型列表 |
| **STT 同步** | POST | `/transcribe` | 上传音频 → 文本（阻塞） |
| STT 同步 | POST | `/transcribeFromUrl` | 从 URL 拉音频转写 |
| **STT 异步** | POST | `/api/v1/tasks/transcribe` | 提交 → task_id |
| STT 异步 | GET | `/api/v1/tasks/{id}` | 查状态 |
| STT 异步 | GET | `/api/v1/tasks/{id}/result` | 取结果 |
| **STT 流式** | WS | `/api/v1/stream/transcribe` | 实时增量识别（LA2） |
| **TTS 同步** | POST | `/api/v1/tts` | 文本 → 音频二进制 |
| TTS | GET | `/api/v1/tts/voices` | 音色数 |
| **TTS 异步** | POST | `/api/v1/tasks/tts` | 提交 → task_id |
| TTS 异步 | GET | `/api/v1/tasks/tts/{id}` | 查状态 |
| TTS 异步 | GET | `/api/v1/tasks/tts/{id}/audio` | 下载音频 |
| **TTS 流式** | WS | `/api/v1/stream/tts` | 增量 PCM |
| 任务 | GET/DELETE | `/api/v1/tasks/{id}` | 查询/删除 |
| 任务 | POST | `/api/v1/tasks/{id}/cancel` | 取消 |
| 任务 | POST | `/api/v1/tasks/{id}/retry` | 重试 |
| 任务 | GET | `/api/v1/tasks/stats` | STT 任务统计 |
| 任务 | GET | `/api/v1/tasks/tts/stats` | TTS 任务统计 |

> STT 接口**向后兼容**（旧客户端无感知，新字段全可选）。TTS 接口**全新设计**（无旧 `/tts/sync`）。

---

## 1. 健康检查 + 模型列表

```bash
curl -s http://localhost:8080/health | python3 -m json.tool
curl -s http://localhost:8080/models | python3 -m json.tool
```

---

## 2. STT — 同步转录

```bash
curl -s -F file=@~/voice-cli-test/jfk.wav \
     -F language=en \
     -F model=base \
     http://localhost:8080/transcribe | python3 -m json.tool
```

可选 multipart 字段（全可选，向后兼容）：
- `language`：`en`/`zh`/`ja`...（不传=自动检测）
- `model`：`tiny`/`base`/`small`...（不传= `whisper.default_model`）
- `initial_prompt`：提示词（影响风格/术语）
- `beam_size`、`temperature`：解码参数

响应结构（兼容旧版）：
```json
{ "text": "and so my fellow Americans...", "segments": [{ "text": "...", "start": 0.0, "end": 2.5 }] }
```

---

## 3. STT — 异步转录

```bash
# 提交
TID=$(curl -s -F file=@~/voice-cli-test/jfk.wav -F language=en \
  http://localhost:8080/api/v1/tasks/transcribe \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['task_id'])")
echo "task_id=$TID"

# 轮询状态
curl -s "http://localhost:8080/api/v1/tasks/$TID" | python3 -m json.tool

# 取结果（Completed 后）
curl -s "http://localhost:8080/api/v1/tasks/$TID/result" | python3 -m json.tool
```

---

## 4. STT — 流式 WebSocket（LocalAgreement 2）

协议：
- 客户端 → 服务端：首帧 JSON `{type:"start", sample_rate:16000, language:"en", model:"base"}` → 二进制 PCM s16le 帧（100-500ms）→ `{type:"stop"}` / `{type:"cancel"}`
- 服务端 → 客户端：`{type:"ready"}` → `{type:"partial",text,committed}` / `{type:"committed",text,committed}`（增量）→ `{type:"done",committed_total}`

> **控制帧精确匹配**：`{type:"stop"}` / `{type:"cancel"}` 按 JSON `type` 字段精确匹配（非子串），含 "stop"/"cancel" 子串的任意文本不会误触发。
> **长会话 utterance 切分**：音频累积超 `buffer_max_sec`（默认 30s）时自动 flush + 切分（封顶 O(n²) 全量重解码）。此时 `done.committed_total` **仅含最后一段**，完整转录需客户端累加所有 `committed` 事件文本。短会话（<30s）不受影响，`done.committed_total` 即完整结果。

Python 客户端（`ws_stt_test.py`）：

```python
#!/usr/bin/env python3
"""STT 流式测试：把 wav 切 500ms PCM 块推给 WS，打印 partial/committed。"""
import asyncio, json, struct, wave, sys
import websockets

URI = "ws://localhost:8080/api/v1/stream/transcribe"
WAV = sys.argv[1] if len(sys.argv) > 1 else "jfk.wav"

async def main():
    wf = wave.open(WAV, "rb")
    assert wf.getframerate() == 16000 and wf.getnchannels() == 1, "需 16k mono wav"
    frames_per_chunk = int(wf.getframerate() * 0.5)  # 500ms
    # proxy=None 关键：绕过系统代理探测
    async with websockets.connect(URI, proxy=None, max_size=None) as ws:
        await ws.send(json.dumps({"type": "start", "sample_rate": 16000, "language": "en"}))
        print((await ws.recv()))  # ready

        async def feed():
            while True:
                raw = wf.readframes(frames_per_chunk)
                if not raw:
                    await ws.send(json.dumps({"type": "stop"}))
                    return
                await ws.send(raw)
                await asyncio.sleep(0.5)

        async def recv():
            async for msg in ws:
                obj = json.loads(msg)
                t = obj.get("type")
                if t == "committed":
                    print(f"[committed] {obj['text']}")
                elif t == "partial":
                    print(f"[partial]   {obj['text']}")
                elif t == "done":
                    print(f"[done] total={obj.get('committed_total')}")
                    return
                elif t == "error":
                    print(f"[error] {obj.get('message')}"); return

        await asyncio.gather(feed(), recv())

asyncio.run(main())
```

```bash
pip install websockets
python3 ws_stt_test.py ~/voice-cli-test/jfk.wav
```

---

## 5. TTS — 同步合成

```bash
curl -s -X POST http://localhost:8080/api/v1/tts \
  -H 'Content-Type: application/json' \
  -d '{"text":"你好世界，这是 Metal 加速的本地语音合成测试。","sid":0,"format":"wav"}' \
  -o /tmp/tts.wav
file /tmp/tts.wav       # RIFF WAVE mono 24kHz 16bit
open /tmp/tts.wav       # macOS 播放
```

请求体字段（`text` 必填，其余可选）：

| 字段 | 类型 | 说明 |
|---|---|---|
| `text` | string | 要合成的文本（≤`tts.max_text_length`） |
| `model` | string | 模型 id（不传=`default_model`；多模型并存时指定，如 `kokoro-multi-lang-v1_0`） |
| `sid` | i32 | 音色 id（0-52；不传=`default_sid`） |
| `voice` | string | 音色别名（v1 暂不解析，保留接口） |
| `speed` | f32 | 语速（1.0 原速；不传=`default_speed`） |
| `length_scale` | f32 | 时长缩放（model-level，仅首次加载生效） |
| `language` | string | 语言提示（`"zh"`/`"en"`，可选） |
| `format` | string | `wav`（默认，含 RIFF 头）/ `pcm_s16le`（裸 PCM） |

响应：二进制音频（`Content-Type: audio/wav`），24kHz mono s16le。

---

## 6. TTS — 音色列表

```bash
curl -s http://localhost:8080/api/v1/tts/voices | python3 -m json.tool
# {"model":"kokoro-multi-lang-v1_0","num_speakers":53}
```

---

## 7. TTS — 异步合成

```bash
# 提交（请求体字段同同步接口：text 必填，model/sid/speed/length_scale/language/format 可选）
TID=$(curl -s -X POST http://localhost:8080/api/v1/tasks/tts \
  -H 'Content-Type: application/json' \
  -d '{"text":"异步语音合成测试，使用 sherpa-onnx Kokoro 模型。","format":"wav"}' \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['task_id'])")
echo "task_id=$TID"

# 轮询状态（Pending → Processing → Completed）
sleep 2
curl -s "http://localhost:8080/api/v1/tasks/tts/$TID" | python3 -m json.tool

# 下载音频（Completed 后）
curl -s "http://localhost:8080/api/v1/tasks/tts/$TID/audio" -o /tmp/tts_async.wav
file /tmp/tts_async.wav
```

任务状态机：`Pending` → `Processing{stage}` → `Completed{audio_file_path,file_size,duration_seconds}` / `Failed{error}` / `Cancelled`。

---

## 8. TTS — 流式 WebSocket

协议：
- 客户端 → 服务端：首帧 JSON `{type:"start", text:"...", sid:0, speed:1.0, language:"zh", model:"kokoro-multi-lang-v1_0", format:"pcm_s16le"}`；`{type:"cancel"}` 或关闭结束
- 服务端 → 客户端：`{type:"ready",sample_rate}` → 二进制 PCM s16le 帧（多帧，增量）→ `{type:"done",total_samples,sample_rate}` / `{type:"error",message}`

> 流式只发**裸 PCM s16le**（无 WAV 头：流式无法预知总长）；`ready` 给采样率，客户端自行封装。

Python 客户端（`ws_tts_test.py`）：

```python
#!/usr/bin/env python3
"""TTS 流式测试：发 start+text，收增量 PCM 存 wav。"""
import asyncio, json, struct, wave
import websockets

URI = "ws://localhost:8080/api/v1/stream/tts"
TEXT = "你好世界，这是流式语音合成测试。"

async def main():
    async with websockets.connect(URI, proxy=None, max_size=None) as ws:
        await ws.send(json.dumps({
            "type": "start", "text": TEXT, "sid": 0, "speed": 1.0,
            "language": "zh", "model": "kokoro-multi-lang-v1_0", "format": "pcm_s16le",
        }))
        sr = None
        pcm = bytearray()
        async for msg in ws:
            if isinstance(msg, bytes):
                pcm.extend(msg)
                print(f"[audio] +{len(msg)} bytes (total {len(pcm)})")
            else:
                obj = json.loads(msg)
                t = obj.get("type")
                if t == "ready":
                    sr = obj["sample_rate"]; print(f"[ready] sr={sr}")
                elif t == "done":
                    print(f"[done] samples={obj['total_samples']}")
                    break
                elif t == "error":
                    print(f"[error] {obj['message']}"); return
        # 写 wav
        with wave.open("tts_stream.wav", "wb") as w:
            w.setnchannels(1); w.setsampwidth(2); w.setframerate(sr or 24000)
            w.writeframes(bytes(pcm))
        print(f"saved tts_stream.wav ({len(pcm)} bytes PCM)")

asyncio.run(main())
```

```bash
python3 ws_tts_test.py
open tts_stream.wav
```

---

## 9. 任务管理（通用）

```bash
# 查任意任务（STT/TTS）状态
curl -s "http://localhost:8080/api/v1/tasks/<task_id>" | python3 -m json.tool

# 取消
curl -s -X POST "http://localhost:8080/api/v1/tasks/<task_id>/cancel"

# 重试
curl -s -X POST "http://localhost:8080/api/v1/tasks/<task_id>/retry"

# 删除（TTS 会一并删音频文件）
curl -s -X DELETE "http://localhost:8080/api/v1/tasks/<task_id>"

# STT 任务统计
curl -s "http://localhost:8080/api/v1/tasks/stats" | python3 -m json.tool

# TTS 任务统计（对称 STT，查 tts_task_info 表）
curl -s "http://localhost:8080/api/v1/tasks/tts/stats" | python3 -m json.tool
```

---

## 10. 一键集成测试脚本

```bash
#!/usr/bin/env bash
# run_all_tests.sh —— 跑通 STT 同步/异步 + TTS 同步/异步/voices
set -e
BASE=http://localhost:8080
WAV=~/voice-cli-test/jfk.wav

echo "=== health ==="
curl -sf $BASE/health && echo

echo "=== STT 同步 ==="
curl -sf -F file=@$WAV -F language=en $BASE/transcribe | python3 -m json.tool

echo "=== STT 异步 ==="
TID=$(curl -sf -F file=@$WAV -F language=en $BASE/api/v1/tasks/transcribe \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['task_id'])")
sleep 3
curl -sf "$BASE/api/v1/tasks/$TID/result" | python3 -m json.tool

echo "=== TTS voices ==="
curl -sf $BASE/api/v1/tts/voices | python3 -m json.tool

echo "=== TTS 同步 ==="
curl -sf -X POST $BASE/api/v1/tts -H 'Content-Type: application/json' \
  -d '{"text":"集成测试，你好世界。","format":"wav"}' -o /tmp/tts.wav
file /tmp/tts.wav

echo "=== TTS 异步 ==="
TTID=$(curl -sf -X POST $BASE/api/v1/tasks/tts -H 'Content-Type: application/json' \
  -d '{"text":"异步合成。","format":"wav"}' \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['task_id'])")
sleep 3
curl -sf "$BASE/api/v1/tasks/tts/$TTID" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d['data']['status'])"
curl -sf "$BASE/api/v1/tasks/tts/$TTID/audio" -o /tmp/tts_async.wav
file /tmp/tts_async.wav

echo "=== ALL PASSED ==="
```

---

## 11. 常见接口错误

| HTTP | 场景 | 原因 |
|---|---|---|
| 403 | `/api/v1/tts*` | `tts.enabled=false`（改 config.yml） |
| 400 | `/transcribe` | 音频无流/损坏、格式不支持、超 `max_file_size` |
| 404 | `/api/v1/tasks/{id}` | task_id 不存在或已过期清理 |
| 409 | `/api/v1/tasks/{id}/audio` | 任务未 Completed |
| 500 | TTS 崩溃 | lexicon 组合错（见 DEPLOYMENT.md §7）/ 文本含 NUL |
