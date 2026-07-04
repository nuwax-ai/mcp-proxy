// deno 运行ts脚本
use crate::deno_runner::common_runner::run_deno_script_with_params;
use crate::model::{CodeScriptExecutionResult, LanguageScript, RunCode};
use anyhow::Result;

#[derive(Default)]
pub struct TsRunner;

impl RunCode for TsRunner {
    async fn run_with_params(
        &self,
        code: &str,
        params: Option<serde_json::Value>,
        timeout_seconds: Option<u64>,
    ) -> Result<CodeScriptExecutionResult> {
        run_deno_script_with_params(
            code,
            params,
            timeout_seconds,
            LanguageScript::Ts,
            |c, show_logs| self.prepare_ts_code(c, show_logs),
        )
        .await
    }
}

impl TsRunner {
    /// 准备TypeScript代码，添加日志捕获和handler函数执行逻辑
    fn prepare_ts_code(&self, code: &str, show_logs: bool) -> String {
        let template = include_str!("../templates/ts_template.ts");

        template
            .replace("{{USER_CODE}}", code)
            .replace("{{SHOW_LOGS}}", &show_logs.to_string())
    }
}
