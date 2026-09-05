# 自定义文件上传后端 API 参考（nuwax 风格）

document-parser 的自定义上传后端（`storage.custom_upload` 配置段 + 三个接口的 `upload_*` 请求参数）所对接的**用户系统文件接口契约**。本文档沉淀接口细节，便于部署对接与排障时查阅。

- 上传接口文档来源：https://nuwax.com/open-api-file-upload.html
- AK 签名下载接口文档来源：https://nuwax.com/open-api-file-ak.html

> 鉴权均使用 Bearer API Key（下文以 `ak-xxxxxx` 占位）。API Key 由用户系统（nuwax 开放平台）签发，通过请求参数或全局配置传入，**不要写入代码或提交到仓库**。

---

## 1. 文件上传接口

```
POST {base_url}/api/v1/file/upload?type=tmp|store
Headers:
  Authorization: Bearer ak-xxxxxx
Content-Type: multipart/form-data
  file: (binary)
```

| 参数 | 位置 | 必填 | 说明 |
|------|------|------|------|
| `type` | Query | 否 | 存储类型：`tmp` 临时文件；`store` 永久存储（document-parser 默认用 store） |
| `file` | Form | 是 | 文件二进制 |

### 响应

```json
{
  "code": "0000",
  "displayCode": "0000",
  "message": "success",
  "data": {
    "url": "https://testagent.example.com/api/f/s3/default/20260904/b2e9c0df....md",
    "key": "s3/default/20260904/b2e9c0df....md",
    "fileName": "uploaded.md",
    "mimeType": "text/markdown",
    "size": 250
  },
  "tid": "6828901788519942543",
  "success": true
}
```

- 成功判定：`code == "0000"`（HTTP 200 不代表业务成功，必须检查 code）
- 使用字段：`data.url`（文件访问地址）、`data.key`（服务端生成的唯一标识）
- `tid` 为跟踪标识，排障时提供

### 语义要点（对接方须知）

- **服务端自管文件 key**：上传方无法指定对象键 → document-parser 的 SHA-256 图片去重在自定义后端下失效（重复上传产生新文件）
- 响应可能比文档示例多字段（`id`/`tenantId`/`authRequired`/`storageType` 等），客户端按需取 `url`/`key`，多余字段忽略
- 实测部分部署返回的 `data.url` 带 `authRequired: true`（见下节）

### curl 示例

```bash
curl -X POST "https://testagent.example.com/api/v1/file/upload?type=store" \
  -H "Authorization: Bearer ak-xxxxxx" \
  -F "file=@/path/to/document.pdf"
```

---

## 2. AK 签名下载接口（文件访问 URL 换取）

私有存储部署下，上传返回的 `data.url` **需要用户登录态**才能访问（API Key 直接 GET 会得到 `4030 API 不存在或无权限`）。本接口用 API Key 把私有 URL 换成**带签名的临时公开 URL**（S3 预签名直连地址，无需登录态即可下载）。

```
GET {base_url}/api/v1/file/ak?fileUrl=<urlencoded 文件URL>
Headers:
  Authorization: Bearer ak-xxxxxx
```

| 参数 | 位置 | 必填 | 说明 |
|------|------|------|------|
| `fileUrl` | Query | 是 | 上传接口返回的 `data.url`（需 URL encode） |

### 响应

```json
{
  "code": "0000",
  "displayCode": "0000",
  "message": "success",
  "data": "https://s3-direct.example.com:9443/27cd8ffc85f980cfd45d001ef55de268.md",
  "tid": "6811821788520372213",
  "success": true
}
```

- `data` 直接就是签名后的 URL 字符串（不是对象）
- 签名 URL **无鉴权可下载**（实测：markdown 返回 `text/markdown` 原文、图片返回 `image/jpeg` 二进制，均 HTTP 200）
- 签名有时效（临时链接），过期后需重新换取

### curl 示例

```bash
# 换取签名 URL
SIGNED_URL=$(curl -s "https://testagent.example.com/api/v1/file/ak?fileUrl=https%3A%2F%2Ftestagent.example.com%2Fapi%2Ff%2Fs3%2Fdefault%2F20260904%2Fb2e9c0df....md" \
  -H "Authorization: Bearer ak-xxxxxx" | python3 -c "import json,sys; print(json.load(sys.stdin)['data'])")

# 无鉴权下载
curl -O "$SIGNED_URL"
```

---

## 3. 与 document-parser 的集成对照

| 接口概念 | document-parser 集成点 |
|----------|------------------------|
| `{base_url}` + 上传 path | 请求参数 `upload_base_url` / `upload_path`（path 默认 `/api/v1/file/upload`）；或全局 `storage.custom_upload.base_url` / `.path` |
| Bearer API Key | 请求参数 `upload_api_key`；或全局 `storage.custom_upload.api_key`；环境变量 `DOCUMENT_PARSER_CUSTOM_UPLOAD_API_KEY` |
| `type=tmp|store` | 请求参数 `upload_type`（默认 `store`） |
| 上传返回的 `data.url` | 存入任务 `oss_data.markdown_url`（markdown）/ 替换进 markdown 图片路径（图片） |
| 上传返回的 `data.key` | 存入任务 `oss_data.markdown_object_key`（溯源用） |
| AK 签名换取 | 已集成：`GET /tasks/{id}/markdown/download`（服务端代理）优先换签后下载，换签失败回退裸 GET（公开存储）；`GET /tasks/{id}/markdown/url?temp=true` 走换签返回临时签名 URL（`temporary:true`），`temp=false` 透传存储的原 URL |

### 触发方式（三个入口）

```
POST /api/v1/documents/upload?upload_base_url=...&upload_api_key=...     (Query 参数)
POST /api/v1/documents/uploadFromUrl   JSON body 带 upload_* 字段
POST /api/v1/documents/parse-sync?upload_base_url=...&upload_api_key=... (Query 参数)
```

出现任一 `upload_*` 参数即启用自定义后端（Fail Fast：base_url 不可解析返回 400）；
不传且全局未配置时走阿里云 OSS（现有行为不变）。

### 已验证行为（2026-09-04，测试环境实测）

- `/upload` 异步链路：图片 + markdown 均上传成功，`/tasks/{id}/markdown/url` 返回后端真实地址
- `/parse-sync`：PDF（MinerU）解析后图片上传、markdown 内路径替换成功
- `oss_data.bucket` 字段在自定义后端下存 `base_url`（溯源）
- 遗留限制：私有部署下 markdown 内的图片 URL 需登录态，外部消费者请配合第 2 节 AK 接口换签名访问
