#!/usr/bin/env python3
"""voice-cli OpenAPI 接口文档校验（utoipa 生成）。

校验项：
1. spec 结构：openapi 版本 / info / paths / components.schemas / tags
2. spec 的 path 与实际路由一致性（按方法+路径比对）
3. 每个文档化的 GET 端点实打（health/models/tasks stats）
4. /transcribe 文档化字段（language/model/engine/initial_prompt/response_format）逐个实打
5. Swagger UI 页面 + 全部内嵌静态资源 200（编译期内嵌，内网可用——vendored feature 的收益）
6. Scalar 文档页可达
7. spec 里 engine 参数枚举覆盖全部 5 个引擎（sensevoice 迁移后不能漏）

用法：python3 openapi_check.py http://127.0.0.1:8099
（Windows GBK 控制台自动兼容；或 python3 -X utf8 显式指定）
"""
import io
import json
import sys
import urllib.request
import urllib.error

# Windows 控制台默认 GBK，编不了 ✅/❌ → 强制 UTF-8 输出（失败则降级 replace）
if sys.stdout.encoding and sys.stdout.encoding.lower() not in ("utf-8", "utf8"):
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
    sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8", errors="replace")

BASE = sys.argv[1].rstrip("/") if len(sys.argv) > 1 else "http://127.0.0.1:8099"
TIMEOUT = 180

# 与 src/server/routes.rs 对齐的真实路由（方法, 路径）
ROUTES = [
    ("GET", "/health"),
    ("GET", "/models"),
    ("POST", "/transcribe"),
    ("POST", "/api/v1/tasks/transcribe"),
    ("POST", "/api/v1/tasks/transcribeFromUrl"),
    ("GET", "/api/v1/tasks/{task_id}"),
    ("DELETE", "/api/v1/tasks/{task_id}"),
    ("GET", "/api/v1/tasks/{task_id}/result"),
    ("POST", "/api/v1/tasks/{task_id}/cancel"),
    ("POST", "/api/v1/tasks/{task_id}/retry"),
    ("GET", "/api/v1/tasks/stats"),
    ("GET", "/api/v1/tasks/tts/stats"),
    ("POST", "/api/v1/tts"),
    ("GET", "/api/v1/tts/voices"),
    ("POST", "/api/v1/tasks/tts"),
    ("GET", "/api/v1/tasks/tts/{task_id}"),
    ("GET", "/api/v1/tasks/tts/{task_id}/audio"),
    ("GET", "/api/v1/stream/transcribe"),
    ("GET", "/api/v1/stream/tts"),
]

ok, fail = [], []


def check(name, cond, detail=""):
    (ok if cond else fail).append(f"{name}{(' — ' + detail) if detail else ''}")
    print(f"  {'✅' if cond else '❌'} {name}" + (f" — {detail}" if detail and not cond else ""))


def get(path, base=BASE):
    req = urllib.request.Request(base + path)
    with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
        return r.status, r.read(), dict(r.headers)


def post_multipart(path, fields, files, base=BASE):
    """fields: {name: value}; files: {name: (filename, bytes)}"""
    boundary = "----openapicheck" + "x" * 20
    body = b""
    for k, v in fields.items():
        body += f"--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n".encode()
    for k, (fn, data) in files.items():
        body += (
            f"--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"; filename=\"{fn}\"\r\n"
            f"Content-Type: application/octet-stream\r\n\r\n"
        ).encode() + data + b"\r\n"
    body += f"--{boundary}--\r\n".encode()
    req = urllib.request.Request(
        base + path, data=body, headers={"Content-Type": f"multipart/form-data; boundary={boundary}"}
    )
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


print(f"\n=== OpenAPI 校验 @ {BASE} ===")

# ---------- 1. spec 结构 ----------
print("\n[1] spec 结构")
try:
    st, raw, _ = get("/api/docs/openapi.json")
    spec = json.loads(raw)
    check("spec 可获取且是合法 JSON", st == 200, f"status={st}")
except Exception as e:  # noqa: BLE001
    check("spec 可获取且是合法 JSON", False, str(e))
    print("\n❌ spec 拿不到，后续校验无法进行")
    sys.exit(1)

check("openapi 版本为 3.x", str(spec.get("openapi", "")).startswith("3."), spec.get("openapi", ""))
info = spec.get("info", {})
check("info.title 非空", bool(info.get("title")), info.get("title", ""))
check("info.version 非空", bool(info.get("version")), info.get("version", ""))
paths = spec.get("paths", {})
schemas = spec.get("components", {}).get("schemas", {})
tags = spec.get("tags", [])
# 19 个 (method, path) 路由注册，去重后 18 条 path（/{task_id} 上 GET+DELETE 共享一条）
EXPECTED_PATHS = 18
check(f"paths 数量 == {EXPECTED_PATHS}", len(paths) == EXPECTED_PATHS, f"实际 {len(paths)}")
check("components.schemas 数量 >= 19", len(schemas) >= 19, f"实际 {len(schemas)}")
check("tags 数量 >= 8", len(tags) >= 8, f"实际 {len(tags)}")

# ---------- 2. spec 路由 vs 真实路由 ----------
print("\n[2] spec 路由 vs 真实路由（routes.rs）")
spec_routes = {(m.upper(), p) for p, item in paths.items() for m in item if m != "parameters"}
missing = [r for r in ROUTES if r not in spec_routes]
check("真实路由全部出现在 spec 中", not missing, f"缺失: {missing}")
extra = sorted(spec_routes - set(ROUTES))
if extra:
    print(f"  ℹ️  spec 有但路由表未列: {extra}")

# ---------- 3. GET 端点实打 ----------
print("\n[3] 文档化 GET 端点实打")
for path in ["/health", "/models", "/api/v1/tasks/stats", "/api/v1/tasks/tts/stats"]:
    try:
        st, body, _ = get(path)
        payload = json.loads(body)
        check(f"GET {path}", st == 200 and payload.get("code") == "0000", f"status={st} body={body[:80]}")
    except Exception as e:  # noqa: BLE001
        check(f"GET {path}", False, str(e))

# ---------- 4. /transcribe 文档化字段 ----------
print("\n[4] /transcribe 文档化字段逐个实打")
wav = None
import tempfile, wave as wave_mod, math, struct, os

with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tf:
    wav_path = tf.name
# 生成 1s 16k mono 静音 wav（够触发 ffmpeg + 引擎链路；内容不重要，验的是字段被接受）
with wave_mod.open(wav_path, "wb") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(16000)
    w.writeframes(b"\x00\x00" * 16000)
wav = open(wav_path, "rb").read()

# 文档化字段组合：每个字段单独传，验证不报"未知字段"且能被接受
cases = [
    ("仅 file", {}),
    ("language=zh", {"language": "zh"}),
    ("language=auto", {"language": "auto"}),
    ("model=base", {"model": "base"}),
    ("engine=whisper", {"engine": "whisper"}),
    ("engine=sensevoice", {"engine": "sensevoice"}),
    ("engine=fireredasr2", {"engine": "fireredasr2"}),
    ("initial_prompt", {"initial_prompt": "以下是普通话语音"}),
    ("response_format=json", {"response_format": "json"}),
    ("response_format=text", {"response_format": "text"}),
    ("beam_size=3", {"beam_size": "3"}),
    ("temperature=0.2", {"temperature": "0.2"}),
]
for name, fields in cases:
    st, body = post_multipart("/transcribe", fields, {"file": ("t.wav", wav)})
    try:
        payload = json.loads(body)
        code = payload.get("code")
        # 成功 = 0000；字段被接受 = 分派到了引擎层、因模型未部署而 404
        #（错误体在 error 字段，兼容 message 字段两种格式）
        accepted = st == 200 and code == "0000"
        err_text = str(payload.get("error", "") or payload.get("message", ""))
        model_missing = (
            "not found" in err_text.lower()
            or "模型" in err_text
            or "ModelNotFound" in err_text
            or "不存在" in err_text
        )
        field_accepted = st in (200, 404, 400) and (accepted or model_missing)
        check(f"字段 {name}", field_accepted, f"status={st} body={body[:140]}")
    except Exception:  # noqa: BLE001
        check(f"字段 {name}", st == 200, f"status={st} body={body[:140]}")

# 未知字段应被忽略而非报错（向后兼容）
st, body = post_multipart("/transcribe", {"unknown_future_field": "x"}, {"file": ("t.wav", wav)})
check("未知字段被忽略（向后兼容）", st == 200, f"status={st} body={body[:120]}")

os.unlink(wav_path)

# ---------- 5. Swagger UI + 内嵌资源 ----------
print("\n[5] Swagger UI 与内嵌静态资源（编译期内嵌，内网可用）")
for path in ["/api/docs", "/api/docs/openapi.json"]:
    try:
        st, body, _ = get(path)
        check(f"GET {path}", st == 200, f"status={st}")
    except Exception as e:  # noqa: BLE001
        check(f"GET {path}", False, str(e))
for asset in ["swagger-ui.css", "swagger-ui-bundle.js", "swagger-ui-standalone-preset.js", "index.css", "favicon-32x32.png"]:
    try:
        st, body, _ = get(f"/api/docs/{asset}")
        check(f"内嵌资源 {asset}", st == 200 and len(body) > 100, f"status={st} size={len(body)}")
    except Exception as e:  # noqa: BLE001
        check(f"内嵌资源 {asset}", False, str(e))

# ---------- 6. Scalar ----------
print("\n[6] Scalar 文档页")
try:
    st, body, _ = get("/api/docs/scalar")
    check("GET /api/docs/scalar", st == 200, f"status={st}")
except Exception as e:  # noqa: BLE001
    check("GET /api/docs/scalar", False, str(e))

# ---------- 7. engine 枚举覆盖 ----------
print("\n[7] spec 中 engine 参数覆盖全部 5 引擎")
spec_text = json.dumps(spec, ensure_ascii=False)
engines = ["whisper", "sensevoice", "fireredasr2", "funasrnano", "qwen3asr"]
missing_engines = [e for e in engines if e not in spec_text]
check("engine 枚举 5 个全在 spec 中", not missing_engines, f"缺失: {missing_engines}")

# ---------- 汇总 ----------
print(f"\n=== 汇总：{len(ok)} 通过 / {len(fail)} 失败 ===")
if fail:
    print("失败项：")
    for f in fail:
        print(f"  ❌ {f}")
    sys.exit(1)
print("✅ OpenAPI 接口文档校验全部通过")
