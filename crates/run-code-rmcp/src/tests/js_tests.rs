#[cfg(test)]
#[allow(clippy::module_inception)]
mod js_tests {
    use anyhow::Result;
    use log::info;
    use serde_json::json;

    use crate::model::{CodeExecutor, LanguageScript};
    use crate::tests::test_utils::setup;

    #[tokio::test]
    async fn test_js_basic_execution() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/test_js.js")?;
        info!("读取测试脚本: test_js.js");

        // 执行脚本
        info!("开始执行JavaScript脚本...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Js, None).await?;
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
    async fn test_js_with_params() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/test_js_params.js")?;
        info!("读取测试脚本: test_js_params.js");

        // 准备参数
        let params = json!({
            "a": 10,
            "b": 20,
            "name": "测试用户"
        });
        info!("准备测试参数: {params:?}");

        // 执行脚本
        info!("开始执行JavaScript脚本...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Js, Some(params))
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

        // 验证返回值
        if let Some(result_val) = result.result {
            // 检查结果是否包含预期的字段
            let json_str = serde_json::to_string(&result_val)?;
            assert!(json_str.contains("sum"), "结果应包含 sum 字段");
            assert!(json_str.contains("greeting"), "结果应包含 greeting 字段");
            assert!(json_str.contains("message"), "结果应包含 message 字段");
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_js_object_result() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/rfunction_test1.js")?;
        info!("读取测试脚本: rfunction_test1.js");

        // 准备参数
        let params = json!({
            "a": 1,
            "b": 2
        });
        info!("准备测试参数: {params:?}");

        // 执行脚本
        info!("开始执行JavaScript脚本...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Js, Some(params))
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
        assert!(result.error.is_none(), "不应有错误");
        assert!(result.result.is_some(), "应有返回结果");

        // 验证返回值
        if let Some(result_val) = result.result {
            // 检查结果是否包含预期的字段
            let json_str = serde_json::to_string(&result_val)?;
            assert!(json_str.contains("message"), "结果应包含 message 字段");
            assert!(json_str.contains("3"), "message 字段应为 3");
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_js_with_timeout() -> Result<()> {
        // 初始化日志
        setup();

        // 创建一个会超时的JavaScript脚本
        let code = r#"
// 创建一个长时间运行的脚本
function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

async function handler(input) {
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
        info!("开始执行JavaScript脚本，设置2秒超时...");
        let start_time = std::time::Instant::now();
        let result =
            CodeExecutor::execute_with_params(code, LanguageScript::Js, Some(params), Some(2))
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

    #[tokio::test]
    async fn test_js_cow_say_hello() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/cow_say_hello.js")?;
        info!("读取测试脚本: cow_say_hello.js");

        // 准备参数
        let params = json!({
            "a": 5,
            "b": 7
        });
        info!("准备测试参数: {params:?}");

        // 执行脚本
        info!("开始执行JavaScript脚本...");
        let result =
            CodeExecutor::execute_with_params(&code, LanguageScript::Js, Some(params), None)
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

        // 验证返回值
        if let Some(result_val) = result.result {
            // 检查结果是否包含预期的字段
            let json_str = serde_json::to_string(&result_val)?;
            assert!(json_str.contains("message"), "结果应包含 message 字段");

            // 检查计算结果是否正确 (5 + 7 = 12)
            if let Some(obj) = result_val.as_object()
                && let Some(message) = obj.get("message")
            {
                assert_eq!(message, 12, "message 字段的值应为 12");
            }
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_js_import_deno_std() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/import_deno_std_example.js")?;
        info!("读取测试脚本: import_deno_std_example.js");

        // 准备参数
        let params = json!({
            "path": "test.txt"
        });
        info!("准备测试参数: {params:?}");

        // 执行脚本
        info!("开始执行JavaScript脚本...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Js, Some(params))
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

        // 验证返回值
        if let Some(result_val) = result.result {
            // 检查结果是否包含预期的字段
            let json_str = serde_json::to_string(&result_val)?;
            assert!(json_str.contains("message"), "结果应包含 message 字段");
            assert!(json_str.contains("pathInfo"), "结果应包含 pathInfo 字段");
            assert!(
                json_str.contains("fileContent"),
                "结果应包含 fileContent 字段"
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_js_import_jsr() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/import_jsr_example.js")?;
        info!("读取测试脚本: import_jsr_example.js");

        // 准备参数
        let params = json!({
            "a": 10,
            "b": 5,
            "expected": 15
        });
        info!("准备测试参数: {params:?}");

        // 执行脚本
        info!("开始执行JavaScript脚本...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Js, Some(params))
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

        // 验证返回值
        if let Some(result_val) = result.result {
            // 检查结果是否包含预期的字段
            let json_str = serde_json::to_string(&result_val)?;
            assert!(json_str.contains("message"), "结果应包含 message 字段");
            assert!(json_str.contains("results"), "结果应包含 results 字段");
            assert!(json_str.contains("summary"), "结果应包含 summary 字段");
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_js_local_module() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/import_local_module_example.js")?;
        info!("读取测试脚本: import_local_module_example.js");

        // 准备参数
        let params = json!({
            "order": {
                "items": [
                    { "name": "测试产品", "price": 200, "quantity": 2 }
                ],
                "taxRate": 0.05
            }
        });
        info!("准备测试参数: {params:?}");

        // 执行脚本
        info!("开始执行JavaScript脚本...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Js, Some(params))
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

        // 验证返回值
        if let Some(result_val) = result.result {
            // 检查结果是否包含预期的字段
            let json_str = serde_json::to_string(&result_val)?;
            assert!(json_str.contains("message"), "结果应包含 message 字段");
            assert!(json_str.contains("order"), "结果应包含 order 字段");
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_js_import_axios() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/import_axios_example.js")?;
        info!("读取测试脚本: import_axios_example.js");

        // 准备参数
        let params = json!({
            "url": "https://jsonplaceholder.typicode.com/todos/1"
        });
        info!("准备测试参数: {params:?}");

        // 执行脚本
        info!("开始执行JavaScript脚本...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Js, Some(params))
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

        // 验证返回值
        if let Some(result_val) = result.result {
            // 检查结果是否包含预期的字段
            let json_str = serde_json::to_string(&result_val)?;
            assert!(json_str.contains("message"), "结果应包含 message 字段");
            assert!(json_str.contains("data"), "结果应包含 data 字段");
            assert!(json_str.contains("timestamp"), "结果应包含 timestamp 字段");
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_js_import_esm() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/import_esm_module_example.js")?;
        info!("读取测试脚本: import_esm_module_example.js");

        // 准备参数
        let params = json!({
            "data": "测试数据",
            "salt": "test-salt-123"
        });
        info!("准备测试参数: {params:?}");

        // 执行脚本
        info!("开始执行JavaScript脚本...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Js, Some(params))
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

        // 验证返回值
        if let Some(result_val) = result.result {
            // 检查结果是否包含预期的字段
            let json_str = serde_json::to_string(&result_val)?;
            assert!(json_str.contains("message"), "结果应包含 message 字段");
            assert!(json_str.contains("result"), "结果应包含 result 字段");
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_js_import_lodash() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/import_lodash_example.js")?;
        info!("读取测试脚本: import_lodash_example.js");

        // 准备参数
        let params = json!({
            "data": [1, 2, 3, 4, 5, 6, 7, 8, 9],
            "chunkSize": 3
        });
        info!("准备测试参数: {params:?}");

        // 执行脚本
        info!("开始执行JavaScript脚本...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Js, Some(params))
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

        // 验证返回值
        if let Some(result_val) = result.result {
            // 检查结果是否包含预期的字段
            let json_str = serde_json::to_string(&result_val)?;
            assert!(json_str.contains("message"), "结果应包含 message 字段");
            assert!(
                json_str.contains("originalData"),
                "结果应包含 originalData 字段"
            );
            assert!(
                json_str.contains("processedData"),
                "结果应包含 processedData 字段"
            );
            assert!(json_str.contains("timestamp"), "结果应包含 timestamp 字段");
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_js_main_function() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/rfunction_js_test3.js")?;
        info!("读取测试脚本: rfunction_js_test3.js");

        // 准备参数
        let params = json!({
            "testKey": "测试值"
        });
        info!("准备测试参数: {params:?}");

        // 执行脚本
        info!("开始执行JavaScript脚本...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Js, Some(params))
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

        // 验证返回值
        if let Some(result_val) = result.result {
            // 检查结果是否包含预期的字段
            let json_str = serde_json::to_string(&result_val)?;
            assert!(json_str.contains("key"), "结果应包含 key 字段");
            assert!(json_str.contains("value"), "key 字段的值应为 value");

            // 检查JSON结构
            if let Some(obj) = result_val.as_object()
                && let Some(key_value) = obj.get("key")
            {
                assert_eq!(key_value, "value", "key 字段的值应为 value");
            }
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_js_large_parameters() -> Result<()> {
        // 初始化日志
        setup();

        // 从 fixtures 目录读取测试脚本
        let code = std::fs::read_to_string("fixtures/test_js_large_params.js")?;
        info!("读取测试脚本: test_js_large_params.js");

        // 生成大于2MB的文本参数
        fn generate_large_text(size_in_mb: usize) -> String {
            let chunk = "This is a test chunk of text designed to generate large parameters for testing the Argument list too long fix. ".repeat(100);
            let iterations = (size_in_mb * 1024 * 1024) / chunk.len();

            let mut result = String::new();
            for i in 0..iterations {
                result.push_str(&chunk);
                result.push_str(&format!("Iteration {}\n", i));
            }

            result
        }

        let large_text = generate_large_text(3); // 3MB
        info!(
            "生成了3MB大小的文本参数，实际大小: {:.2} MB",
            large_text.len() as f64 / 1024.0 / 1024.0
        );

        // 准备参数
        let params = json!({
            "largeText": large_text
        });
        info!(
            "准备测试参数，JSON大小: {:.2} MB",
            serde_json::to_string(&params).unwrap().len() as f64 / 1024.0 / 1024.0
        );

        // 执行脚本
        info!("开始执行JavaScript脚本，测试大参数处理...");
        let result =
            CodeExecutor::execute_with_params_compat(&code, LanguageScript::Js, Some(params))
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
        assert!(result.error.is_none(), "不应有错误，这表明大参数处理成功");
        assert!(result.result.is_some(), "应有返回结果");

        // 验证返回值包含成功处理大参数的信息
        if let Some(result_val) = result.result {
            let json_str = serde_json::to_string(&result_val)?;
            assert!(json_str.contains("success"), "结果应包含 success 字段");
            assert!(json_str.contains("true"), "success 应为 true");
            assert!(
                json_str.contains("Successfully processed large text parameter"),
                "结果应包含成功消息"
            );
            assert!(json_str.contains("sizeMB"), "结果应包含 sizeMB 字段");

            // 验证返回值是对象格式
            if let Some(obj) = result_val.as_object() {
                if let Some(success) = obj.get("success") {
                    assert_eq!(success, true, "success 字段的值应为 true");
                }
                if let Some(text_size) = obj.get("textSize") {
                    assert!(text_size.is_number(), "textSize 应为数字");
                    let size = text_size.as_u64().unwrap();
                    assert!(size > 2 * 1024 * 1024, "文本大小应大于2MB");
                }
            }
        }

        info!("大参数测试通过！临时文件解决方案有效。");
        Ok(())
    }
}
