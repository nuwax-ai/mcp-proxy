use crate::cache::CodeFileCache;
use crate::model::{
    CodeExecutor, CodeScriptExecutionResult, LanguageScript, ParamsTempFile,
    run_command_with_timeout,
};
use anyhow::Result;
use log::{debug, error, info};
use serde_json::Value;
use tokio::process::Command;

/// 通用的 Deno 脚本执行逻辑，供 JS/TS Runner 复用
pub async fn run_deno_script_with_params<F>(
    code: &str,
    params: Option<Value>,
    timeout_seconds: Option<u64>,
    lang: LanguageScript,
    prepare_code_fn: F,
) -> Result<CodeScriptExecutionResult>
where
    F: Fn(&str, bool) -> String,
{
    debug!("开始执行{lang:?}脚本...,执行参数: {params:?}");

    let hash = CodeFileCache::obtain_code_hash(code);
    let cache_exist = CodeFileCache::check_code_file_cache_exisht(&hash, &lang).await;

    let run_code_script_file_tuple = if cache_exist {
        let cache_code = CodeFileCache::get_code_file_cache(&hash, &lang).await;
        debug!("从缓存中读取代码:hash值 {:?}", hash);
        cache_code?
    } else {
        let wrapped_code = prepare_code_fn(code, true);
        CodeFileCache::save_code_file_cache(&hash, &wrapped_code, &lang).await?;
        let code_script_file_tuple = CodeFileCache::get_code_file_cache(&hash, &lang).await?;
        debug!("创建脚本缓存:hash值 {:?}", hash);
        code_script_file_tuple
    };

    let temp_path = run_code_script_file_tuple.1;

    let mut execute_command = Command::new("deno");
    execute_command
        .arg("run")
        .arg("--allow-net")
        .arg("--allow-env")
        .arg("--allow-read")
        .arg("--no-check")
        .arg("--v8-flags=--max-heap-size=512")
        .arg(&temp_path)
        .kill_on_drop(true);

    // 处理参数：统一使用临时文件传递；守卫存续到函数结束，
    // 成功/失败/超时任何退出路径都由 Drop 自动清理
    let _params_guard = if let Some(params) = params.as_ref() {
        let temp = ParamsTempFile::create(params)?;
        execute_command.env("INPUT_JSON_FILE", &temp.path);
        debug!("使用临时文件传递参数，文件路径: {:?}", temp.path);
        Some(temp)
    } else {
        // 没有参数时设置空对象
        execute_command.env("INPUT_JSON", "{}");
        None
    };

    debug!("Deno命令[{:?}]: {:?}", lang, execute_command);
    info!("执行命令: {:?}", execute_command);

    // 进程组执行 + 超时组杀
    let output = match run_command_with_timeout(execute_command, timeout_seconds).await {
        Ok(output) => output,
        Err(e) => {
            error!("Deno命令执行失败 [{lang:?}]: {e:?}");
            return Err(e.into());
        }
    };
    debug!("标准输出:\n{}", String::from_utf8_lossy(&output.stdout));
    debug!("错误输出:\n{}", String::from_utf8_lossy(&output.stderr));

    CodeExecutor::parse_execution_output(&output.stdout, &output.stderr).await
}
