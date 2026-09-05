//! 上传后端选择：请求级 `upload_*` 参数与全局 `storage.custom_upload` 默认值合并，
//! 解析出本次请求/任务使用的自定义上传端点（nuwax 风格 REST API）。
//!
//! 解析规则（Fail Fast）：
//! - 任一请求字段出现，但 base_url 无法解析（请求与全局均为空）→ [`AppError::Validation`]（400）
//! - 无任何请求字段且全局 base_url 为空 → `Ok(None)`（走 OSS，与现状完全一致）
//! - 请求字段逐项覆盖全局默认；path 兜底 nuwax 契约 `/api/v1/file/upload`

use crate::config::CustomUploadConfig;
use crate::error::AppError;
use crate::models::UploadEndpoint;
use serde::Deserialize;
use utoipa::ToSchema;

/// 三个入口共用的请求级上传参数
///
/// 各入口 DTO（Query/JSON）显式声明 4 个同名字段后转换为该结构；
/// 不使用 `#[serde(flatten)]`（serde_urlencoded 对 flatten 支持不可靠）。
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct UploadTargetParams {
    /// 自定义上传服务基地址（如 `https://agent.example.com`）
    #[serde(default)]
    pub upload_base_url: Option<String>,
    /// 上传接口路径，默认 `/api/v1/file/upload`
    #[serde(default)]
    pub upload_path: Option<String>,
    /// API Key（Bearer）
    #[serde(default)]
    pub upload_api_key: Option<String>,
    /// 上传存储类型："store"（默认）或 "tmp"
    #[serde(default)]
    #[schema(value_type = String, example = "store")]
    pub upload_type: Option<oss_client::CustomUploadType>,
}

impl UploadTargetParams {
    /// 是否携带了任一 upload 字段（出现即视为启用自定义上传）
    pub fn has_any(&self) -> bool {
        self.upload_base_url.is_some()
            || self.upload_path.is_some()
            || self.upload_api_key.is_some()
            || self.upload_type.is_some()
    }
}

/// 合并解析上传目标
///
/// 返回 `Ok(None)`：走 OSS（现状行为）；
/// 返回 `Ok(Some)`：走自定义上传后端；
/// 返回 `Err`（Validation）：参数矛盾或非法（调用方应映射为 400）。
pub fn resolve_upload_target(
    params: &UploadTargetParams,
    global: &CustomUploadConfig,
) -> Result<Option<UploadEndpoint>, AppError> {
    let request_base = non_empty(params.upload_base_url.as_deref());
    let global_base = non_empty(Some(global.base_url.as_str()));

    let base_url = match request_base.or(global_base) {
        Some(base) => base.trim_end_matches('/').to_string(),
        None => {
            if params.has_any() {
                // Fail Fast：携带了 upload_* 字段但没有目标地址
                return Err(AppError::Validation(
                    "请求携带 upload_* 参数，但未能解析出上传 base_url \
                     （请求与全局 storage.custom_upload.base_url 均未提供）"
                        .to_string(),
                ));
            }
            // 未启用：走 OSS
            return Ok(None);
        }
    };

    // base_url 必须是合法 http(s) URL、有主机名且不带 query/fragment
    // （校验规则与全局配置共用一份：CustomUploadConfig::validate_base_url）
    CustomUploadConfig::validate_base_url(&base_url).map_err(AppError::Validation)?;

    // path：请求覆盖全局，兜底 nuwax 契约默认值（规则共用 oss_client::validate_upload_path）
    let path = non_empty(params.upload_path.as_deref())
        .or_else(|| non_empty(Some(global.path.as_str())))
        .map_or_else(
            || oss_client::DEFAULT_UPLOAD_PATH.to_string(),
            ToOwned::to_owned,
        );
    oss_client::validate_upload_path(&path).map_err(AppError::Validation)?;

    let api_key = non_empty(params.upload_api_key.as_deref())
        .or_else(|| non_empty(Some(global.api_key.as_str())))
        .unwrap_or_default()
        .to_string();

    Ok(Some(UploadEndpoint {
        base_url,
        path,
        api_key,
        upload_type: params.upload_type.unwrap_or_default(),
    }))
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn global(base_url: &str) -> CustomUploadConfig {
        CustomUploadConfig {
            base_url: base_url.to_string(),
            api_key: "global-key".to_string(),
            path: "/api/v1/file/upload".to_string(),
        }
    }

    #[test]
    fn test_no_params_and_empty_global_returns_none() {
        let result = resolve_upload_target(&UploadTargetParams::default(), &global("")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_any_param_without_base_url_fails_fast() {
        let params = UploadTargetParams {
            upload_api_key: Some("ak-xxx".to_string()),
            ..Default::default()
        };
        let err = resolve_upload_target(&params, &global("")).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));

        // 只传 path 也一样
        let params = UploadTargetParams {
            upload_path: Some("/p".to_string()),
            ..Default::default()
        };
        assert!(resolve_upload_target(&params, &global("")).is_err());

        // 只传 type 也一样
        let params = UploadTargetParams {
            upload_type: Some(oss_client::CustomUploadType::Tmp),
            ..Default::default()
        };
        assert!(resolve_upload_target(&params, &global("")).is_err());
    }

    #[test]
    fn test_global_only_enables_custom_backend() {
        // 全局配置 base_url 后，不带字段的请求默认走自定义后端
        let result =
            resolve_upload_target(&UploadTargetParams::default(), &global("https://g.com"))
                .unwrap()
                .expect("应为 Some");
        assert_eq!(result.base_url, "https://g.com");
        assert_eq!(result.path, "/api/v1/file/upload");
        assert_eq!(result.api_key, "global-key");
        assert_eq!(result.upload_type, oss_client::CustomUploadType::Store);
    }

    #[test]
    fn test_request_overrides_global_field_by_field() {
        let params = UploadTargetParams {
            upload_base_url: Some("https://r.com/".to_string()),
            upload_path: Some("/custom/path".to_string()),
            upload_api_key: Some("req-key".to_string()),
            upload_type: Some(oss_client::CustomUploadType::Tmp),
        };
        let result = resolve_upload_target(&params, &global("https://g.com"))
            .unwrap()
            .expect("应为 Some");
        assert_eq!(result.base_url, "https://r.com"); // 尾 / 已去除
        assert_eq!(result.path, "/custom/path");
        assert_eq!(result.api_key, "req-key");
        assert_eq!(result.upload_type, oss_client::CustomUploadType::Tmp);

        // 部分覆盖：只传 base_url，其余取全局
        let params = UploadTargetParams {
            upload_base_url: Some("https://r2.com".to_string()),
            ..Default::default()
        };
        let result = resolve_upload_target(&params, &global("https://g.com"))
            .unwrap()
            .expect("应为 Some");
        assert_eq!(result.base_url, "https://r2.com");
        assert_eq!(result.path, "/api/v1/file/upload");
        assert_eq!(result.api_key, "global-key");
    }

    #[test]
    fn test_request_base_url_with_empty_global() {
        let params = UploadTargetParams {
            upload_base_url: Some("https://r.com".to_string()),
            upload_api_key: Some("k".to_string()),
            ..Default::default()
        };
        let result = resolve_upload_target(&params, &global(""))
            .unwrap()
            .expect("应为 Some");
        assert_eq!(result.base_url, "https://r.com");
        assert_eq!(result.path, oss_client::DEFAULT_UPLOAD_PATH);
        assert_eq!(result.api_key, "k");
    }

    #[test]
    fn test_invalid_path_rejected() {
        let params = UploadTargetParams {
            upload_base_url: Some("https://r.com".to_string()),
            upload_path: Some("no-leading-slash".to_string()),
            ..Default::default()
        };
        assert!(resolve_upload_target(&params, &global("")).is_err());

        let params = UploadTargetParams {
            upload_base_url: Some("https://r.com".to_string()),
            upload_path: Some("/p?x=1".to_string()),
            ..Default::default()
        };
        assert!(resolve_upload_target(&params, &global("")).is_err());
    }

    #[test]
    fn test_invalid_base_url_rejected() {
        let params = UploadTargetParams {
            upload_base_url: Some("ftp://r.com".to_string()),
            ..Default::default()
        };
        assert!(resolve_upload_target(&params, &global("")).is_err());

        let params = UploadTargetParams {
            upload_base_url: Some("not a url".to_string()),
            ..Default::default()
        };
        assert!(resolve_upload_target(&params, &global("")).is_err());
    }

    #[test]
    fn test_base_url_with_query_or_fragment_rejected() {
        // query：会被 endpoint_url 字符串拼接吞进错误位置，必须拒绝
        let params = UploadTargetParams {
            upload_base_url: Some("https://r.com?lang=1".to_string()),
            ..Default::default()
        };
        assert!(resolve_upload_target(&params, &global("")).is_err());

        // fragment（反代地址复制粘贴残留）
        let params = UploadTargetParams {
            upload_base_url: Some("https://gw.r.com/#/app".to_string()),
            ..Default::default()
        };
        assert!(resolve_upload_target(&params, &global("")).is_err());
    }
}
