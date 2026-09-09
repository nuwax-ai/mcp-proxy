use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

//http返回的结构,data的结构一般是: CodeScriptExecutionResult,就是代码脚本的执行结果
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RunCodeHttpResult {
    //js 结果,json_value
    pub data: Value,
    // 是否执行成功,ture:默认值,执行成功
    pub success: bool,
    //如果执行错误的话,错误日志
    pub error: Option<String>,
}
