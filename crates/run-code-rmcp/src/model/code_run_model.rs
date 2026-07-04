use std::{
    pin::Pin,
    task::{Context, Poll},
};

use anyhow::{Context as AnyHowContext, Result};
use log::{info, warn};
use pin_project::pin_project;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use tokio::{
    io,
    time::{Duration, Sleep, sleep},
};

use crate::{deno_runner::JsRunner, deno_runner::TsRunner, python_runner::PythonRunner};

///语言脚本,选择对应的语言脚本运行期
#[derive(Debug, Clone)]
pub enum LanguageScript {
    Js,
    Ts,
    Python,
}

impl LanguageScript {
    /// 获取文件后缀
    pub fn get_file_suffix(&self) -> &str {
        match self {
            LanguageScript::Js => ".js",
            LanguageScript::Ts => ".ts",
            LanguageScript::Python => ".py",
        }
    }
}

///执行结果,包含js/python 执行结果,和打印的log日志
#[derive(Debug, Serialize, Deserialize)]
pub struct CodeScriptExecutionResult {
    //js/python 执行结果
    pub result: Option<Value>,
    //js/python 打印的log日志
    pub logs: Vec<String>,
    // 是否执行成功,ture:默认值,执行成功
    #[serde(skip_serializing)]
    pub success: bool,
    //如果执行错误的话,错误信息
    #[serde(skip_serializing)]
    pub error: Option<String>,
}

///运行代码的抽象
#[allow(async_fn_in_trait)]
pub trait RunCode {
    ///运行代码并传递参数，可选设置超时时间
    async fn run_with_params(
        &self,
        code: &str,
        params: Option<serde_json::Value>,
        timeout_seconds: Option<u64>,
    ) -> Result<CodeScriptExecutionResult>;
}

/// 代码执行器
pub struct CodeExecutor;

impl CodeExecutor {
    /// 执行代码并传递参数，可选设置超时时间
    pub async fn execute_with_params(
        code: &str,
        language: LanguageScript,
        params: Option<serde_json::Value>,
        timeout_seconds: Option<u64>,
    ) -> Result<CodeScriptExecutionResult> {
        info!("开始执行代码... 语言[{language:?}],执行参数: {params:?}");
        match language {
            LanguageScript::Js => {
                JsRunner
                    .run_with_params(code, params, timeout_seconds)
                    .await
            }
            LanguageScript::Ts => {
                TsRunner
                    .run_with_params(code, params, timeout_seconds)
                    .await
            }
            LanguageScript::Python => {
                PythonRunner
                    .run_with_params(code, params, timeout_seconds)
                    .await
            }
        }
    }

    /// 兼容旧代码的方法，不指定超时时间
    pub async fn execute_with_params_compat(
        code: &str,
        language: LanguageScript,
        params: Option<serde_json::Value>,
    ) -> Result<CodeScriptExecutionResult> {
        Self::execute_with_params(code, language, params, None).await
    }

    /// 解析执行输出
    pub async fn parse_execution_output(
        stdout: &[u8],
        stderr: &[u8],
    ) -> Result<CodeScriptExecutionResult> {
        let stdout_str = String::from_utf8_lossy(stdout).to_string();
        let stderr_str = String::from_utf8_lossy(stderr).to_string();

        // 尝试从stdout中查找JSON输出
        let json_pattern = r#"\{"logs":\s*\[.*\],\s*"result":.*,\s*"error":.*\}"#;
        let re = Regex::new(json_pattern)?;

        if let Some(captures) = re.find(&stdout_str) {
            let json_str = captures.as_str();
            let parsed: serde_json::Value =
                serde_json::from_str(json_str).context("Failed to parse JSON output")?;

            // 从JSON中提取logs、result和error
            let logs = parsed["logs"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            // 处理结果，尝试解析JSON字符串
            let result = if parsed["result"].is_null() {
                None
            } else if let Some(result_str) = parsed["result"].as_str() {
                // 如果是字符串，尝试解析为JSON对象
                match serde_json::from_str::<Value>(result_str) {
                    Ok(json_value) => Some(json_value),
                    Err(_) => Some(Value::String(result_str.to_string())),
                }
            } else {
                // 其他类型（数字、布尔值等）直接使用
                Some(parsed["result"].clone())
            };

            let error = parsed["error"].as_str().map(String::from);

            return Ok(CodeScriptExecutionResult {
                logs,
                result,
                success: error.is_none(),
                error,
            });
        }

        // 如果没有找到结构化输出，返回原始输出
        Ok(CodeScriptExecutionResult {
            logs: if !stdout_str.is_empty() {
                vec![stdout_str]
            } else {
                vec![]
            },
            result: None,
            success: false,
            error: Some(format!("Failed to extract structured output: {stderr_str}")),
        })
    }
}

///使用 pin-project 实现一个代码执行器,参数:timeout 超时时间; command 执行命令; 以及内置限制 command命令执行的堆大小限制
#[pin_project]
pub struct CommandExecutor<F> {
    #[pin]
    timeout: Sleep,
    #[pin]
    future: F,
}

impl<F> CommandExecutor<F> {
    pub fn default(future: F) -> Self {
        let timeout = sleep(Duration::from_secs(180));

        Self { timeout, future }
    }

    pub fn with_timeout(future: F, timeout_seconds: u64) -> Self {
        let timeout = sleep(Duration::from_secs(timeout_seconds));

        Self { timeout, future }
    }

    #[allow(dead_code)]
    pub fn change_timeout(&mut self, timeout: Duration) {
        self.timeout = sleep(timeout);
    }
}

impl<F: Future> Future for CommandExecutor<F> {
    type Output = io::Result<F::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        //根据 heap_size ,通过setrlimit设置执行 command 的堆大小限制
        match this.future.poll(cx) {
            Poll::Ready(result) => Poll::Ready(Ok(result)),
            Poll::Pending => match this.timeout.poll(cx) {
                Poll::Ready(()) => {
                    warn!("执行命令超时");
                    Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "future timed out",
                    )))
                }
                Poll::Pending => Poll::Pending,
            },
        }
    }
}
