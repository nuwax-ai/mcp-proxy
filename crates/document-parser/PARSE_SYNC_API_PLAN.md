# 同步文档解析接口开发分析 (Parse-Sync API)

> **用途**：本文档供开发 agent 使用，分析新增一个**同步解析文档** HTTP 接口的方案。
> **背景**：当前 document-parser 所有真正调用解析引擎（MinerU/MarkItDown）的 HTTP 接口都是异步任务模式（`/upload`、`/uploadFromUrl` 返回 `task_id`）。为了**方便测试验证**，需要一个"上传文档 → 同一请求内同步返回 markdown 结果"的接口。
> **定位**：开发/测试辅助接口，**非生产大文件场景**。

---

## 1. 结论

**完全可行，改动量小。** 底层已有纯同步解析方法 `parse_document_local()`，HTTP 层只需组合「multipart 上传 + 调解析 + 包装响应」，不需要重写解析逻辑。

---

## 2. 可复用的现有资产

新接口应**直接复用**以下组件，禁止重复造轮子：

| 用途 | 现有资产 | 位置 |
|------|---------|------|
| **核心解析（同步、无 task、无 OSS）** | `DocumentService::parse_document_local(&self, file_path) -> AnyhowResult<ParseResult>` | `src/services/document_service.rs:187` |
| **multipart 流式上传 + 大小校验 + 格式自动检测** | `process_multipart_upload_streaming_with_task_id(multipart, config, max_size, task_id) -> Result<(file_path, filename, file_size, DocumentFormat), AppError>` | `src/handlers/document_handler.rs:370` |
| **文件大小 + 超时校验（已内置）** | `parse_document_local` 内部用 `GlobalFileSizeConfig` 和 `tokio::time::timeout(self.config.task_timeout, ...)` | `src/services/document_service.rs:198-226` |
| **临时文件清理** | `cleanup_temp_file(file_path: &str)` | `src/handlers/document_handler.rs:762` |
| **临时文件创建** | `create_temp_file_for_task(dir, task_id, filename)` | `src/handlers/document_handler.rs:527` |
| **统一响应包装** | `ApiResponse::success_with_status` / `ApiResponse::from_app_error` | `src/handlers/response.rs` |
| **上传配置** | `UploadConfig::with_global_config()`（含 `max_file_size`、`allowed_extensions`、`chunk_size`、`upload_timeout_secs`） | `src/handlers/document_handler.rs:52` |
| **同步 handler 模板（参考实现）** | `parse_markdown_sections`（`/markdown/parse`，签名与流程几乎一致） | `src/handlers/markdown_handler.rs:161` |
| **路由挂载点** | `document_routes()` | `src/routes.rs:45` |

> **关键**：`parse_document_local` 已自带超时和大小校验，新接口**无需再造超时逻辑**。

---

## 3. ParseResult 返回结构

定义在 `src/models/parse_result.rs:7`：

```rust
pub struct ParseResult {
    pub markdown_content: String,       // 核心结果：解析出的 markdown
    pub format: DocumentFormat,
    pub engine: ParserEngine,           // MinerU / MarkItDown
    pub processing_time: Option<f64>,   // 处理时间（秒）
    pub word_count: Option<usize>,
    pub error_count: Option<usize>,
    pub output_dir: Option<String>,     // MinerU 输出目录（图片所在）
    pub work_dir: Option<String>,
}
```

---

## 4. 推荐接口设计

### 4.1 HTTP 契约

```
POST /api/v1/documents/parse-sync
Content-Type: multipart/form-data

Query 参数：
  enable_toc      (bool, 可选)   是否生成结构化 TOC，默认 false
  max_toc_depth   (usize, 可选)  TOC 最大深度，默认 6

Form 字段：
  file            (必填)         要解析的文档文件（pdf/docx/xlsx/pptx/...）

响应：
  200  SyncParseResponse   解析成功，返回 markdown + 元信息
  400                     请求参数错误 / 文件格式不支持 / 文件过大（同步阈值）
  408                     解析超时
  413                     文件过大
  500                     服务器内部错误（含解析失败）
```

### 4.2 响应体结构（新建）

参考现有 `SectionsSyncResponse`（`src/handlers/markdown_handler.rs:79`）的风格：

```rust
/// 同步解析文档响应
#[derive(Debug, Serialize, ToSchema)]
pub struct SyncParseResponse {
    /// 解析得到的 Markdown 内容
    pub markdown_content: String,
    /// 文档格式（自动检测）
    pub format: DocumentFormat,
    /// 实际使用的解析引擎（MinerU / MarkItDown）
    pub engine: ParserEngine,
    /// 解析耗时（毫秒）
    pub processing_time_ms: u64,
    /// 字数统计
    pub word_count: Option<usize>,
    /// 原始文件名
    pub filename: String,
    /// 文件大小（字节）
    pub file_size: u64,
}
```

### 4.3 handler 流程（伪代码）

```rust
pub async fn parse_document_sync(
    State(state): State<AppState>,
    Query(params): Query<SyncParseQuery>,
    mut multipart: Multipart,
) -> Response {
    let start = std::time::Instant::now();

    // 1. 复用现成 multipart 上传：拿到 (file_path, filename, file_size, format)
    //    task_id 传一个随机 uuid（仅用于生成临时文件路径，不创建真实 task）
    let upload_config = UploadConfig::with_global_config();
    let (file_path, filename, file_size, _format) = match process_multipart_upload_streaming_with_task_id(
        &mut multipart, &upload_config, upload_config.max_file_size, &uuid::Uuid::new_v4().to_string()
    ).await {
        Ok(v) => v,
        Err(e) => return ApiResponse::from_app_error::<SyncParseResponse>(e).into_response(),
    };

    // 2. 同步解析（结果在任意分支都要清理临时文件 —— 用 guard 或 defer 模式）
    let result = parse_and_cleanup(&state, &file_path).await;

    // 3. 清理临时文件（成功/失败都要清）
    //    ⚠️ 推荐 RAII / Drop guard 模式，避免每个 error 分支重复写 cleanup

    // 4. 包装响应
    match result {
        Ok(parse_result) => {
            let response = SyncParseResponse {
                markdown_content: parse_result.markdown_content,
                format: parse_result.format,
                engine: parse_result.engine,
                processing_time_ms: start.elapsed().as_millis() as u64,
                word_count: parse_result.word_count,
                filename, file_size,
            };
            ApiResponse::success_with_status(response, StatusCode::OK).into_response()
        }
        Err(e) => ApiResponse::from_app_error::<SyncParseResponse>(e.into()).into_response(),
    }
}
```

---

## 5. 关键决策点（需确认）

以下 3 个决策会显著影响实现，**建议在开工前明确**：

### 决策 ①：图片处理策略（最关键）
MinerU 解析 PDF 会把图片输出到 `output_dir`，markdown 里引用的是**服务器本地绝对路径**。同步接口调用方拿到 markdown 后，图片是看不到的。

- **(A) 仅返回 markdown，图片路径保留为本地相对/绝对路径** —— 最简单，适合纯文本内容验证 ✅ **推荐**（符合"方便测试验证"定位）
- (B) 图片读出后 base64 内嵌进 markdown —— 自包含，但体积膨胀严重
- (C) 上传 OSS 替换路径 —— 违背"同步纯本地"初衷，引入 OSS 依赖与耗时

### 决策 ②：是否额外返回结构化文档（TOC/章节）
- **(A) 仅返回 `markdown_content`** —— 简单，够测试用 ✅ **推荐**
- (B) 额外调 `generate_structured_document_simple()` 一并返回 `StructuredDocument` —— 信息更全，但多一次处理、响应更大

### 决策 ③：文件大小阈值
全局 `max_file_size` 可能很大（生产配置）。同步接口长时间挂着连接解析大 PDF 风险高。

- 建议：给同步接口一个上限（默认 500MB，与全局默认一致），超过直接 `413` 拒绝（middleware 统一限制），引导走异步任务接口。
- 该阈值应做成可配置（config 或常量），而非硬编码在 handler 里。

---

## 6. 实施任务清单

- [ ] 新建 `SyncParseResponse` 结构（`src/handlers/document_handler.rs`，参考 `StructuredDocumentResponse`）
- [ ] 实现 `parse_document_sync` handler，复用：
  - `process_multipart_upload_streaming_with_task_id`
  - `state.document_service.parse_document_local`
  - `cleanup_temp_file`
- [ ] **临时文件清理**：保证 success 与所有 error 路径都清理（RAII guard 或每个分支显式调用），避免磁盘泄漏
- [ ] 错误处理：`anyhow::Error` → `AppError` 转换，加 `.context()` 补充上下文
- [ ] （决策③如选做）同步大小阈值校验 + 可配置化
- [ ] 路由注册：在 `src/routes.rs` 的 `document_routes()` 加 `.route("/parse-sync", post(document_handler::parse_document_sync))`
- [ ] OpenAPI：写 `#[utoipa::path]`，并在 `ApiDoc`（`src/lib.rs`）注册新 path 与 `SyncParseResponse` schema
- [ ] 单元测试：参考 `markdown_handler` 现有测试模式
- [ ] `cargo fmt && cargo clippy --all-targets --all-features` 通过

---

## 7. 项目规范约束（必须遵守）

来自 `CLAUDE.md` 与全局规范：

1. **禁止 `unwrap()` / `expect()`**（测试代码除外）。用 `?` 传播错误，加 `anyhow::Context`。
2. **Fail Fast**：错误尽早暴露，不要吞错。
3. **SOLID 原则**：handler 只做编排，解析逻辑已在 service 层，不要在 handler 里塞业务。
4. **HTTP 接口必须用 `utoipa` 写完整 OpenAPI 文档**。
5. **不用 `unsafe`**。
6. **4 空格缩进，行宽 100 字符**；所有 public API 有 `///` 文档注释。
7. 错误信息**不得包含敏感数据**。

---

## 8. 风险与注意事项

1. **长连接占用**：PDF 解析可能几十秒，HTTP 连接全程占用。仅限开发/测试，**禁止生产大文件场景**。接口命名与文档需注明"仅供测试验证"。
2. **网关超时**：若前置 nginx/网关，需保证其 `proxy_read_timeout` ≥ `task_timeout`，否则网关先断连。
3. **并发冲击**：MinerU 是重资源（GPU/CPU），同步接口并发请求会**直接打到 parser 池**，没有任务队列缓冲。建议在 handler 用 `Semaphore` 限流（如最多 2 路并发），避免把服务打挂。
4. **临时文件泄漏**：任一 error 分支漏清理都会让 `./temp` 堆积，最终磁盘打满 —— **强烈建议用 RAII guard**。
5. **`enable_toc` 参数当前设计为查询项**：若决策②选择不返回结构化文档，则该参数无实际作用，需移除或重新定义语义。

---

## 9. 验收标准

- [ ] 上传 PDF，同步返回完整 markdown 文本，引擎标识正确（MinerU）
- [ ] 上传 docx/xlsx 等，返回 markdown（MarkItDown 引擎）
- [ ] 超大文件（超同步阈值）→ 明确的 400 错误
- [ ] 不支持的格式 → 明确的 400 错误
- [ ] 解析失败/超时 → 500/408 且临时文件已清理
- [ ] OpenAPI 文档可在 `/api/docs` 正常渲染
- [ ] clippy 无 warning
