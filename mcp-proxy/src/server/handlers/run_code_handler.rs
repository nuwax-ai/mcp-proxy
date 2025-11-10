use std::collections::HashMap;

use axum::{Json, response::IntoResponse};
use http::StatusCode;
use log::{debug, error, info};
use run_code_rmcp::{CodeExecutor, LanguageScript, RunCodeHttpResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AppError;

///代码运行请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCodeMessageRequest {
    //js运行参数
    pub json_param: HashMap<String, Value>,
    //运行的代码
    pub code: String,
    //前端生成的随机uid,用于查找websocket连接,发送执行过程中的log日志
    pub uid: String,
    pub engine_type: String,
}

impl RunCodeMessageRequest {
    //获取语言脚本
    pub fn get_language_script(&self) -> LanguageScript {
        match self.engine_type.as_str() {
            "js" => LanguageScript::Js,
            "ts" => LanguageScript::Ts,
            "python" => LanguageScript::Python,
            _ => LanguageScript::Js,
        }
    }
}

/// 执行js/ts/python代码,通过 uv/deno 命令方式执行
// #[axum::debug_handler]
pub async fn run_code_handler(
    Json(run_code_message_request): Json<RunCodeMessageRequest>,
) -> Result<impl IntoResponse, AppError> {
    //json_param: HashMap<String, Value> 转换为 json 对象 Value
    let params = match serde_json::to_value(run_code_message_request.json_param.clone()) {
        Ok(v) => v,
        Err(e) => {
            error!("run_code_handler参数序列化失败: {e:?}");
            return Err(AppError::from(e));
        }
    };

    //执行代码
    let code = run_code_message_request.code.clone();
    let language = run_code_message_request.get_language_script();

    debug!("run_code_handler language:{language:?}");
    debug!("run_code_handler code:{code:?}");
    debug!("run_code_handler params:{params:?}");
    let result = match CodeExecutor::execute_with_params(&code, language, Some(params), None).await
    {
        Ok(result) => result,
        Err(e) => {
            error!("run_code_handler执行失败: {e:?}");
            return Err(AppError::from(e));
        }
    };

    let data = match serde_json::to_value(&result) {
        Ok(data) => data,
        Err(e) => {
            error!("run_code_handler序列化结果失败: {e:?}");
            return Err(AppError::from(e));
        }
    };
    //打印结果
    info!("run_code_handler result:{:?}", &result.success);
    debug!("run_code_handler result:{:?}", &data);
    //返回结果,使用 RunCodeHttpResult 封装执行结果
    let run_code_http_result = RunCodeHttpResult {
        data,
        success: result.success,
        error: result.error.clone(),
    };
    let body = serde_json::to_string(&run_code_http_result)?;
    Ok((StatusCode::OK, body).into_response())
}
