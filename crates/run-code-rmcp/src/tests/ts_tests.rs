#[cfg(test)]
#[allow(clippy::module_inception)]
mod ts_tests {
    use anyhow::Result;
    use log::info;
    use serde_json::json;

    use crate::model::{CodeExecutor, LanguageScript};
    use crate::tests::test_utils::setup;

    #[tokio::test]
    async fn test_ts_basic_execution() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/test_ts.ts")?;
        info!("读取测试脚本: test_ts.ts");

        // 执行脚本
        info!("开始执行TypeScript脚本...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Ts, None).await?;
        info!("脚本执行完成, 日志: {:?}", result.logs);

        if let Some(error) = &result.error {
            info!("执行错误: {error}");
        } else {
            info!("脚本执行成功");
        }

        if let Some(result_val) = &result.result {
            info!("执行结果: {result_val:?}");
        } else {
            info!("无执行结果");
        }

        // 验证结果
        assert!(!result.logs.is_empty(), "日志不应为空");
        assert!(result.error.is_none(), "不应有错误");
        assert!(result.result.is_some(), "应有返回结果");

        Ok(())
    }

    #[tokio::test]
    async fn test_ts_with_params() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/test_ts_params.ts")?;
        info!("读取测试脚本: test_ts_params.ts");

        // 准备参数
        let params = json!({
            "a": 10,
            "b": 20,
            "name": "测试用户"
        });
        info!("准备测试参数: {params:?}");

        // 执行脚本
        info!("开始执行TypeScript脚本...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Ts, Some(params))
                .await?;
        info!("脚本执行完成, 日志: {:?}", result.logs);

        if let Some(error) = &result.error {
            info!("执行错误: {error}");
        } else {
            info!("脚本执行成功");
        }

        if let Some(result_val) = &result.result {
            info!("执行结果: {result_val:?}");
        } else {
            info!("无执行结果");
        }

        // 验证结果
        assert!(!result.logs.is_empty(), "日志不应为空");
        assert!(result.error.is_none(), "不应有错误");
        assert!(result.result.is_some(), "应有返回结果");

        // 验证返回值包含预期的字段
        if let Some(result_val) = result.result {
            // 结果应该是JSON对象（TypeScript对象被正确解析）
            assert!(result_val.get("sum").is_some(), "结果应包含 sum 字段");
            assert!(
                result_val.get("greeting").is_some(),
                "结果应包含 greeting 字段"
            );
            assert!(
                result_val.get("message").is_some(),
                "结果应包含 message 字段"
            );
            assert!(
                result_val.get("numbers").is_some(),
                "结果应包含 numbers 字段"
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_ts_type_checking() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/test_ts.ts")?;
        info!("读取测试脚本: test_ts.ts");

        // 执行脚本
        info!("开始执行TypeScript类型检查...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Ts, None).await?;
        info!("类型检查完成");

        if let Some(error) = &result.error {
            info!("类型检查错误: {error}");
        } else {
            info!("类型检查通过");
        }

        // 验证结果
        assert!(result.error.is_none(), "TypeScript 类型检查应通过");

        Ok(())
    }

    #[tokio::test]
    async fn test_ts_with_timeout() -> Result<()> {
        // 初始化日志
        setup();

        // 创建一个会超时的TypeScript脚本
        let code = r#"
// 创建一个长时间运行的脚本
function sleep(ms: number): Promise<void> {
  return new Promise<void>(resolve => setTimeout(resolve, ms));
}

async function handler(input: any): Promise<{result: string}> {
  console.log("开始执行耗时操作");
  
  // 这个操作会运行10秒钟
  for (let i = 0; i < 10; i++) {
    console.log(`已经执行了 ${i+1} 秒`);
    await sleep(1000); // 等待1秒
  }
  
  console.log("操作完成");
  return { result: "完成" };
}
"#;
        info!("创建了一个会运行10秒的测试脚本");

        // 准备参数
        let params = json!({
            "test": "超时测试"
        });
        info!("准备测试参数: {params:?}");

        // 设置2秒超时并执行脚本
        info!("开始执行TypeScript脚本，设置2秒超时...");
        let start_time = std::time::Instant::now();
        let result =
            CodeExecutor::execute_with_params(code, LanguageScript::Ts, Some(params), Some(2))
                .await;
        let elapsed = start_time.elapsed();

        // 检查是否在2-3秒内超时（给一点缓冲时间）
        info!("脚本执行耗时: {elapsed:?}");
        assert!(elapsed.as_secs() >= 2, "脚本应该至少运行2秒");
        assert!(elapsed.as_secs() < 4, "脚本应该在4秒内超时");

        // 检查是否返回超时错误
        match result {
            Ok(exec_result) => {
                if let Some(error) = exec_result.error {
                    info!("正确捕获到超时错误: {error}");
                    assert!(
                        error.contains("timed out")
                            || error.contains("TimedOut")
                            || error.contains("executor await error"),
                        "错误信息应该包含超时相关信息"
                    );
                } else {
                    panic!("应该捕获到超时错误，但脚本执行成功了");
                }
            }
            Err(e) => {
                // 使用特殊格式化获取完整错误链
                let full_error = format!("{e:#}");
                info!("捕获到错误: {full_error}");
                assert!(
                    full_error.contains("timed out"),
                    "完整错误链中应该包含'timed out'超时信息"
                );
            }
        }

        Ok(())
    }
}
