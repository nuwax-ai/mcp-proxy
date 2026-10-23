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
    pub fn with_timeout(future: F, timeout_seconds: u64) -> Self {
        let timeout = sleep(Duration::from_secs(timeout_seconds));

        Self { timeout, future }
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

/// 默认执行超时（秒），与 [`CommandExecutor::default`] 保持一致
const DEFAULT_TIMEOUT_SECS: u64 = 180;

/// 执行子命令，超时终止**整个进程组**。
///
/// 背景：`kill_on_drop` 只对直接子进程生效；python 路径下 `uv run` 会再拉起
/// Python 解释器作为孙进程，超时若只杀 uv，孙进程会被 reparent 给 init 继续
/// 运行（对执行不可信代码的服务是资源事故）。Unix 上让子进程 `process_group(0)`
/// 自立进程组（pid 即 pgid），超时对整组 SIGKILL 并收尸；Windows 无进程组
/// 语义，维持 kill_on_drop 仅终止直接子进程的现状（uv 在 Windows 不派生
/// 中间进程，影响面小）。
pub async fn run_command_with_timeout(
    mut command: tokio::process::Command,
    timeout_seconds: Option<u64>,
) -> io::Result<std::process::Output> {
    #[cfg(unix)]
    {
        // tokio Command 原生提供 process_group（Unix）；子进程自立进程组，pid 即 pgid
        command.process_group(0);
    }
    command.kill_on_drop(true);
    // output() 会自动管道化，spawn+wait_with_output 不会——必须显式 piped，
    // 否则子进程输出直接打到终端，管道捕获为空
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let timeout = Duration::from_secs(timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECS));
    let child = command.spawn()?;
    // wait_with_output 会消耗 child，先捕获 pid（process_group(0) 下 pid 即 pgid）
    let pgid = child.id();
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(result) => result,
        Err(_elapsed) => {
            kill_process_group(pgid);
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                // "timed out" 子串是既有测试契约（tests/*_with_timeout 断言），勿改
                format!(
                    "timed out: 命令执行超过 {}s，已终止进程组",
                    timeout.as_secs()
                ),
            ))
        }
    }
}

/// 对进程组发 SIGKILL（Unix）。负 pid 语义 = 整个进程组。
#[cfg(unix)]
fn kill_process_group(pgid: Option<u32>) {
    if let Some(pgid) = pgid {
        // SAFETY: libc::kill 为 C FFI 调用；负 pid 仅影响目标进程组，无其他副作用
        if unsafe { libc::kill(-(pgid as i32), libc::SIGKILL) } != 0 {
            warn!("进程组 SIGKILL 失败, pgid={pgid}");
        }
    }
}

#[cfg(not(unix))]
fn kill_process_group(_pid: Option<u32>) {}

/// 参数临时文件守卫：持有期间文件可用，任何退出路径（成功/失败/超时）的
/// Drop 都会清理目录——替代旧模式 `std::mem::forget(TempDir)` + 手动删除
/// （旧模式在错误/超时路径会泄漏含参数 JSON 的临时目录）。
pub struct ParamsTempFile {
    /// 仅存活守卫作用，不直接使用
    _dir: tempfile::TempDir,
    /// 参数 JSON 文件路径（设置到子进程 `INPUT_JSON_FILE`）
    pub path: std::path::PathBuf,
}

impl ParamsTempFile {
    pub fn create(params: &Value) -> Result<Self> {
        let dir = tempfile::TempDir::new()?;
        let path = dir.path().join("input_params.json");
        std::fs::write(&path, serde_json::to_string(params)?)?;
        Ok(Self { _dir: dir, path })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// 超时必须终止**整个进程组**：`sh -c 'sleep 30 & wait'` 中 sh 是直接
    /// 子进程、sleep 是孙进程——只杀直接子进程会让 sleep 孤儿化继续运行
    /// （生产上对应 uv 超时后 Python 孙进程泄漏）。
    #[tokio::test]
    async fn timeout_kills_entire_process_group() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg("sleep 30 & wait");

        let err = run_command_with_timeout(cmd, Some(1)).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(err.to_string().contains("timed out"));

        // 组杀信号传递后，孙进程 sleep 30 不应存活（pgrep 退出码非 0 = 无匹配）
        sleep(Duration::from_millis(300)).await;
        let probe = std::process::Command::new("pgrep")
            .arg("-f")
            .arg("^sleep 30$")
            .output()
            .expect("pgrep 探针执行失败");
        assert!(
            !probe.status.success(),
            "超时后孙进程仍存活，进程组未被终止"
        );
    }

    /// ParamsTempFile 在错误路径（? 提前返回）也应清理目录，不泄漏参数文件
    #[tokio::test]
    async fn params_temp_file_cleans_on_error_path() {
        let guard = ParamsTempFile::create(&Value::String("secret".into())).unwrap();
        let dir = guard.path.parent().unwrap().to_path_buf();
        assert!(guard.path.exists());

        // 模拟错误路径：用 early-return 作用域结束触发 Drop
        let result: Result<()> = (|| {
            let _g = guard;
            anyhow::bail!("模拟执行失败")
        })();
        assert!(result.is_err());

        assert!(!dir.exists(), "TempDir 守卫 Drop 后目录应被清理");
    }
}
