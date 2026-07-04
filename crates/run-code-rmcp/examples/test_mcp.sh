#!/bin/bash

# 测试script_runner的简单bash脚本

# 初始化请求
echo "发送初始化请求..."
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"client_info":{"name":"test_client"}}}' | script_runner

# 等待一下
sleep 1

# 列出工具请求
echo -e "\n发送列出工具请求..."
echo '{"jsonrpc":"2.0","id":2,"method":"listTools"}' | script_runner

# 等待一下
sleep 1

# 执行JavaScript代码
echo -e "\n发送JavaScript代码执行请求..."
echo '{"jsonrpc":"2.0","id":3,"method":"callTool","params":{"name":"run_javascript","arguments":{"code":"console.log(\"Hello from JavaScript!\"); return { message: \"Hello, World!\", value: 42 };"}}}' | script_runner 