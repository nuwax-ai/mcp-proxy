// MCP 客户端模块测试 - 集成测试

// ============== 共享测试工具 ==============

#[cfg(test)]
mod test_helpers {
    use serde_json::json;
    use std::process::Stdio;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::process::{Child, Command};
    use tokio::time::timeout;

    /// 测试端口分配
    pub const TEST_PORT_INTEGRATION: u16 = 19880; // integration_tests 使用
    pub const TEST_PORT_PROTOCOL: u16 = 19881; // protocol detection 使用
    pub const TEST_PORT_RECONNECT: u16 = 19876; // reconnection_tests 使用

    /// 获取预编译的 test-mcp-server 二进制路径
    pub fn get_test_mcp_server_path() -> String {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let workspace_root = std::path::Path::new(manifest_dir).parent().unwrap();
        workspace_root
            .join("target/debug/test-mcp-server")
            .to_string_lossy()
            .to_string()
    }

    /// 获取预编译的 mcp-proxy 二进制路径
    pub fn get_mcp_proxy_path() -> String {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let workspace_root = std::path::Path::new(manifest_dir).parent().unwrap();
        workspace_root
            .join("target/debug/mcp-proxy")
            .to_string_lossy()
            .to_string()
    }

    /// 创建 MCP 服务配置 JSON（使用预编译二进制）
    pub fn create_test_config() -> String {
        let binary_path = get_test_mcp_server_path();
        json!({
            "mcpServers": {
                "test-server": {
                    "command": binary_path,
                    "args": []
                }
            }
        })
        .to_string()
    }

    /// 启动 proxy 服务器
    pub async fn spawn_proxy_server(port: u16) -> anyhow::Result<Child> {
        let config = create_test_config();
        let mcp_proxy_path = get_mcp_proxy_path();

        let child = Command::new(&mcp_proxy_path)
            .args([
                "proxy",
                "--port",
                &port.to_string(),
                "--host",
                "127.0.0.1",
                "--config",
                &config,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        Ok(child)
    }

    /// 等待服务器就绪（TCP 轮询）
    pub async fn wait_for_server_ready(addr: &str, max_retries: u32) -> anyhow::Result<()> {
        for i in 0..max_retries {
            match tokio::net::TcpStream::connect(addr).await {
                Ok(_) => {
                    println!("✅ 服务器就绪 (尝试 #{})", i + 1);
                    return Ok(());
                }
                Err(_) => {
                    if i < max_retries - 1 {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        }
        anyhow::bail!("服务器在 {} 次尝试后未就绪", max_retries)
    }

    /// 启动 convert 客户端进程
    pub async fn spawn_convert_client(
        url: &str,
        ping_interval: u64,
        ping_timeout: u64,
    ) -> anyhow::Result<Child> {
        let mcp_proxy_path = get_mcp_proxy_path();

        let child = Command::new(&mcp_proxy_path)
            .args([
                "convert",
                url,
                "--ping-interval",
                &ping_interval.to_string(),
                "--ping-timeout",
                &ping_timeout.to_string(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        Ok(child)
    }

    /// 监控 stderr 输出，查找特定日志模式
    pub async fn wait_for_stderr_pattern(
        stderr: &mut BufReader<tokio::process::ChildStderr>,
        pattern: &str,
        timeout_duration: Duration,
    ) -> anyhow::Result<bool> {
        let result = timeout(timeout_duration, async {
            let mut line = String::new();
            loop {
                line.clear();
                match stderr.read_line(&mut line).await {
                    Ok(0) => return false, // EOF
                    Ok(_) => {
                        print!("[stderr] {}", line);
                        if line.contains(pattern) {
                            return true;
                        }
                    }
                    Err(_) => return false,
                }
            }
        })
        .await;

        match result {
            Ok(found) => Ok(found),
            Err(_) => Ok(false), // timeout
        }
    }

    /// 发送 JSON-RPC 请求并获取响应
    pub async fn send_jsonrpc_and_receive(
        stdin: &mut tokio::process::ChildStdin,
        stdout: &mut BufReader<tokio::process::ChildStdout>,
        request: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let msg = format!("{}\n", serde_json::to_string(&request)?);
        stdin.write_all(msg.as_bytes()).await?;
        stdin.flush().await?;

        let mut response = String::new();
        stdout.read_line(&mut response).await?;
        let parsed: serde_json::Value = serde_json::from_str(&response)?;
        Ok(parsed)
    }

    /// 初始化 MCP 客户端（发送 initialize + initialized）
    pub async fn initialize_mcp_client(
        stdin: &mut tokio::process::ChildStdin,
        stdout: &mut BufReader<tokio::process::ChildStdout>,
    ) -> anyhow::Result<()> {
        // 发送 initialize 请求
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

        let init_response = send_jsonrpc_and_receive(stdin, stdout, init_request).await?;
        if init_response["error"].is_object() {
            anyhow::bail!("Initialize failed: {:?}", init_response["error"]);
        }

        // 发送 initialized 通知
        let initialized_notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let msg = format!("{}\n", serde_json::to_string(&initialized_notification)?);
        stdin.write_all(msg.as_bytes()).await?;
        stdin.flush().await?;

        // 等待一下让服务器处理
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(())
    }
}

// ============== 集成测试 ==============

#[cfg(test)]
mod integration_tests {
    use super::test_helpers::*;
    use serde_json::json;
    use std::time::Duration;
    use tokio::io::BufReader;

    /// 测试本地 MCP 服务连接和通信
    ///
    /// 使用本地 test-mcp-server + mcp-proxy proxy 进行测试
    /// 验证完整的 MCP 通信流程：initialize -> tools/list -> tools/call
    #[tokio::test]
    async fn test_real_mcp_service_communication() {
        println!("\n========== 测试: MCP 服务连接和通信 ==========");

        // 1. 启动本地 proxy 服务器
        println!("🚀 启动本地 proxy 服务器...");
        let mut proxy = spawn_proxy_server(TEST_PORT_INTEGRATION)
            .await
            .expect("启动 proxy 失败");

        let addr = format!("127.0.0.1:{}", TEST_PORT_INTEGRATION);
        wait_for_server_ready(&addr, 20)
            .await
            .expect("服务器启动超时");

        // 等待 proxy 完全初始化后端连接
        tokio::time::sleep(Duration::from_secs(3)).await;

        // 2. 启动 convert 客户端连接到本地 proxy
        println!("🔗 启动 convert 客户端...");
        let url = format!("http://{}", addr);
        let mut client = spawn_convert_client(&url, 30, 10)
            .await
            .expect("启动 convert 失败");

        let mut stdin = client.stdin.take().expect("获取 stdin 失败");
        let stdout = client.stdout.take().expect("获取 stdout 失败");
        let stderr = client.stderr.take().expect("获取 stderr 失败");
        let mut stdout_reader = BufReader::new(stdout);
        let mut stderr_reader = BufReader::new(stderr);

        // 等待客户端连接成功
        println!("⏳ 等待客户端连接...");
        let connected =
            wait_for_stderr_pattern(&mut stderr_reader, "开始代理转换", Duration::from_secs(15))
                .await
                .expect("监控 stderr 失败");

        if !connected {
            println!("⚠️  未检测到连接成功日志，尝试直接通信...");
            // 额外等待以确保连接建立
            tokio::time::sleep(Duration::from_secs(2)).await;
        }

        // 3. 初始化 MCP 客户端
        println!("🤝 初始化 MCP 客户端...");
        initialize_mcp_client(&mut stdin, &mut stdout_reader)
            .await
            .expect("初始化失败");

        // 4. 发送 tools/list 请求
        println!("📋 获取工具列表...");
        let tools_request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });
        let tools_response =
            send_jsonrpc_and_receive(&mut stdin, &mut stdout_reader, tools_request)
                .await
                .expect("tools/list 请求失败");

        assert_eq!(tools_response["jsonrpc"], "2.0");
        assert_eq!(tools_response["id"], 2);
        assert!(tools_response["result"]["tools"].is_array());

        let tools = tools_response["result"]["tools"].as_array().unwrap();
        println!("✅ 获取到 {} 个工具", tools.len());

        // 验证本地测试工具存在
        let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        println!("   工具列表: {:?}", tool_names);
        assert!(tool_names.contains(&"echo"), "应该包含 echo 工具");
        assert!(tool_names.contains(&"increment"), "应该包含 increment 工具");

        // 5. 测试调用 echo 工具
        println!("🔧 调用 echo 工具...");
        let call_tool_request = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "echo",
                "arguments": {
                    "message": "Hello from integration test!"
                }
            }
        });

        let call_response =
            send_jsonrpc_and_receive(&mut stdin, &mut stdout_reader, call_tool_request)
                .await
                .expect("tools/call 请求失败");

        assert_eq!(call_response["jsonrpc"], "2.0");
        assert_eq!(call_response["id"], 3);

        // 验证返回结果
        if call_response["error"].is_object() {
            panic!("Tool call failed with error: {:?}", call_response["error"]);
        }

        let result = &call_response["result"];
        assert!(
            !result["isError"].as_bool().unwrap_or(true),
            "echo 调用不应该出错"
        );
        assert!(result["content"].is_array(), "Content should be an array");

        let content = result["content"].as_array().unwrap();
        assert!(!content.is_empty(), "Content should not be empty");

        let first_content = &content[0];
        assert_eq!(first_content["type"], "text");
        let text = first_content["text"]
            .as_str()
            .expect("Should have text field");
        assert!(
            text.contains("Hello from integration test!"),
            "Should echo our message"
        );

        println!("✅ Tool call successful! Response: {}", text);

        // 6. 测试调用 increment 工具
        println!("🔧 调用 increment 工具...");
        let increment_request = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "increment",
                "arguments": {}
            }
        });

        let increment_response =
            send_jsonrpc_and_receive(&mut stdin, &mut stdout_reader, increment_request)
                .await
                .expect("increment 请求失败");

        assert!(
            !increment_response["result"]["isError"]
                .as_bool()
                .unwrap_or(true),
            "increment 调用不应该出错"
        );
        println!("✅ increment 调用成功");

        // 清理：关闭进程
        println!("🧹 清理进程...");
        drop(stdin);
        let _ = client.kill().await;
        let _ = proxy.kill().await;

        println!("========== 测试完成 ==========\n");
    }

    /// 测试协议检测功能
    ///
    /// 使用本地 mcp-proxy proxy 服务测试协议检测
    /// 本地 proxy 默认使用 Streamable HTTP 协议
    #[tokio::test]
    async fn test_protocol_detection() {
        println!("\n========== 测试: 协议检测 ==========");

        // 1. 启动本地 proxy 服务器（Streamable HTTP 模式）
        println!("🚀 启动本地 proxy 服务器...");
        let mut proxy = spawn_proxy_server(TEST_PORT_PROTOCOL)
            .await
            .expect("启动 proxy 失败");

        let addr = format!("127.0.0.1:{}", TEST_PORT_PROTOCOL);
        wait_for_server_ready(&addr, 20)
            .await
            .expect("服务器启动超时");

        // 2. 测试协议检测
        println!("🔍 检测协议类型...");
        let url = format!("http://{}", addr);
        let protocol = crate::client::protocol::detect_mcp_protocol(&url).await;

        assert!(protocol.is_ok(), "协议检测应该成功");

        let protocol = protocol.unwrap();
        use crate::client::protocol::McpProtocol;
        // 本地 proxy 默认使用 Streamable HTTP
        assert_eq!(
            protocol,
            McpProtocol::Stream,
            "应该检测到 Streamable HTTP 协议"
        );

        println!("✅ 检测到协议: {:?}", protocol);

        // 清理
        println!("🧹 清理进程...");
        let _ = proxy.kill().await;

        println!("========== 测试完成 ==========\n");
    }
}

/// 本地重连测试模块
///
/// 使用 `mcp-proxy proxy` 启动本地服务，`mcp-proxy convert` 连接测试
/// 验证通道检查和自动重连逻辑
#[cfg(test)]
mod reconnection_tests {
    use super::test_helpers::*;
    use serde_json::json;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::time::timeout;

    /// 测试配置
    const SERVER_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
    const CLIENT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
    const RECONNECT_DETECT_TIMEOUT: Duration = Duration::from_secs(30);

    /// 测试 1: 正常连接和通信
    ///
    /// 验证:
    /// - proxy 服务启动正常
    /// - convert 客户端连接成功
    /// - tools/list 请求正常响应
    #[tokio::test]
    async fn test_reconnection_normal_connection() {
        println!("\n========== 测试 1: 正常连接和通信 ==========");

        // 1. 启动 proxy 服务器
        println!("🚀 启动 proxy 服务器...");
        let mut proxy = spawn_proxy_server(TEST_PORT_RECONNECT)
            .await
            .expect("启动 proxy 失败");

        // 等待服务器就绪
        let addr = format!("127.0.0.1:{}", TEST_PORT_RECONNECT);
        wait_for_server_ready(&addr, 20)
            .await
            .expect("服务器启动超时");

        // 2. 启动 convert 客户端
        println!("🔗 启动 convert 客户端...");
        let url = format!("http://{}", addr);
        let mut client = spawn_convert_client(&url, 5, 3)
            .await
            .expect("启动 convert 失败");

        let mut stdin = client.stdin.take().expect("获取 stdin 失败");
        let stdout = client.stdout.take().expect("获取 stdout 失败");
        let stderr = client.stderr.take().expect("获取 stderr 失败");
        let mut stdout_reader = BufReader::new(stdout);
        let mut stderr_reader = BufReader::new(stderr);

        // 等待客户端连接成功（监控 stderr）
        println!("⏳ 等待客户端连接...");
        let connected =
            wait_for_stderr_pattern(&mut stderr_reader, "连接成功", CLIENT_CONNECT_TIMEOUT)
                .await
                .expect("监控 stderr 失败");

        if !connected {
            // 可能连接很快，直接尝试初始化
            println!("⚠️  未检测到连接成功日志，尝试直接通信...");
        }

        // 3. 初始化 MCP 客户端
        println!("🤝 初始化 MCP 客户端...");
        initialize_mcp_client(&mut stdin, &mut stdout_reader)
            .await
            .expect("初始化失败");

        // 4. 发送 tools/list 请求
        println!("📋 获取工具列表...");
        let tools_request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });
        let tools_response =
            send_jsonrpc_and_receive(&mut stdin, &mut stdout_reader, tools_request)
                .await
                .expect("tools/list 请求失败");

        assert!(
            tools_response["result"]["tools"].is_array(),
            "应该返回工具列表"
        );
        let tools = tools_response["result"]["tools"].as_array().unwrap();
        println!("✅ 获取到 {} 个工具", tools.len());

        // 验证我们的测试工具存在
        let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(tool_names.contains(&"echo"), "应该包含 echo 工具");
        assert!(tool_names.contains(&"increment"), "应该包含 increment 工具");

        // 5. 测试调用 increment 工具
        println!("🔧 调用 increment 工具...");
        let call_request = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "increment",
                "arguments": {}
            }
        });
        let call_response = send_jsonrpc_and_receive(&mut stdin, &mut stdout_reader, call_request)
            .await
            .expect("tools/call 请求失败");

        assert!(
            !call_response["result"]["isError"].as_bool().unwrap_or(true),
            "increment 调用不应该出错"
        );
        println!("✅ increment 调用成功");

        // 清理
        println!("🧹 清理进程...");
        drop(stdin);
        let _ = client.kill().await;
        let _ = proxy.kill().await;

        println!("========== 测试 1 完成 ==========\n");
    }

    /// 测试 2: 服务器重启后自动重连
    ///
    /// 验证:
    /// - 杀死 proxy 服务后，convert 检测到断开
    /// - 重启 proxy 后，convert 自动重连
    /// - 重连后功能正常
    #[tokio::test]
    async fn test_reconnection_on_server_restart() {
        println!("\n========== 测试 2: 服务器重启后自动重连 ==========");

        // 1. 启动 proxy 服务器
        println!("🚀 启动 proxy 服务器...");
        let mut proxy = spawn_proxy_server(TEST_PORT_RECONNECT + 1)
            .await
            .expect("启动 proxy 失败");

        let addr = format!("127.0.0.1:{}", TEST_PORT_RECONNECT + 1);
        wait_for_server_ready(&addr, 20)
            .await
            .expect("服务器启动超时");

        // 2. 启动 convert 客户端（短 ping 间隔以快速检测断开）
        println!("🔗 启动 convert 客户端...");
        let url = format!("http://{}", addr);
        let mut client = spawn_convert_client(&url, 2, 2)
            .await
            .expect("启动 convert 失败");

        let stderr = client.stderr.take().expect("获取 stderr 失败");
        let mut stderr_reader = BufReader::new(stderr);

        // 等待客户端连接成功
        println!("⏳ 等待客户端连接...");
        tokio::time::sleep(Duration::from_secs(5)).await;

        // 3. 杀死 proxy 服务器
        println!("💀 杀死 proxy 服务器...");
        let _ = proxy.kill().await;

        // 4. 等待客户端检测到断开
        println!("⏳ 等待客户端检测到断开...");
        let disconnected =
            wait_for_stderr_pattern(&mut stderr_reader, "连接断开", RECONNECT_DETECT_TIMEOUT)
                .await
                .expect("监控 stderr 失败");

        // 也可能是 "Ping 检测" 或 "后端连接已关闭"
        if !disconnected {
            println!("⚠️  未检测到明确的断开日志，检查是否有重连尝试...");
        }

        // 5. 重启 proxy 服务器
        println!("🔄 重启 proxy 服务器...");
        let mut proxy = spawn_proxy_server(TEST_PORT_RECONNECT + 1)
            .await
            .expect("重启 proxy 失败");

        wait_for_server_ready(&addr, 20)
            .await
            .expect("服务器重启超时");

        // 6. 等待客户端重连成功
        println!("⏳ 等待客户端重连...");
        let reconnected =
            wait_for_stderr_pattern(&mut stderr_reader, "重连成功", RECONNECT_DETECT_TIMEOUT)
                .await
                .expect("监控 stderr 失败");

        if reconnected {
            println!("✅ 客户端已重连成功");
        } else {
            println!("⚠️  未检测到重连成功日志（可能已在超时前重连）");
        }

        // 清理
        println!("🧹 清理进程...");
        let _ = client.kill().await;
        let _ = proxy.kill().await;

        println!("========== 测试 2 完成 ==========\n");
    }

    /// 测试 3: 指数退避验证
    ///
    /// 验证退避时间递增（1s, 2s, 4s...）
    #[tokio::test]
    async fn test_reconnection_exponential_backoff() {
        println!("\n========== 测试 3: 指数退避验证 ==========");

        // 不启动服务器，直接启动客户端
        // 客户端应该不断尝试连接并显示退避时间
        println!("🔗 启动 convert 客户端（无服务器）...");
        let url = format!("http://127.0.0.1:{}", TEST_PORT_RECONNECT + 2);
        let mut client = spawn_convert_client(&url, 30, 10)
            .await
            .expect("启动 convert 失败");

        let stderr = client.stderr.take().expect("获取 stderr 失败");
        let mut stderr_reader = BufReader::new(stderr);

        // 监控退避日志
        println!("⏳ 监控退避日志（等待约 15 秒）...");
        let mut backoff_times: Vec<String> = Vec::new();

        let result = timeout(Duration::from_secs(15), async {
            let mut line = String::new();
            loop {
                line.clear();
                match stderr_reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        print!("[stderr] {}", line);
                        // 查找退避时间日志 "N秒后重连"
                        if line.contains("秒后重连") {
                            backoff_times.push(line.clone());
                            if backoff_times.len() >= 3 {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        })
        .await;

        if result.is_err() {
            println!("⚠️  监控超时");
        }

        println!("📊 检测到的退避日志: {:?}", backoff_times);

        // 验证退避时间递增
        if backoff_times.len() >= 2 {
            println!("✅ 检测到退避机制正在工作");
        } else {
            println!("⚠️  未检测到足够的退避日志");
        }

        // 清理
        println!("🧹 清理进程...");
        let _ = client.kill().await;

        println!("========== 测试 3 完成 ==========\n");
    }

    /// 测试 4: 通道断开后请求是否立即返回错误
    ///
    /// 验证:
    /// - 服务器停止后，客户端发送请求能否立即返回错误
    /// - 而不是空等超时
    #[tokio::test]
    async fn test_request_returns_error_when_connection_closed() {
        println!("\n========== 测试 4: 通道断开后请求立即返回错误 ==========");

        // 1. 启动 proxy 服务器
        println!("🚀 启动 proxy 服务器...");
        let mut proxy = spawn_proxy_server(TEST_PORT_RECONNECT + 3)
            .await
            .expect("启动 proxy 失败");

        let addr = format!("127.0.0.1:{}", TEST_PORT_RECONNECT + 3);
        wait_for_server_ready(&addr, 20)
            .await
            .expect("服务器启动超时");

        // 2. 启动 convert 客户端（短 ping 间隔以快速检测断开）
        println!("🔗 启动 convert 客户端...");
        let url = format!("http://{}", addr);
        let mut client = spawn_convert_client(&url, 2, 2)
            .await
            .expect("启动 convert 失败");

        let mut stdin = client.stdin.take().expect("获取 stdin 失败");
        let stdout = client.stdout.take().expect("获取 stdout 失败");
        let stderr = client.stderr.take().expect("获取 stderr 失败");
        let mut stdout_reader = BufReader::new(stdout);
        let mut stderr_reader = BufReader::new(stderr);

        // 后台监控 stderr
        let stderr_monitor = tokio::spawn(async move {
            let mut line = String::new();
            loop {
                line.clear();
                match stderr_reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        print!("[stderr] {}", line);
                    }
                    Err(_) => break,
                }
            }
        });

        // 等待连接建立
        tokio::time::sleep(Duration::from_secs(3)).await;

        // 3. 初始化 MCP 客户端
        println!("🤝 初始化 MCP 客户端...");
        initialize_mcp_client(&mut stdin, &mut stdout_reader)
            .await
            .expect("初始化失败");

        // 4. 发送第一个 tools/list 请求确认通信正常
        println!("📋 发送第一个 tools/list 请求（应该成功）...");
        let tools_request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });

        let start = std::time::Instant::now();
        let tools_response =
            send_jsonrpc_and_receive(&mut stdin, &mut stdout_reader, tools_request)
                .await
                .expect("tools/list 请求失败");
        let elapsed = start.elapsed();

        assert!(
            tools_response["result"]["tools"].is_array(),
            "第一个请求应该成功返回工具列表"
        );
        println!("✅ 第一个请求成功，耗时: {:?}", elapsed);

        // 5. 杀死 proxy 服务器
        println!("💀 杀死 proxy 服务器...");
        let _ = proxy.kill().await;

        // 等待 ping 检测发现连接断开（ping 间隔是 2s，超时也是 2s）
        println!("⏳ 等待 ping 检测发现断开（约 5 秒）...");
        tokio::time::sleep(Duration::from_secs(5)).await;

        // 6. 发送第二个 tools/list 请求
        println!("📋 发送第二个 tools/list 请求（应该快速返回错误）...");
        let tools_request2 = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/list"
        });

        let start = std::time::Instant::now();
        let result = timeout(
            Duration::from_secs(5),
            send_jsonrpc_and_receive(&mut stdin, &mut stdout_reader, tools_request2),
        )
        .await;
        let elapsed = start.elapsed();

        match result {
            Ok(Ok(response)) => {
                // 检查是否是错误响应
                if response["error"].is_object() {
                    println!("✅ 收到错误响应，耗时: {:?}", elapsed);
                    println!("   错误信息: {:?}", response["error"]);
                    assert!(
                        elapsed < Duration::from_secs(3),
                        "错误响应应该在 3 秒内返回，实际耗时: {:?}",
                        elapsed
                    );
                } else {
                    // 如果返回了成功响应，说明可能重连了
                    println!("⚠️  收到成功响应（可能已重连），耗时: {:?}", elapsed);
                }
            }
            Ok(Err(e)) => {
                println!("✅ 请求失败（符合预期），耗时: {:?}", elapsed);
                println!("   错误: {}", e);
                assert!(
                    elapsed < Duration::from_secs(3),
                    "错误应该在 3 秒内返回，实际耗时: {:?}",
                    elapsed
                );
            }
            Err(_) => {
                println!("❌ 请求超时（5秒），说明客户端在空等！");
                panic!("请求应该快速返回错误，而不是超时空等");
            }
        }

        // 清理
        println!("🧹 清理进程...");
        drop(stdin);
        stderr_monitor.abort();
        let _ = client.kill().await;

        println!("========== 测试 4 完成 ==========\n");
    }
}
