use crate::cache::CodeFileCache;
use crate::model::{CodeExecutor, CodeScriptExecutionResult, CommandExecutor, LanguageScript};
use anyhow::Result;
use log::{debug, error, info};
use serde_json::Value;
use tokio::fs;
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

    // 处理参数：统一使用临时文件传递
    let temp_input_path = if let Some(params) = params {
        let params_json = serde_json::to_string(&params)?;

        // 创建临时文件写入参数
        let temp_dir = tempfile::TempDir::new()?;
        let temp_file_path = temp_dir.path().join("input_params.json");

        // 写入参数到临时文件
        std::fs::write(&temp_file_path, params_json.as_bytes())?;

        // 保持TempDir存在（这样文件就不会被删除）
        std::mem::forget(temp_dir);

        // 设置环境变量指向临时文件
        execute_command.env("INPUT_JSON_FILE", &temp_file_path);
        debug!("使用临时文件传递参数，文件路径: {:?}", temp_file_path);

        Some(temp_file_path)
    } else {
        // 没有参数时设置空对象
        execute_command.env("INPUT_JSON", "{}");
        None
    };

    debug!("Deno命令[{:?}]: {:?}", lang, execute_command);

    let executor = match timeout_seconds {
        Some(timeout) => CommandExecutor::with_timeout(execute_command.output(), timeout),
        None => CommandExecutor::default(execute_command.output()),
    };
    info!("执行命令: {:?}", execute_command);

    let executor_result = executor.await;
    let output = match executor_result {
        Ok(cmd_result) => match cmd_result {
            Ok(output) => output,
            Err(e) => {
                error!("Deno命令执行失败 [{lang:?}]: {e:?}");
                return Err(e.into());
            }
        },
        Err(e) => {
            error!("Deno任务执行异常 [{lang:?}]: {e:?}");
            return Err(e.into());
        }
    };
    debug!("标准输出:\n{}", String::from_utf8_lossy(&output.stdout));
    debug!("错误输出:\n{}", String::from_utf8_lossy(&output.stderr));

    // 执行完成后删除临时文件和目录
    if let Some(temp_file_path) = temp_input_path {
        // 删除文件
        let _ = fs::remove_file(&temp_file_path).await;
        // 尝试删除父目录（如果为空）
        if let Some(parent) = temp_file_path.parent() {
            let _ = fs::remove_dir(parent).await;
        }
        debug!("已删除临时文件: {:?}", temp_file_path);
    }

    CodeExecutor::parse_execution_output(&output.stdout, &output.stderr).await
}
