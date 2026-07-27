use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize, Serializer, ser::SerializeStruct};

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct HttpResult<T> {
    pub code: String,
    pub message: String,
    pub data: Option<T>,
    pub tid: Option<String>,
    #[serde(skip)]
    pub success: bool,
}

impl<T> HttpResult<T> {
    pub fn success(data: T, tid: Option<String>) -> Self {
        HttpResult {
            code: "0000".to_string(),
            message: "成功".to_string(),
            data: Some(data),
            tid,
            success: true,
        }
    }

    pub fn error(code: &str, message: &str, tid: Option<String>) -> Self {
        HttpResult {
            code: code.to_string(),
            message: message.to_string(),
            data: None,
            tid,
            success: false,
        }
    }
}

impl<T: Serialize> Serialize for HttpResult<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("HttpResult", 5)?;
        state.serialize_field("code", &self.code)?;
        state.serialize_field("message", &self.message)?;
        state.serialize_field("data", &self.data)?;
        state.serialize_field("tid", &self.tid)?;
        let is_success = self.code == "0000";
        state.serialize_field("success", &is_success)?;
        state.end()
    }
}

impl<T: Serialize> IntoResponse for HttpResult<T> {
    fn into_response(self) -> Response {
        match serde_json::to_string(&self) {
            Ok(body) => (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                body,
            )
                .into_response(),
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CONTENT_TYPE, "text/plain")],
                "Internal Server Error",
            )
                .into_response(),
        }
    }
}
