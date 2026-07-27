// deno 运行js脚本
use crate::deno_runner::common_runner::run_deno_script_with_params;
use crate::model::{CodeScriptExecutionResult, LanguageScript, RunCode};
use anyhow::Result;

#[derive(Default)]
pub struct JsRunner;

impl RunCode for JsRunner {
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
            LanguageScript::Js,
            |c, show_logs| self.prepare_js_code(c, show_logs),
        )
        .await
    }
}

impl JsRunner {
    /// 准备JavaScript代码，添加日志捕获和handler函数执行逻辑
    fn prepare_js_code(&self, code: &str, show_logs: bool) -> String {
        // 检查代码中是否包含ES模块特征
        let is_esm = {
            // 检查正式的 import/export 语句
            let has_import_export = code.contains("import ")
                || code.contains("export ")
                || code.contains("import{")
                || code.contains("export{");

            // 检查动态 import
            let has_dynamic_import = code.contains("import(");

            // 检查是否有 require - CommonJS 的标志
            let has_require = code.contains("require(");

            // 如果有 import/export 特征，或者动态 import，但没有 require，则判定为 ESM
            (has_import_export || has_dynamic_import) && !has_require
        };

        // 根据代码特征选择合适的模板
        let template = if is_esm {
            include_str!("../templates/js_template_es.js")
        } else {
            include_str!("../templates/js_template_normal.js")
        };

        // 替换模板中的占位符
        template
            .replace("{{USER_CODE}}", code)
            .replace("{{SHOW_LOGS}}", &show_logs.to_string())
    }
}
