//! 自定义文件上传后端客户端（nuwax 风格 REST API）
//!
//! 面向私有化部署场景：解析产物（图片、Markdown）不上传阿里云 OSS，
//! 而是调用用户自建系统的文件上传接口。v1 钉死 nuwax 开放平台契约：
//!
//! - `POST {base_url}{path}?type=tmp|store`（tmp=临时文件，store=永久存储）
//! - 请求头 `Authorization: Bearer ak-xxx`
//! - multipart/form-data，`file` 字段
//! - 响应 `{code:"0000", displayCode, message, success, tid,
//!          data:{url, key, fileName, mimeType, size, width, height}}`，
//!   `code == "0000"` 表示成功，使用 `data.url` / `data.key`
//!
//! 与 OSS 的语义差异（有意为之）：
//! - 服务端自管文件 key，调用方无法指定对象键 → 请求侧哈希去重失效；
//! - 无删除/存在性/预签名契约 → 本客户端不提供这些能力。
//!
//! 本客户端**不实现** [`crate::OssClientTrait`]（9 个方法中多数在 REST 契约下
//! 无映射），调用方（document-parser）通过枚举分派选择后端。

use crate::error::{OssError, Result};
use crate::utils::detect_mime_type;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

/// 上传存储类型，对应 nuwax `?type=` 查询参数
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CustomUploadType {
    /// 永久存储（默认；Markdown 引用的图片应长期可访问）
    #[default]
    Store,
    /// 临时文件（时效由服务端自管）
    Tmp,
}

impl CustomUploadType {
    /// 查询参数值
    pub fn as_query_value(&self) -> &'static str {
        match self {
            Self::Store => "store",
            Self::Tmp => "tmp",
        }
    }
}

/// nuwax 契约的默认上传路径（唯一权威定义，document-parser 侧引用此常量）
pub const DEFAULT_UPLOAD_PATH: &str = "/api/v1/file/upload";

/// nuwax 契约的 AK 签名换取路径
pub const DEFAULT_AK_PATH: &str = "/api/v1/file/ak";

/// 校验上传接口路径规则：必须以 `/` 开头，不得含 `?` 或 `#`
///
/// （query 参数由 `upload_type` / `fileUrl` 管理，不允许调用方从 path 夹带）
pub fn validate_upload_path(path: &str) -> std::result::Result<(), String> {
    if !path.starts_with('/') {
        return Err(format!("自定义上传 path 必须以 / 开头: {path}"));
    }
    if path.contains('?') || path.contains('#') {
        return Err(format!(
            "自定义上传 path 不得包含 ? 或 #（query 由 upload_type 管理）: {path}"
        ));
    }
    Ok(())
}

/// 自定义上传后端配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiUploadConfig {
    /// 服务基地址，如 `https://agent.example.com`（尾部的 `/` 会在校验时规整掉）
    pub base_url: String,
    /// 上传接口路径，默认 [`DEFAULT_UPLOAD_PATH`]；必须以 `/` 开头，不得含 `?` 或 `#`
    pub path: String,
    /// API Key（Bearer 明文；由调用方负责保管）
    pub api_key: String,
    /// 上传存储类型
    pub upload_type: CustomUploadType,
}

impl Default for ApiUploadConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            path: DEFAULT_UPLOAD_PATH.to_string(),
            api_key: String::new(),
            upload_type: CustomUploadType::Store,
        }
    }
}

impl ApiUploadConfig {
    /// 校验配置有效性（Fail Fast）
    pub fn validate(&self) -> Result<()> {
        if self.base_url.trim().is_empty() {
            return Err(OssError::config("自定义上传 base_url 不能为空"));
        }
        validate_upload_path(&self.path).map_err(OssError::config)?;
        Ok(())
    }

    /// 完整上传端点 URL（含 `?type=` 查询参数）
    pub fn endpoint_url(&self) -> String {
        format!(
            "{}{}?type={}",
            self.base_url.trim_end_matches('/'),
            self.path,
            self.upload_type.as_query_value()
        )
    }

    /// AK 换签接口路径：与上传路径同前缀推导
    ///
    /// `upload_path` 以 nuwax 契约后缀 `/api/v1/file/upload` 结尾时（含反代
    /// 前缀重映射场景，如 `/nuwax/api/v1/file/upload`），把该后缀替换为
    /// `/api/v1/file/ak`，保证同一部署里上传与换签打到同一前缀；
    /// 无法识别的 path 回退契约默认值。
    pub fn ak_path(&self) -> String {
        const UPLOAD_SUFFIX: &str = "/api/v1/file/upload";
        const AK_SUFFIX: &str = "/api/v1/file/ak";
        if let Some(prefix) = self.path.strip_suffix(UPLOAD_SUFFIX) {
            format!("{prefix}{AK_SUFFIX}")
        } else {
            DEFAULT_AK_PATH.to_string()
        }
    }

    /// 是否配置了 API Key（空值时不发送 Authorization 头）
    fn has_api_key(&self) -> bool {
        !self.api_key.trim().is_empty()
    }
}

/// nuwax 风格上传响应信封（v1 钉死此格式）
///
/// 成功判定以 `code == "0000"` 为准（契约文档钉死；`success` 键在网关类
/// 响应中可能缺省，不作为判据——见 CUSTOM_UPLOAD_API.md）。
#[derive(Debug, Deserialize)]
struct NuwaxEnvelope {
    /// 业务状态码，"0000" 表示成功
    code: String,
    #[serde(default)]
    message: String,
    /// 跟踪唯一标识（排障用）
    tid: Option<String>,
    data: Option<NuwaxFileData>,
}

/// nuwax 响应 data 字段（JSON 为 camelCase）
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NuwaxFileData {
    url: String,
    key: String,
}

/// 上传成功结果
#[derive(Debug, Clone)]
pub struct UploadedFile {
    /// 文件完整网络地址
    pub url: String,
    /// 服务端生成的文件唯一标识
    pub key: String,
}

/// 自定义上传后端客户端
pub struct ApiFileClient {
    config: ApiUploadConfig,
    http: reqwest::Client,
    /// 单次上传请求超时（请求级，覆盖 Client 级设置）
    timeout: Duration,
}

impl ApiFileClient {
    /// 默认单请求超时：5 分钟（与 document-parser 的上传超时对齐）
    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

    /// AK 换签单请求超时：轻量元数据 GET，不应长等（换签 + 回退下载有整体上界约束）
    const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(15);

    /// 创建客户端（自建 HTTP 客户端）
    pub fn new(config: ApiUploadConfig) -> Result<Self> {
        config.validate()?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| OssError::network(format!("创建 HTTP 客户端失败: {e}")))?;
        Ok(Self {
            config,
            http,
            timeout: Self::DEFAULT_TIMEOUT,
        })
    }

    /// 创建客户端（复用外部 HTTP 客户端；reqwest Client 内部为 Arc，clone 共享连接池）
    pub fn with_client(config: ApiUploadConfig, http: reqwest::Client) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            http,
            timeout: Self::DEFAULT_TIMEOUT,
        })
    }

    /// 核心实现：POST multipart 上传
    async fn post_multipart(
        &self,
        body: Vec<u8>,
        filename: &str,
        mime: Option<&str>,
    ) -> Result<UploadedFile> {
        let part = reqwest::multipart::Part::stream(body)
            .file_name(filename.to_string())
            .mime_str(mime.unwrap_or("application/octet-stream"))
            .map_err(|e| OssError::invalid_parameter(format!("无效的 MIME 类型: {e}")))?;
        let form = reqwest::multipart::Form::new().part("file", part);

        let mut request = self
            .http
            .post(self.config.endpoint_url())
            .timeout(self.timeout)
            .multipart(form);
        // api_key 为空时不发送 Authorization 头（无鉴权部署），
        // 避免产生畸形的 "Bearer "（尾部空 token）被严格网关拒绝
        if self.config.has_api_key() {
            request = request.bearer_auth(&self.config.api_key);
        }
        let response = request.send().await.map_err(|e| {
            OssError::network(format!(
                "自定义上传请求失败 ({}{}): {e}",
                self.config.base_url, self.config.path
            ))
        })?;

        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| OssError::network(format!("读取自定义上传响应失败: {e}")))?;

        if !status.is_success() {
            return Err(OssError::network(format!(
                "自定义上传后端返回 HTTP {status}: {}",
                truncate_for_log(&bytes, 512)
            )));
        }

        let envelope: NuwaxEnvelope = serde_json::from_slice(&bytes)
            .map_err(|e| OssError::sdk(format!("自定义上传后端响应解析失败: {e}")))?;

        if envelope.code != "0000" {
            return Err(OssError::sdk(format!(
                "自定义上传后端业务失败: code={}, message={}, tid={:?}",
                envelope.code, envelope.message, envelope.tid
            )));
        }

        let data = envelope
            .data
            .ok_or_else(|| OssError::sdk("自定义上传后端响应缺少 data 字段"))?;

        Ok(UploadedFile {
            url: data.url,
            key: data.key,
        })
    }

    /// 上传字节内容（Markdown 等内存产物）
    ///
    /// `filename` 会作为 multipart 文件名发送；服务端自管 key，
    /// 本客户端不发送对象键。
    pub async fn upload_bytes(
        &self,
        content: &[u8],
        filename: &str,
        content_type: Option<&str>,
    ) -> Result<UploadedFile> {
        self.post_multipart(content.to_vec(), filename, content_type)
            .await
    }

    /// 上传本地文件（图片等落盘产物）
    ///
    /// `filename` 缺省取本地文件名；MIME 类型按扩展名探测。
    pub async fn upload_file_by_path(
        &self,
        local_path: &str,
        filename: Option<&str>,
    ) -> Result<UploadedFile> {
        let path = Path::new(local_path);
        if !path.exists() {
            return Err(OssError::file_not_found(local_path));
        }
        let effective_filename = filename.map_or_else(
            || {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file")
                    .to_string()
            },
            ToOwned::to_owned,
        );
        let body = tokio::fs::read(path)
            .await
            .map_err(|e| OssError::io_error(format!("读取文件失败 {local_path}: {e}")))?;
        let mime = detect_mime_type(local_path);
        let mime = if mime == "application/octet-stream" {
            None
        } else {
            Some(mime)
        };
        self.post_multipart(body, &effective_filename, mime.as_deref())
            .await
    }

    /// AK 签名换取：把上传返回的（可能需要登录态的）文件 URL 换成带签名的临时公开 URL
    ///
    /// `GET {base_url}{DEFAULT_AK_PATH}?fileUrl=<urlencoded>`，响应信封的 `data`
    /// 直接是签名后的 URL 字符串。签名 URL 无需登录态即可下载，时效由服务端自管。
    pub async fn exchange_signed_url(&self, file_url: &str) -> Result<String> {
        let url = format!(
            "{}{}",
            self.config.base_url.trim_end_matches('/'),
            self.config.ak_path()
        );
        let mut request = self.http.get(&url).query(&[("fileUrl", file_url)]);
        if self.config.has_api_key() {
            request = request.bearer_auth(&self.config.api_key);
        }
        let response = request
            .timeout(Self::EXCHANGE_TIMEOUT)
            .send()
            .await
            .map_err(|e| {
                OssError::network(format!("自定义上传后端 AK 换签请求失败 ({url}): {e}"))
            })?;

        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| OssError::network(format!("读取 AK 换签响应失败: {e}")))?;

        if !status.is_success() {
            return Err(OssError::network(format!(
                "自定义上传后端 AK 换签返回 HTTP {status}: {}",
                truncate_for_log(&bytes, 512)
            )));
        }

        let envelope: NuwaxAkEnvelope = serde_json::from_slice(&bytes)
            .map_err(|e| OssError::sdk(format!("自定义上传后端 AK 换签响应解析失败: {e}")))?;
        if envelope.code != "0000" {
            return Err(OssError::sdk(format!(
                "自定义上传后端 AK 换签业务失败: code={}, message={}, tid={:?}",
                envelope.code, envelope.message, envelope.tid
            )));
        }
        envelope
            .data
            .filter(|u| !u.trim().is_empty())
            .ok_or_else(|| OssError::sdk("自定义上传后端 AK 换签响应缺少 data（签名 URL）"))
    }

    /// 下载文件（完整下载协议编排，供服务端代理下载使用）
    ///
    /// 流程：AK 换签（15s）→ 失败回退裸 GET 原 URL（公开存储部署仍可用）→
    /// GET（60s 请求级）→ 响应防御校验 → 大小限制 → 返回字节。
    ///
    /// 响应防御（防错误响应体被当作文件内容返回，即"静默数据污染"）：
    /// 1. content-type 含 `application/json` 或 `text/html` → 拒（网关错误信封/
    ///    SSO 登录页；文件正文不可能是这两种类型）；
    /// 2. 信封嗅探：body 可解析为 nuwax 信封（含必填 `code` 键）→ 必是错误响应
    ///    体（合法的 Markdown/图片几乎不可能带顶层 `code` 字段）→ 拒。
    ///
    /// 大小限制：`content_length()` 预检 + 流式累计双保险，超限即拒。
    pub async fn download_file(&self, file_url: &str, max_bytes: u64) -> Result<Vec<u8>> {
        // 1. AK 换签；失败回退裸 GET（公开存储）
        let effective_url = match self.exchange_signed_url(file_url).await {
            Ok(signed) => signed,
            Err(e) => {
                tracing::warn!(
                    "AK signed-URL exchange failed, falling back to direct GET \
                     (public storage only): {}",
                    e
                );
                file_url.to_string()
            }
        };

        // 2. GET（请求级 60s）
        let response = self
            .http
            .get(&effective_url)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| {
                OssError::network(format!("自定义上传后端下载失败 ({effective_url}): {e}"))
            })?;

        if !response.status().is_success() {
            return Err(OssError::network(format!(
                "自定义上传后端下载返回 HTTP {}: {}",
                response.status(),
                effective_url
            )));
        }

        // 3. content-type 防御
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_lowercase();
        if content_type.contains("application/json") || content_type.contains("text/html") {
            return Err(OssError::network(format!(
                "自定义上传后端返回的是 {content_type} 而非文件内容（多为错误信封或登录页），\
                 拒绝作为文件返回: {effective_url}"
            )));
        }

        // 4. 大小预检（Content-Length 可缺失，流式累计兜底）
        if let Some(len) = response.content_length()
            && len > max_bytes
        {
            return Err(OssError::network(format!(
                "自定义上传后端响应体过大: {len} > {max_bytes} bytes: {effective_url}"
            )));
        }

        // 5. 流式读取并累计（防御无 Content-Length 的超大响应）
        let mut response = response;
        let mut body: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| OssError::network(format!("读取下载响应失败: {e}")))?
        {
            if body.len() as u64 + chunk.len() as u64 > max_bytes {
                return Err(OssError::network(format!(
                    "自定义上传后端响应体超过大小上限 {max_bytes} bytes: {effective_url}"
                )));
            }
            body.extend_from_slice(&chunk);
        }

        // 6. 信封嗅探防御（content-type 正常但 body 是错误 JSON 信封的场景，
        //    如网关以 text/plain 返回 JSON 错误）
        if let Ok(envelope) = serde_json::from_slice::<NuwaxAkEnvelope>(&body)
            && envelope.code != "0000"
        {
            return Err(OssError::sdk(format!(
                "自定义上传后端返回了错误信封而非文件内容: code={}, message={}, tid={:?}",
                envelope.code, envelope.message, envelope.tid
            )));
        }

        Ok(body)
    }
}

/// AK 换签响应信封（`data` 直接是签名字符串，非对象）
#[derive(Debug, Deserialize)]
struct NuwaxAkEnvelope {
    /// 业务状态码，"0000" 表示成功（与上传信封一致，`success` 键不作判据）
    code: String,
    #[serde(default)]
    message: String,
    tid: Option<String>,
    data: Option<String>,
}

/// 截断字节数组用于错误日志（有损 UTF-8 容错）
fn truncate_for_log(bytes: &[u8], max: usize) -> String {
    if bytes.len() <= max {
        String::from_utf8_lossy(bytes).to_string()
    } else {
        format!("{}...(truncated)", String::from_utf8_lossy(&bytes[..max]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config(base_url: String, upload_type: CustomUploadType) -> ApiUploadConfig {
        ApiUploadConfig {
            base_url,
            path: "/api/v1/file/upload".to_string(),
            api_key: "ak-test-key".to_string(),
            upload_type,
        }
    }

    fn ok_response() -> serde_json::Value {
        serde_json::json!({
            "code": "0000",
            "displayCode": "0000",
            "message": "success",
            "data": {
                "url": "https://statics.example.com/store/abc123.png",
                "key": "store/abc123.png",
                "fileName": "abc123.png",
                "mimeType": "image/png",
                "size": 18,
                "width": 0,
                "height": 0
            },
            "tid": "7383371776087252690",
            "success": true
        })
    }

    #[tokio::test]
    async fn test_upload_ok_store() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/file/upload"))
            .and(query_param("type", "store"))
            .and(header("Authorization", "Bearer ak-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_response()))
            .expect(1)
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let uploaded = client
            .upload_bytes(b"hello", "test.md", Some("text/markdown"))
            .await
            .unwrap();

        assert_eq!(uploaded.url, "https://statics.example.com/store/abc123.png");
        assert_eq!(uploaded.key, "store/abc123.png");
    }

    #[tokio::test]
    async fn test_upload_ok_tmp() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/file/upload"))
            .and(query_param("type", "tmp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_response()))
            .expect(1)
            .mount(&server)
            .await;

        let client = ApiFileClient::new(test_config(server.uri(), CustomUploadType::Tmp)).unwrap();
        let uploaded = client
            .upload_bytes(b"hello", "test.md", Some("text/markdown"))
            .await
            .unwrap();
        assert!(!uploaded.url.is_empty());
    }

    #[tokio::test]
    async fn test_upload_business_fail() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "5000",
                "message": "quota exceeded",
                "tid": "t-999",
                "success": false
            })))
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let err = client
            .upload_bytes(b"hello", "test.md", None)
            .await
            .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("5000"), "错误应包含业务码: {msg}");
        assert!(msg.contains("quota exceeded"), "错误应包含 message: {msg}");
        assert!(msg.contains("t-999"), "错误应包含 tid: {msg}");
    }

    #[tokio::test]
    async fn test_upload_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let err = client
            .upload_bytes(b"hello", "test.md", None)
            .await
            .unwrap_err();
        assert!(err.is_network_error());
        assert!(err.to_string().contains("500"));
    }

    #[tokio::test]
    async fn test_upload_malformed_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let err = client
            .upload_bytes(b"hello", "test.md", None)
            .await
            .unwrap_err();
        assert!(matches!(err, OssError::Sdk(_)));
    }

    #[tokio::test]
    async fn test_upload_missing_data() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "0000",
                "success": true,
                "message": "ok",
                "data": null
            })))
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let err = client
            .upload_bytes(b"hello", "test.md", None)
            .await
            .unwrap_err();
        assert!(matches!(err, OssError::Sdk(_)));
        assert!(err.to_string().contains("data"));
    }

    #[tokio::test]
    async fn test_upload_file_by_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/file/upload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_response()))
            .expect(1)
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("a.png");
        std::fs::write(&file_path, b"fake png").unwrap();

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let uploaded = client
            .upload_file_by_path(file_path.to_str().unwrap(), None)
            .await
            .unwrap();
        assert!(!uploaded.url.is_empty());

        // 文件不存在
        let err = client
            .upload_file_by_path("/nonexistent/file.png", None)
            .await
            .unwrap_err();
        assert!(err.is_file_not_found());
    }

    #[test]
    fn test_config_validate() {
        // path 不以 / 开头
        let mut cfg = test_config("https://x.com".to_string(), CustomUploadType::Store);
        cfg.path = "api/v1/file/upload".to_string();
        assert!(cfg.validate().is_err());

        // path 含 ?
        cfg.path = "/api?x=1".to_string();
        assert!(cfg.validate().is_err());

        // path 含 #
        cfg.path = "/api#frag".to_string();
        assert!(cfg.validate().is_err());

        // base_url 为空
        cfg.path = "/api/v1/file/upload".to_string();
        cfg.base_url = "  ".to_string();
        assert!(cfg.validate().is_err());

        // 合法
        cfg.base_url = "https://x.com".to_string();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_endpoint_url() {
        // 尾部 / 规整
        let cfg = test_config("https://x.com/".to_string(), CustomUploadType::Store);
        assert_eq!(
            cfg.endpoint_url(),
            "https://x.com/api/v1/file/upload?type=store"
        );

        let cfg = test_config("https://x.com".to_string(), CustomUploadType::Tmp);
        assert_eq!(
            cfg.endpoint_url(),
            "https://x.com/api/v1/file/upload?type=tmp"
        );

        // 允许 base_url 带前缀路径（反代场景）
        let cfg = test_config(
            "https://gw.example.com/nuwax".to_string(),
            CustomUploadType::Store,
        );
        assert_eq!(
            cfg.endpoint_url(),
            "https://gw.example.com/nuwax/api/v1/file/upload?type=store"
        );
    }

    // ===== Review 修复补测 =====

    #[tokio::test]
    async fn test_upload_error_body_without_success_field() {
        // 网关类错误体缺 success 键：serde 不应在解析层失败，真实 code/message 必须带出
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "4030",
                "message": "API 不存在或无权限"
            })))
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let err = client
            .upload_bytes(b"hello", "test.md", None)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("4030"), "错误应包含真实业务码: {msg}");
        assert!(
            msg.contains("API 不存在或无权限"),
            "错误应包含 message: {msg}"
        );
    }

    #[tokio::test]
    async fn test_empty_api_key_omits_authorization_header() {
        // 空 api_key 不发送 Authorization 头（避免畸形 "Bearer "）：
        // 先挂"带 Authorization 头"的失败 mock（expect 0，命中即测试失败），
        // 再挂普通成功 mock（expect 1）——wiremock 按挂载顺序匹配
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/file/upload"))
            .and(header("Authorization", "Bearer"))
            .respond_with(ResponseTemplate::new(401))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/file/upload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_response()))
            .expect(1)
            .mount(&server)
            .await;

        let config = ApiUploadConfig {
            base_url: server.uri(),
            path: "/api/v1/file/upload".to_string(),
            api_key: String::new(),
            upload_type: CustomUploadType::Store,
        };
        let client = ApiFileClient::new(config).unwrap();
        let uploaded = client
            .upload_bytes(b"hello", "test.md", Some("text/markdown"))
            .await
            .unwrap();
        assert!(!uploaded.url.is_empty());
    }

    #[tokio::test]
    async fn test_exchange_signed_url_ok() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/file/ak"))
            .and(query_param(
                "fileUrl",
                "https://agent.example.com/api/f/s3/default/x.md",
            ))
            .and(header("Authorization", "Bearer ak-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "0000",
                "message": "success",
                "data": "https://s3-direct.example.com:9443/signed-token.md",
                "tid": "t-1",
                "success": true
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let signed = client
            .exchange_signed_url("https://agent.example.com/api/f/s3/default/x.md")
            .await
            .unwrap();
        assert_eq!(signed, "https://s3-direct.example.com:9443/signed-token.md");
    }

    #[tokio::test]
    async fn test_exchange_signed_url_business_fail() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "4030",
                "message": "未授权",
                "tid": "t-2",
                "success": false
            })))
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let err = client
            .exchange_signed_url("https://x/f.md")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("4030") && msg.contains("未授权"),
            "应带出 code/message: {msg}"
        );
    }

    // ===== 第二轮 review 修复补测 =====

    #[tokio::test]
    async fn test_upload_success_without_success_key() {
        // 契约钉死成功判定以 code=="0000" 为准：缺 success 键的成功体必须成功
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "0000",
                "data": {
                    "url": "https://statics.example.com/store/ok.png",
                    "key": "store/ok.png"
                },
                "tid": "t-ok"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let uploaded = client
            .upload_bytes(b"hello", "test.md", None)
            .await
            .unwrap();
        assert_eq!(uploaded.url, "https://statics.example.com/store/ok.png");
    }

    #[test]
    fn test_ak_path_derivation() {
        // 默认契约路径 → 默认 AK 路径
        let config = test_config("https://x.com".to_string(), CustomUploadType::Store);
        assert_eq!(config.ak_path(), "/api/v1/file/ak");

        // 反代前缀重映射：上传路径带前缀 → AK 路径同前缀
        let mut config = config;
        config.path = "/nuwax/api/v1/file/upload".to_string();
        assert_eq!(config.ak_path(), "/nuwax/api/v1/file/ak");

        // 完全自定义路径（无法识别）→ 回退默认
        config.path = "/custom/upload".to_string();
        assert_eq!(config.ak_path(), "/api/v1/file/ak");
    }

    #[tokio::test]
    async fn test_ak_path_prefix_remapped_in_exchange() {
        // 换签请求应打到与 upload_path 同前缀的 AK 路径
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nuwax/api/v1/file/ak"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "0000",
                "data": "https://signed.example.com/x.md",
                "success": true
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut config = test_config(server.uri(), CustomUploadType::Store);
        config.path = "/nuwax/api/v1/file/upload".to_string();
        let client = ApiFileClient::new(config).unwrap();
        let signed = client.exchange_signed_url("https://x/f.md").await.unwrap();
        assert_eq!(signed, "https://signed.example.com/x.md");
    }

    /// download_file 的 helper：换签成功后从签名 URL 下载
    async fn mock_exchange_ok(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/api/v1/file/ak"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "0000",
                "data": format!("{}/signed/content.md", server.uri()),
                "success": true
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn test_download_file_ok_markdown() {
        let server = MockServer::start().await;
        mock_exchange_ok(&server).await;
        Mock::given(method("GET"))
            .and(path("/signed/content.md"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/markdown")
                    .set_body_string("# title\n\n![](img.jpg)\n"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let body = client
            .download_file("https://backend.example.com/api/f/x.md", 1024 * 1024)
            .await
            .unwrap();
        assert!(body.starts_with(b"# title"));
    }

    #[tokio::test]
    async fn test_download_file_rejects_html_login_page() {
        // 重定向到 HTML 登录页（跟随重定向后 200 text/html）→ 拒绝
        let server = MockServer::start().await;
        mock_exchange_ok(&server).await;
        Mock::given(method("GET"))
            .and(path("/signed/content.md"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/login", server.uri()))
                    .insert_header("Content-Type", "text/html"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/login"))
            // wiremock 的 set_body_string 会用自己的 mime 覆盖 insert_header 的
            // Content-Type，因此用空 body 让 header 生效（防御检查在读 body 前）
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/html; charset=utf-8"),
            )
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let err = client
            .download_file("https://backend.example.com/api/f/x.md", 1024 * 1024)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("text/html"), "应拒绝 HTML: {err}");
    }

    #[tokio::test]
    async fn test_download_file_rejects_json_envelope_by_content_type() {
        let server = MockServer::start().await;
        mock_exchange_ok(&server).await;
        Mock::given(method("GET"))
            .and(path("/signed/content.md"))
            // 空 body + application/json：content-type 拦截发生在读 body 之前
            .respond_with(
                ResponseTemplate::new(200).insert_header("Content-Type", "application/json"),
            )
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let err = client
            .download_file("https://backend.example.com/api/f/x.md", 1024 * 1024)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("application/json"),
            "应拒绝 JSON 信封: {err}"
        );
    }

    #[tokio::test]
    async fn test_download_file_envelope_sniffing_catches_text_plain_json() {
        // content-type 正常但 body 是 JSON 错误信封（text/plain 返回 JSON）→ 信封嗅探兜住
        let server = MockServer::start().await;
        mock_exchange_ok(&server).await;
        Mock::given(method("GET"))
            .and(path("/signed/content.md"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/plain")
                    .set_body_string(r#"{"code":"4030","message":"session expired","tid":"t-x"}"#),
            )
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let err = client
            .download_file("https://backend.example.com/api/f/x.md", 1024 * 1024)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("4030"),
            "信封嗅探应带出业务码: {err}"
        );
    }

    #[tokio::test]
    async fn test_download_file_rejects_oversized_content_length() {
        let server = MockServer::start().await;
        mock_exchange_ok(&server).await;
        Mock::given(method("GET"))
            .and(path("/signed/content.md"))
            // 真实 body 超过 max_bytes：走流式累计超限路径
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/markdown")
                    .set_body_string("x".repeat(5000).as_str()),
            )
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let err = client
            .download_file("https://backend.example.com/api/f/x.md", 1024)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("过大") || err.to_string().contains("超过"),
            "应报大小超限: {err}"
        );
    }

    #[tokio::test]
    async fn test_download_file_exchange_fail_falls_back_to_direct_get() {
        // AK 接口 404（公开存储无 AK 接口）→ 回退裸 GET 原文件 URL 成功
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/file/ak"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/f/public.md"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/markdown")
                    .set_body_string("# public content"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let body = client
            .download_file(&format!("{}/api/f/public.md", server.uri()), 1024 * 1024)
            .await
            .unwrap();
        assert!(body.starts_with(b"# public content"));
    }

    #[tokio::test]
    async fn test_exchange_signed_url_missing_data() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": "0000",
                "success": true,
                "data": null
            })))
            .mount(&server)
            .await;

        let client =
            ApiFileClient::new(test_config(server.uri(), CustomUploadType::Store)).unwrap();
        let err = client
            .exchange_signed_url("https://x/f.md")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("data"));
    }
}
