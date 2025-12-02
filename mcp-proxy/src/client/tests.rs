// MCP 客户端模块测试 - 集成测试

#[cfg(test)]
mod integration_tests {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::process::Command;
    use serde_json::json;
    
    /// 测试真实的 MCP 服务连接和通信
    #[tokio::test]
    #[ignore] // 默认忽略，因为需要真实的网络连接
    async fn test_real_mcp_service_communication() {
        let url = "https://testagent.xspaceagi.com/api/mcp/sse?ak=ak-11b1dba295b74e87b9b62ecb2cf43d0a";
        
        // 启动 mcp-proxy 进程
        let mut child = Command::new("mcp-proxy")
            .arg(url)
            .arg("-q") // 静默模式，避免stderr干扰
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn mcp-proxy");
        
        let mut stdin = child.stdin.take().expect("Failed to get stdin");
        let stdout = child.stdout.take().expect("Failed to get stdout");
        let mut reader = BufReader::new(stdout);
        
        // 1. 发送 initialize 请求
        let init_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "roots": {"listChanged": true},
                    "sampling": {}
                },
                "clientInfo": {
                    "name": "test-client",
                    "version": "1.0.0"
                }
            }
        });
        
        let init_msg = format!("{}\n", serde_json::to_string(&init_request).unwrap());
        stdin.write_all(init_msg.as_bytes()).await.expect("Failed to write init");
        stdin.flush().await.expect("Failed to flush");
        
        // 读取 initialize 响应
        let mut init_response = String::new();
        reader.read_line(&mut init_response).await.expect("Failed to read init response");
        println!("Initialize response: {}", init_response);
        
        let init_result: serde_json::Value = serde_json::from_str(&init_response)
            .expect("Failed to parse init response");
        assert_eq!(init_result["jsonrpc"], "2.0");
        assert_eq!(init_result["id"], 1);
        assert!(init_result["result"]["serverInfo"].is_object());
        
        // 2. 发送 initialized 通知
        let initialized_notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let init_notif_msg = format!("{}\n", serde_json::to_string(&initialized_notification).unwrap());
        stdin.write_all(init_notif_msg.as_bytes()).await.expect("Failed to write initialized");
        stdin.flush().await.expect("Failed to flush");
        
        // 等待一下让服务器处理
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        // 3. 发送 tools/list 请求
        let tools_request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });
        let tools_msg = format!("{}\n", serde_json::to_string(&tools_request).unwrap());
        stdin.write_all(tools_msg.as_bytes()).await.expect("Failed to write tools/list");
        stdin.flush().await.expect("Failed to flush");
        
        // 读取 tools/list 响应
        let mut tools_response = String::new();
        reader.read_line(&mut tools_response).await.expect("Failed to read tools response");
        println!("Tools/list response: {}", tools_response);
        
        let tools_result: serde_json::Value = serde_json::from_str(&tools_response)
            .expect("Failed to parse tools response");
        assert_eq!(tools_result["jsonrpc"], "2.0");
        assert_eq!(tools_result["id"], 2);
        assert!(tools_result["result"]["tools"].is_array());
        
        let tools = tools_result["result"]["tools"].as_array().unwrap();
        println!("Available tools count: {}", tools.len());
        
        // 4. 如果有工具，测试调用第一个工具
        if !tools.is_empty() {
            let first_tool = &tools[0];
            let tool_name = first_tool["name"].as_str().expect("Tool must have name");
            println!("Testing tool: {}", tool_name);
            
            // 构造一个合法的 tool call 请求
            // forex_exchange_rate 需要 currencyPair 参数
            let call_tool_request = json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": tool_name,
                    "arguments": {
                        "currencyPair": "USDCNY"
                    }
                }
            });
            
            let call_msg = format!("{}\n", serde_json::to_string(&call_tool_request).unwrap());
            stdin.write_all(call_msg.as_bytes()).await.expect("Failed to write tools/call");
            stdin.flush().await.expect("Failed to flush");
            
            // 读取 tools/call 响应
            let mut call_response = String::new();
            reader.read_line(&mut call_response).await.expect("Failed to read call response");
            println!("Tools/call response: {}", call_response);
            
            let call_result: serde_json::Value = serde_json::from_str(&call_response)
                .expect("Failed to parse call response");
            assert_eq!(call_result["jsonrpc"], "2.0");
            assert_eq!(call_result["id"], 3);
            
            // 验证返回结果
            if call_result["error"].is_object() {
                panic!("Tool call failed with error: {:?}", call_result["error"]);
            } else {
                // 验证结果结构
                assert!(call_result["result"].is_object(), "Result should be an object");
                let result = &call_result["result"];
                
                // 检查是否有错误标记
                if result["isError"].as_bool().unwrap_or(false) {
                    panic!("Tool returned error: {:?}", result["content"]);
                }
                
                // 验证成功的响应
                assert!(result["content"].is_array(), "Content should be an array");
                let content = result["content"].as_array().unwrap();
                assert!(!content.is_empty(), "Content should not be empty");
                
                // 验证汇率数据
                let first_content = &content[0];
                assert_eq!(first_content["type"], "text");
                let text = first_content["text"].as_str().expect("Should have text field");
                assert!(text.contains("Exchange Rate"), "Should contain exchange rate info");
                assert!(text.contains("USD/CNY"), "Should contain USD/CNY pair");
                
                println!("✅ Tool call successful! Response: {}", text);
            }
        }
        
        // 清理：关闭进程
        drop(stdin);
        let _ = child.wait().await;
    }
    
    /// 测试协议检测功能
    #[tokio::test]
    #[ignore] // 默认忽略，因为需要真实的网络连接
    async fn test_protocol_detection() {
        let url = "https://testagent.xspaceagi.com/api/mcp/sse?ak=ak-11b1dba295b74e87b9b62ecb2cf43d0a";
        
        let protocol = crate::server::protocol_detector::detect_mcp_protocol(url).await;
        assert!(protocol.is_ok());
        
        let protocol = protocol.unwrap();
        use crate::model::McpProtocol;
        assert_eq!(protocol, McpProtocol::Sse);
    }
}
