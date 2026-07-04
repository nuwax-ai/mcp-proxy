// MCP客户端示例，用于与script_runner交互
// 使用方法：node mcp_client.js

const { spawn } = require('node:child_process');
const readline = require('node:readline');

// 启动script_runner进程
const scriptRunner = spawn('script_runner', ['--verbose']);

// 创建读取标准输入的接口
const rl = readline.createInterface({
  input: process.stdin,
  output: process.stdout
});

// 处理script_runner的输出
scriptRunner.stdout.on('data', (data) => {
  console.log(`接收到MCP服务器响应: ${data}`);
  try {
    const response = JSON.parse(data);
    console.log('解析后的响应:', JSON.stringify(response, null, 2));
  } catch (e) {
    console.log('无法解析为JSON，显示原始输出');
  }
});

// 处理script_runner的错误
scriptRunner.stderr.on('data', (data) => {
  console.error(`MCP服务器错误: ${data}`);
});

// 处理script_runner进程结束
scriptRunner.on('close', (code) => {
  console.log(`MCP服务器进程退出，退出码: ${code}`);
  rl.close();
});

// 初始化MCP连接
const initializeRequest = {
  jsonrpc: '2.0',
  id: 1,
  method: 'initialize',
  params: {
    client_info: {
      name: 'mcp_client_example'
    }
  }
};

// 发送初始化请求
console.log('发送初始化请求...');
scriptRunner.stdin.write(JSON.stringify(initializeRequest) + '\n');

// 等待初始化响应后，发送工具列表请求
setTimeout(() => {
  const listToolsRequest = {
    jsonrpc: '2.0',
    id: 2,
    method: 'listTools'
  };
  console.log('发送工具列表请求...');
  scriptRunner.stdin.write(JSON.stringify(listToolsRequest) + '\n');
}, 1000);

// 等待工具列表响应后，执行JavaScript代码
setTimeout(() => {
  const callToolRequest = {
    jsonrpc: '2.0',
    id: 3,
    method: 'callTool',
    params: {
      name: 'run_javascript',
      arguments: {
        code: 'console.log("Hello from JavaScript!"); return { message: "Hello, World!", value: 42 };'
      }
    }
  };
  console.log('发送JavaScript代码执行请求...');
  scriptRunner.stdin.write(JSON.stringify(callToolRequest) + '\n');
}, 2000);

// 等待JavaScript执行响应后，执行TypeScript代码
setTimeout(() => {
  const callToolRequest = {
    jsonrpc: '2.0',
    id: 4,
    method: 'callTool',
    params: {
      name: 'run_typescript',
      arguments: {
        code: 'const greeting: string = "Hello from TypeScript!"; console.log(greeting); return { message: greeting, timestamp: new Date().toISOString() };'
      }
    }
  };
  console.log('发送TypeScript代码执行请求...');
  scriptRunner.stdin.write(JSON.stringify(callToolRequest) + '\n');
}, 3000);

// 等待TypeScript执行响应后，执行Python代码
setTimeout(() => {
  const callToolRequest = {
    jsonrpc: '2.0',
    id: 5,
    method: 'callTool',
    params: {
      name: 'run_python',
      arguments: {
        code: 'import math\nprint("Hello from Python!")\nresult = math.sqrt(16)\nprint(f"Square root of 16 is {result}")\n{"message": "Hello from Python", "result": result}'
      }
    }
  };
  console.log('发送Python代码执行请求...');
  scriptRunner.stdin.write(JSON.stringify(callToolRequest) + '\n');
}, 4000);

// 处理用户输入，允许用户发送自定义请求
rl.on('line', (input) => {
  if (input.toLowerCase() === 'exit') {
    console.log('退出程序...');
    scriptRunner.kill();
    rl.close();
    return;
  }

  try {
    // 尝试解析用户输入为JSON
    const request = JSON.parse(input);
    console.log('发送自定义请求:', JSON.stringify(request, null, 2));
    scriptRunner.stdin.write(JSON.stringify(request) + '\n');
  } catch (e) {
    console.error('输入不是有效的JSON:', e.message);
  }
});

// 显示帮助信息
console.log('\n=== MCP客户端示例 ===');
console.log('已自动发送初始化、工具列表和代码执行请求');
console.log('您可以输入自定义JSON请求，或输入"exit"退出程序');
console.log('示例请求:');
console.log(JSON.stringify({
  jsonrpc: '2.0',
  id: 100,
  method: 'callTool',
  params: {
    name: 'run_javascript',
    arguments: {
      code: 'return "Hello, " + new Date().toISOString();'
    }
  }
}, null, 2));
console.log('====================\n'); 